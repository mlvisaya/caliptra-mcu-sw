/*++

Licensed under the Apache-2.0 license.

File Name:

    boot_source_test.rs

Abstract:

    MCU ROM test for the boot-source protocol via the network mailbox.

--*/

use boot_source_protocol::messages::{
    BootFlags, ChunkAck, ChunkAckFlags, FinalizeAck, FinalizeRequest, ImageChunkHeader,
    ImageDownloadRequest, ImageMetadataRequest, ImageMetadataResponse, InitiateBootRequest,
    InitiateBootResponse, MessageType, Status, FIRMWARE_ID_MCU_RT,
};
use core::cell::Cell;
use network_drivers::network_mbox::NetworkMboxDriver;
use network_hil::network_mbox::{
    NetworkMailbox, NetworkMailboxClient, NetworkMboxError, NetworkMboxStatus,
    NetworkMboxTargetStatus, Result,
};
use zerocopy::{FromBytes, IntoBytes};

const TEST_FIRMWARE_ID: u8 = FIRMWARE_ID_MCU_RT;
const EXPECTED_IMAGE_SIZE: u32 = 4096;

// -----------------------------------------------------------------------
// Test phases
// -----------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    SendInitiateBoot,
    WaitInitiateBootResponse,
    SendMetadataRequest,
    WaitMetadataResponse,
    SendImageDownload,
    WaitImageChunk,
    SendChunkAck,
    SendFinalize,
    WaitFinalizeAck,
    Done,
}

struct BootSourceTestClient<'a> {
    driver: &'a NetworkMboxDriver<'a>,
    phase: Cell<Phase>,
    passed: Cell<bool>,
    last_seq: Cell<u16>,
    bytes_received: Cell<u32>,
}

impl<'a> BootSourceTestClient<'a> {
    fn new(driver: &'a NetworkMboxDriver<'a>) -> Self {
        Self {
            driver,
            phase: Cell::new(Phase::SendInitiateBoot),
            passed: Cell::new(true),
            last_seq: Cell::new(0),
            bytes_received: Cell::new(0),
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
        // Copy SRAM dwords to a local u32 buffer (guarantees alignment
        // for zerocopy ref_from_prefix on structs with u32 fields).
        // Must be large enough for ImageChunkHeader (12) + max payload (1024).
        const MAX_RESP_DW: usize = 276; // 276 * 4 = 1104 bytes
        let copy_len = dlen.min(MAX_RESP_DW * 4);
        let dw_len = copy_len.div_ceil(4);
        let mut local_dw = [0u32; MAX_RESP_DW];
        for i in 0..dw_len {
            local_dw[i] = rx_buf[i];
        }
        let bytes = &local_dw.as_bytes()[..copy_len];

        match self.phase.get() {
            Phase::WaitInitiateBootResponse => {
                self.verify_initiate_boot_response(status, bytes);
                self.phase.set(Phase::SendMetadataRequest);
            }
            Phase::WaitMetadataResponse => {
                self.verify_metadata_response(status, bytes);
                self.phase.set(Phase::SendImageDownload);
            }
            Phase::WaitImageChunk => {
                self.verify_image_chunk(status, bytes);
                if self.bytes_received.get() >= EXPECTED_IMAGE_SIZE {
                    romtime::println!(
                        "[mcu-boot] Image download complete: {} bytes",
                        self.bytes_received.get()
                    );
                    self.phase.set(Phase::SendFinalize);
                } else {
                    self.phase.set(Phase::SendChunkAck);
                }
            }
            Phase::WaitFinalizeAck => {
                self.verify_finalize_ack(status, bytes);
                self.phase.set(Phase::Done);
            }
            _ => {
                romtime::println!(
                    "[mcu-boot] Unexpected response in phase {:?}",
                    self.phase.get() as u8
                );
                self.passed.set(false);
            }
        }
        self.driver.restore_rx_buffer(rx_buf);
    }

    fn send_done(&self, _result: Result<()>) {}
}

impl<'a> BootSourceTestClient<'a> {
    fn verify_initiate_boot_response(&self, status: NetworkMboxTargetStatus, bytes: &[u8]) {
        if status.status != NetworkMboxStatus::CmdComplete {
            romtime::println!(
                "[mcu-boot] FAILED: InitiateBootResponse status {:?}",
                status.status
            );
            self.passed.set(false);
            return;
        }
        let resp = match InitiateBootResponse::ref_from_prefix(bytes) {
            Ok((r, _)) => r,
            Err(_) => {
                romtime::println!("[mcu-boot] FAILED: InitiateBootResponse too short");
                self.passed.set(false);
                return;
            }
        };
        if resp.message_type != MessageType::InitiateBootResponse.as_u8() {
            romtime::println!(
                "[mcu-boot] FAILED: unexpected msg_type {:#x}",
                resp.message_type
            );
            self.passed.set(false);
            return;
        }
        if resp.status != Status::Success.as_u8() {
            romtime::println!(
                "[mcu-boot] FAILED: InitiateBootResponse status {:#x}",
                resp.status
            );
            self.passed.set(false);
            return;
        }
        romtime::println!("[mcu-boot] InitiateBootResponse verified OK");
    }

