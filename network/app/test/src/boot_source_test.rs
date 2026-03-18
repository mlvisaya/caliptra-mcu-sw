/*++

Licensed under the Apache-2.0 license.

File Name:

    boot_source_test.rs

Abstract:

    Network Coprocessor integration test for the boot-source protocol.

    This test app creates a BootSourceApp, pre-populates the TOC with
    a test entry, sets the state to Ready, and then polls for mailbox
    requests. The MCU side sends ImageMetadataRequest and Finalize
    messages to exercise the protocol handlers.

--*/

use network_app_boot_source::app::{AppState, BootSourceApp};
use network_drivers::network_mbox::NetworkMboxDriver;
use network_drivers::{exit_emulator, println};
use network_hil::network_mbox::NetworkMailbox;

/// Test firmware ID matches the MCU side.
const TEST_FIRMWARE_ID: u8 = 0x02; // MCU_RT
const TEST_IMAGE_SIZE: u32 = 4096;
const TEST_IMAGE_CHECKSUM: u32 = 0xAABBCCDD;

pub fn run() {
    println!();
    println!("=====================================");
    println!("  Boot Source Protocol Test Started! ");
    println!("=====================================");
    println!();

    let driver = NetworkMboxDriver::new();
    let app = BootSourceApp::new(&driver, 1024);

    // Initialize without lwIP — we only test protocol message handling.
    app.init_for_test();

    // Pre-populate a TOC entry so ImageMetadataRequest can find it.
    let added = app.add_toc_entry(
        TEST_FIRMWARE_ID,
        b"mcu_rt.bin\0",
        TEST_IMAGE_SIZE,
        TEST_IMAGE_CHECKSUM,
    );
    if !added {
        println!("[boot-src] ERROR: Failed to add TOC entry");
        exit_emulator(0x01);
    }

    // Set state to Ready so that ImageMetadataRequest is accepted.
    app.set_state_for_test(AppState::Ready);

    println!("[boot-src] TOC populated, state=Ready, waiting for requests...");

    // Poll until the app returns to Idle (after Finalize).
    let max_polls: u32 = 50_000_000;
    for _ in 0..max_polls {
        driver.poll();
        if app.state() == AppState::Idle {
            println!("[boot-src] State returned to Idle — test PASSED!");
            exit_emulator(0x00);
        }
    }

    println!("[boot-src] ERROR: Timed out waiting for Finalize");
    exit_emulator(0x01);
}
