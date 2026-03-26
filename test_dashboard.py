#!/usr/bin/env python3
"""
Test Dashboard – four-pane terminal viewer for the network boot integration test.

Layout:
  ┌──────────────────────┬──────────────────────┐
  │ Caliptra MCU terminal│ Caliptra Core terminal│
  ├──────────────────────┼──────────────────────┤
  │ Network Coprocessor  │ Packet capture on Tap0│
  │       terminal       │                       │
  └──────────────────────┴──────────────────────┘

* Caliptra Core terminal  – stdout/stderr of the test process
* Caliptra MCU terminal   – tails /tmp/mcu_log.txt
* Network Coprocessor terminal – tails /tmp/network_log.txt
* Packet capture on Tap0  – live tshark output on tap0

Usage:
  python3 -m venv .venv && source .venv/bin/activate
  pip install -r requirements.txt
  python3 test_dashboard.py
"""

import os
import subprocess
import time

from textual.app import App, ComposeResult
from textual.containers import Center, Horizontal, Middle, Vertical
from textual.screen import ModalScreen
from textual.widgets import RichLog, Static

MCU_LOG = "/tmp/mcu_log.txt"
NETWORK_LOG = "/tmp/network_log.txt"

# Truncate the log files so we only see output from this run.
for path in (MCU_LOG, NETWORK_LOG):
    open(path, "w").close()

import glob

def _find_test_cmd():
    """Use the pre-built test binary directly if it exists, otherwise fall back to cargo test."""
    pattern = os.path.join(os.getcwd(), "target", "debug", "deps", "tests_integration-*")
    candidates = [p for p in glob.glob(pattern) if os.access(p, os.X_OK) and not p.endswith(".d")]
    if candidates:
        # Pick the most recently modified binary.
        binary = max(candidates, key=os.path.getmtime)
        return [binary, "test_network_boot", "--nocapture"]
    return ["cargo", "test", "-p", "tests-integration", "test_network_boot", "--", "--nocapture"]

TEST_CMD = _find_test_cmd()
TSHARK_CMD = ["sudo", "tshark", "-i", "tap0", "-l"]


class StartScreen(ModalScreen):
    CSS = """
    StartScreen {
        align: center middle;
        background: rgba(0, 0, 0, 0.7);
    }
    #banner {
        width: auto;
        height: auto;
        padding: 2 4;
        border: double green;
        text-align: center;
        color: #00ff00;
        text-style: bold;
        background: #1e1e1e;
    }
    """

    def compose(self) -> ComposeResult:
        with Center():
            with Middle():
                yield Static(
                    "Caliptra Network Boot Test Dashboard\n\n"
                    "Press any key to start test...",
                    id="banner",
                )

    def on_key(self) -> None:
        self.app.pop_screen()
        self.app.begin_test()


class TestDashboard(App):
    CSS = """
    Screen {
        layout: vertical;
    }
    Horizontal {
        height: 1fr;
    }
    .pane {
        width: 1fr;
        height: 1fr;
        border: solid green;
    }
    .pane-label {
        dock: top;
        width: 100%;
        text-align: center;
        color: #00ff00;
        text-style: bold;
        background: #1e1e1e;
    }
    RichLog {
        background: #0c0c0c;
        color: #d4d4d4;
    }
    """

    def __init__(self):
        super().__init__()
        self._proc = None
        self._tshark_proc = None

    def compose(self) -> ComposeResult:
        with Horizontal():
            with Vertical(classes="pane"):
                yield Static("Caliptra MCU terminal", classes="pane-label")
                yield RichLog(id="mcu", wrap=True, markup=False)
            with Vertical(classes="pane"):
                yield Static("Caliptra Core terminal", classes="pane-label")
                yield RichLog(id="core", wrap=True, markup=False)
        with Horizontal():
            with Vertical(classes="pane"):
                yield Static("Network Coprocessor terminal", classes="pane-label")
                yield RichLog(id="net", wrap=True, markup=False)
            with Vertical(classes="pane"):
                yield Static("Packet capture on Tap0", classes="pane-label")
                yield RichLog(id="tshark", wrap=True, markup=False)

    def on_mount(self) -> None:
        self.push_screen(StartScreen())

    def begin_test(self) -> None:
        self._start_test_process()
        self._start_tshark()
        self._start_file_tailer(MCU_LOG, "mcu")
        self._start_file_tailer(NETWORK_LOG, "net")

    def _start_test_process(self) -> None:
        env = os.environ.copy()
        env["CPTRA_FIRMWARE_BUNDLE"] = os.path.join(os.getcwd(), "target", "all-fw.zip")

        self._proc = subprocess.Popen(
            TEST_CMD,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            env=env,
        )
        self.run_worker(self._read_process, thread=True)

    def _read_process(self) -> None:
        log = self.query_one("#core", RichLog)
        while True:
            line = self._proc.stdout.readline()
            if not line:
                break
            self.call_from_thread(log.write, line.decode("utf-8", errors="replace").rstrip("\n"))
        self._proc.wait()
        self.call_from_thread(
            log.write, f"\n--- process exited with code {self._proc.returncode} ---"
        )

    def _start_tshark(self) -> None:
        self._tshark_proc = subprocess.Popen(
            TSHARK_CMD,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        self.run_worker(self._read_tshark, thread=True)

    def _read_tshark(self) -> None:
        log = self.query_one("#tshark", RichLog)
        while True:
            line = self._tshark_proc.stdout.readline()
            if not line:
                break
            self.call_from_thread(log.write, line.decode("utf-8", errors="replace").rstrip("\n"))
        self._tshark_proc.wait()
        self.call_from_thread(
            log.write, f"\n--- tshark exited with code {self._tshark_proc.returncode} ---"
        )

    def _start_file_tailer(self, path: str, widget_id: str) -> None:
        self.run_worker(lambda: self._tail_file(path, widget_id), thread=True)

    def _tail_file(self, path: str, widget_id: str) -> None:
        log = self.query_one(f"#{widget_id}", RichLog)
        while not self._exit:
            try:
                with open(path, "r") as f:
                    f.seek(0, 2)
                    while not self._exit:
                        data = f.read(4096)
                        if data:
                            for line in data.splitlines():
                                self.call_from_thread(log.write, line)
                        else:
                            time.sleep(0.1)
            except FileNotFoundError:
                time.sleep(0.5)

    def on_unmount(self) -> None:
        if self._proc and self._proc.poll() is None:
            self._proc.terminate()
        if self._tshark_proc and self._tshark_proc.poll() is None:
            self._tshark_proc.terminate()


if __name__ == "__main__":
    TestDashboard().run()