    fn verify_metadata_response(&self, status: NetworkMboxTargetStatus, bytes: &[u8]) {
        if status.status != NetworkMboxStatus::CmdComplete {
            romtime::println!(
                "[mcu-boot] FAILED: ImageMetadataResponse status {:?}",
                status.status
            );
            self.passed.set(false);
            return;
        }
        let resp = match ImageMetadataResponse::ref_from_prefix(bytes) {
            Ok((r, _)) => r,
            Err(_) => {
                romtime::println!("[mcu-boot] FAILED: ImageMetadataResponse too short");
                self.passed.set(false);
                return;
            }
        };
        if resp.message_type != MessageType::ImageMetadataResponse.as_u8() {
            romtime::println!(
                "[mcu-boot] FAILED: unexpected msg_type {:#x}",
                resp.message_type
            );
            self.passed.set(false);
            return;
        }
        if resp.status != Status::Success.as_u8() {
            romtime::println!("[mcu-boot] FAILED: response status {:#x}", resp.status);
            self.passed.set(false);
            return;
        }
        if resp.image_size != EXPECTED_IMAGE_SIZE {
            romtime::println!(
                "[mcu-boot] FAILED: image_size={}, expected={}",
                resp.image_size,
                EXPECTED_IMAGE_SIZE
            );
            self.passed.set(false);
            return;
        }

        romtime::println!("[mcu-boot] ImageMetadataResponse verified OK");
    }

    fn verify_finalize_ack(&self, status: NetworkMboxTargetStatus, bytes: &[u8]) {
        if status.status != NetworkMboxStatus::CmdComplete {
            romtime::println!("[mcu-boot] FAILED: FinalizeAck status {:?}", status.status);
            self.passed.set(false);
            return;
        }
        let ack = match FinalizeAck::ref_from_prefix(bytes) {
            Ok((r, _)) => r,
            Err(_) => {
                romtime::println!("[mcu-boot] FAILED: FinalizeAck too short");
                self.passed.set(false);
                return;
            }
        };
        if ack.message_type != MessageType::FinalizeAck.as_u8() {
            romtime::println!(
                "[mcu-boot] FAILED: unexpected msg_type {:#x}",
                ack.message_type
            );
            self.passed.set(false);
            return;
        }
        if ack.status != Status::Success.as_u8() {
            romtime::println!("[mcu-boot] FAILED: FinalizeAck status {:#x}", ack.status);
            self.passed.set(false);
            return;
        }

        romtime::println!("[mcu-boot] FinalizeAck verified OK");
    }

