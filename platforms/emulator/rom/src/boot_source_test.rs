/*++

Licensed under the Apache-2.0 license.

File Name:

    boot_source_test.rs

Abstract:

    MCU ROM test for the boot-source protocol via the network mailbox.

    Sends an ImageMetadataRequest for firmware ID 0x02 (MCU_RT) and
    verifies that the Network CoP responds with the expected image size.
    Then sends a Finalize to clean up and verifies the FinalizeAck.

--*/

use core::cell::Cell;
use network_drivers::network_mbox::NetworkMboxDriver;
use network_hil::network_mbox::{
    NetworkMailbox, NetworkMailboxClient, NetworkMboxError, NetworkMboxStatus,
    NetworkMboxTargetStatus, Result,
};

/// Firmware ID matching the Network CoP test TOC entry.
const TEST_FIRMWARE_ID: u8 = 0x02;
/// Expected image size matching the Network CoP test TOC entry.
const EXPECTED_IMAGE_SIZE: u32 = 4096;

// Message type bytes from the boot-source protocol.
const MSG_TYPE_IMAGE_METADATA_REQUEST: u8 = 0x02;
const MSG_TYPE_IMAGE_METADATA_RESPONSE: u8 = 0x82;
const MSG_TYPE_FINALIZE: u8 = 0x05;
const MSG_TYPE_FINALIZE_ACK: u8 = 0x85;

/// Status codes.
const STATUS_SUCCESS: u8 = 0x00;

// -----------------------------------------------------------------------
// Test phases
// -----------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    SendMetadataRequest,
    WaitMetadataResponse,
    SendFinalize,
    WaitFinalizeAck,
    Done,
}

/// Client that drives the two-phase test.
struct BootSourceTestClient<'a> {
    driver: &'a NetworkMboxDriver<'a>,
    phase: Cell<Phase>,
    passed: Cell<bool>,
}

impl<'a> BootSourceTestClient<'a> {
    fn new(driver: &'a NetworkMboxDriver<'a>) -> Self {
        Self {
            driver,
            phase: Cell::new(Phase::SendMetadataRequest),
            passed: Cell::new(true),
        }
    }
}

impl<'a> NetworkMailboxClient for BootSourceTestClient<'a> {
    fn request_received(
        &self,
        _command: u32,
        _user: u32,
        rx_buf: &'static mut [u32],
        _dlen: usize,
    ) -> Result<()> {
        // MCU is acting as sender — should not receive requests.
        self.driver.restore_rx_buffer(rx_buf);
        Ok(())
    }

    fn response_received(
        &self,
        status: NetworkMboxTargetStatus,
        rx_buf: &'static mut [u32],
        dlen: usize,
    ) {
        // Copy SRAM dwords to a local byte buffer. The mailbox SRAM
        // bus only supports word-aligned 32-bit reads.
        const MAX_RESP: usize = 256;
        let copy_len = dlen.min(MAX_RESP);
        let mut local_buf = [0u8; MAX_RESP];
        let dw_len = copy_len.div_ceil(4);
        for i in 0..dw_len {
            let dw = rx_buf[i];
            let base = i * 4;
            let bytes = dw.to_le_bytes();
            for j in 0..4 {
                if base + j < copy_len {
                    local_buf[base + j] = bytes[j];
                }
            }
        }
        let bytes = &local_buf[..copy_len];

        match self.phase.get() {
            Phase::WaitMetadataResponse => {
                self.verify_metadata_response(status, bytes);
                self.phase.set(Phase::SendFinalize);
            }
            Phase::WaitFinalizeAck => {
                self.verify_finalize_ack(status, bytes);
                self.phase.set(Phase::Done);
            }
            _ => {
                romtime::println!("[mcu-boot] Unexpected response in phase {:?}", self.phase.get() as u8);
                self.passed.set(false);
            }
        }
        self.driver.restore_rx_buffer(rx_buf);
    }

    fn send_done(&self, _result: Result<()>) {}
}

impl<'a> BootSourceTestClient<'a> {
    fn verify_metadata_response(
        &self,
        status: NetworkMboxTargetStatus,
        bytes: &[u8],
    ) {
        if status.status != NetworkMboxStatus::CmdComplete {
            romtime::println!(
                "[mcu-boot] FAILED: ImageMetadataResponse status {:?}",
                status.status
            );
            self.passed.set(false);
            return;
        }

        if bytes.len() < 2 {
            romtime::println!("[mcu-boot] FAILED: response too short ({})", bytes.len());
            self.passed.set(false);
            return;
        }

        if bytes[0] != MSG_TYPE_IMAGE_METADATA_RESPONSE {
            romtime::println!("[mcu-boot] FAILED: unexpected msg_type {:#x}", bytes[0]);
            self.passed.set(false);
            return;
        }
        if bytes[1] != STATUS_SUCCESS {
            romtime::println!("[mcu-boot] FAILED: response status {:#x}", bytes[1]);
            self.passed.set(false);
            return;
        }

        // image_size is at offset 4 (after msg_type:1, status:1, reserved:2).
        if bytes.len() < 8 {
            romtime::println!("[mcu-boot] FAILED: response too short for image_size");
            self.passed.set(false);
            return;
        }
        let image_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if image_size != EXPECTED_IMAGE_SIZE {
            romtime::println!(
                "[mcu-boot] FAILED: image_size={}, expected={}",
                image_size,
                EXPECTED_IMAGE_SIZE
            );
            self.passed.set(false);
            return;
        }

        romtime::println!("[mcu-boot] ImageMetadataResponse verified OK");
    }

    fn verify_finalize_ack(
        &self,
        status: NetworkMboxTargetStatus,
        bytes: &[u8],
    ) {
        if status.status != NetworkMboxStatus::CmdComplete {
            romtime::println!(
                "[mcu-boot] FAILED: FinalizeAck status {:?}",
                status.status
            );
            self.passed.set(false);
            return;
        }

        if bytes.len() < 2 {
            romtime::println!("[mcu-boot] FAILED: FinalizeAck too short");
            self.passed.set(false);
            return;
        }
        if bytes[0] != MSG_TYPE_FINALIZE_ACK {
            romtime::println!("[mcu-boot] FAILED: unexpected msg_type {:#x}", bytes[0]);
            self.passed.set(false);
            return;
        }
        if bytes[1] != STATUS_SUCCESS {
            romtime::println!("[mcu-boot] FAILED: FinalizeAck status {:#x}", bytes[1]);
            self.passed.set(false);
            return;
        }

        romtime::println!("[mcu-boot] FinalizeAck verified OK");
    }
}

/// Build an ImageMetadataRequest as dwords.
fn build_metadata_request() -> ([u32; 1], usize) {
    // ImageMetadataRequest: msg_type(1) + firmware_id(1) + reserved(2) = 4 bytes = 1 dword.
    let bytes: [u8; 4] = [MSG_TYPE_IMAGE_METADATA_REQUEST, TEST_FIRMWARE_ID, 0, 0];
    ([u32::from_le_bytes(bytes)], 4)
}

/// Build a FinalizeRequest as dwords.
fn build_finalize_request() -> ([u32; 2], usize) {
    // FinalizeRequest: msg_type(1) + status(1) + error_code(2) + reserved(4) = 8 bytes.
    let bytes: [u8; 8] = [MSG_TYPE_FINALIZE, STATUS_SUCCESS, 0, 0, 0, 0, 0, 0];
    let dw0 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let dw1 = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    ([dw0, dw1], 8)
}

pub fn run() {
    romtime::println!();
    romtime::println!("=====================================");
    romtime::println!("  MCU Boot Source Test Started!      ");
    romtime::println!("=====================================");
    romtime::println!();

    let driver = NetworkMboxDriver::new();
    let client = BootSourceTestClient::new(&driver);
    driver.set_client(&client);

    let max_polls: u64 = 500_000_000;

    for _ in 0..max_polls {
        match client.phase.get() {
            Phase::SendMetadataRequest => {
                let (data, dlen) = build_metadata_request();
                match driver.send_request(0, 0, data.iter().copied(), dlen) {
                    Ok(()) => {
                        romtime::println!("[mcu-boot] ImageMetadataRequest sent");
                        client.phase.set(Phase::WaitMetadataResponse);
                    }
                    Err(NetworkMboxError::Locked) => continue,
                    Err(e) => {
                        romtime::println!("[mcu-boot] FAILED: send error {:?}", e);
                        return;
                    }
                }
            }
            Phase::SendFinalize => {
                let (data, dlen) = build_finalize_request();
                match driver.send_request(0, 0, data.iter().copied(), dlen) {
                    Ok(()) => {
                        romtime::println!("[mcu-boot] FinalizeRequest sent");
                        client.phase.set(Phase::WaitFinalizeAck);
                    }
                    Err(NetworkMboxError::Locked) => continue,
                    Err(e) => {
                        romtime::println!("[mcu-boot] FAILED: send error {:?}", e);
                        return;
                    }
                }
            }
            Phase::Done => {
                if client.passed.get() {
                    romtime::println!("[mcu-boot] Boot source protocol test PASSED!");
                } else {
                    romtime::println!("[mcu-boot] Boot source protocol test FAILED!");
                }
                return;
            }
            _ => {
                // WaitMetadataResponse or WaitFinalizeAck — just poll.
                driver.poll();
            }
        }
    }

    romtime::println!("[mcu-boot] FAILED: timed out");
}