    fn verify_image_chunk(&self, status: NetworkMboxTargetStatus, bytes: &[u8]) {
        if status.status != NetworkMboxStatus::CmdComplete {
            romtime::println!("[mcu-boot] FAILED: ImageChunk status {:?}", status.status);
            self.passed.set(false);
            return;
        }
        let hdr = match ImageChunkHeader::ref_from_prefix(bytes) {
            Ok((h, _)) => h,
            Err(_) => {
                romtime::println!("[mcu-boot] FAILED: ImageChunkHeader too short");
                self.passed.set(false);
                return;
            }
        };
        if hdr.message_type != MessageType::ImageChunk.as_u8() {
            romtime::println!(
                "[mcu-boot] FAILED: unexpected msg_type {:#x}",
                hdr.message_type
            );
            self.passed.set(false);
            return;
        }
        if hdr.status != Status::Success.as_u8() {
            romtime::println!("[mcu-boot] FAILED: ImageChunk status {:#x}", hdr.status);
            self.passed.set(false);
            return;
        }
        let chunk_size = hdr.chunk_size as usize;
        let expected_offset = self.bytes_received.get();
        if hdr.offset != expected_offset {
            romtime::println!(
                "[mcu-boot] FAILED: offset={}, expected={}",
                hdr.offset,
                expected_offset
            );
            self.passed.set(false);
            return;
        }
        // Verify payload data matches expected pattern: (byte_index & 0xFF).
        if chunk_size > 0 {
            let payload_start = ImageChunkHeader::SIZE;
            let payload_end = payload_start + chunk_size;
            if bytes.len() < payload_end {
                romtime::println!(
                    "[mcu-boot] FAILED: response too short for chunk payload ({} < {})",
                    bytes.len(),
                    payload_end
                );
                self.passed.set(false);
                return;
            }
            let payload = &bytes[payload_start..payload_end];
            for (j, &byte) in payload.iter().enumerate() {
                let expected = ((expected_offset as usize + j) & 0xFF) as u8;
                if byte != expected {
                    romtime::println!(
                        "[mcu-boot] FAILED: data[{}]={:#x}, expected={:#x}",
                        expected_offset as usize + j,
                        byte,
                        expected
                    );
                    self.passed.set(false);
                    return;
                }
            }
        }
        self.last_seq.set(hdr.sequence_number);
        self.bytes_received.set(expected_offset + chunk_size as u32);
        romtime::println!(
            "[mcu-boot] ImageChunk seq={} offset={} size={} OK",
            hdr.sequence_number,
            hdr.offset,
            chunk_size
        );
    }
}

fn pkt_to_dwords<T: IntoBytes + zerocopy::Immutable>(
    pkt: &T,
) -> (impl Iterator<Item = u32> + '_, usize) {
    let bytes = pkt.as_bytes();
    let dlen = bytes.len();
    let iter = bytes.chunks(4).map(|chunk| {
        let mut dw = [0u8; 4];
        dw[..chunk.len()].copy_from_slice(chunk);
        u32::from_le_bytes(dw)
    });
    (iter, dlen)
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
            Phase::SendInitiateBoot => {
                let pkt = InitiateBootRequest::new(1, BootFlags(0));
                let (data, dlen) = pkt_to_dwords(&pkt);
                match driver.send_request(0, 0, data, dlen) {
                    Ok(()) => {
                        romtime::println!("[mcu-boot] InitiateBootRequest sent");
                        client.phase.set(Phase::WaitInitiateBootResponse);
                    }
                    Err(NetworkMboxError::Locked) => continue,
                    Err(e) => {
                        romtime::println!("[mcu-boot] FAILED: send error {:?}", e);
                        return;
                    }
                }
            }
            Phase::SendMetadataRequest => {
                let pkt = ImageMetadataRequest::new(TEST_FIRMWARE_ID);
                let (data, dlen) = pkt_to_dwords(&pkt);
                match driver.send_request(0, 0, data, dlen) {
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
            Phase::SendImageDownload => {
                let pkt = ImageDownloadRequest::new(TEST_FIRMWARE_ID);
                let (data, dlen) = pkt_to_dwords(&pkt);
                match driver.send_request(0, 0, data, dlen) {
                    Ok(()) => {
                        romtime::println!("[mcu-boot] ImageDownloadRequest sent");
                        client.phase.set(Phase::WaitImageChunk);
                    }
                    Err(NetworkMboxError::Locked) => continue,
                    Err(e) => {
                        romtime::println!("[mcu-boot] FAILED: send error {:?}", e);
                        return;
                    }
                }
            }
            Phase::SendChunkAck => {
                let mut flags = ChunkAckFlags(0);
                flags.set_ready_for_next(true);
                let pkt = ChunkAck::new(TEST_FIRMWARE_ID, client.last_seq.get(), flags);
                let (data, dlen) = pkt_to_dwords(&pkt);
                match driver.send_request(0, 0, data, dlen) {
                    Ok(()) => {
                        client.phase.set(Phase::WaitImageChunk);
                    }
                    Err(NetworkMboxError::Locked) => continue,
                    Err(e) => {
                        romtime::println!("[mcu-boot] FAILED: send error {:?}", e);
                        return;
                    }
                }
            }
            Phase::SendFinalize => {
                let pkt = FinalizeRequest::success();
                let (data, dlen) = pkt_to_dwords(&pkt);
                match driver.send_request(0, 0, data, dlen) {
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
                // WaitInitiateBootResponse, WaitMetadataResponse, WaitImageChunk,
                // or WaitFinalizeAck — just poll.
                driver.poll();
            }
        }
    }

    romtime::println!("[mcu-boot] FAILED: timed out");
}
