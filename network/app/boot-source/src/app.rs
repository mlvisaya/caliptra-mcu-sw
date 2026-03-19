// Licensed under the Apache-2.0 license

//! Boot source provider application.
//!
//! Implements the boot source protocol over the network mailbox, handling
//! requests from the MCU ROM/Runtime. Uses lwIP for DHCP configuration
//! and TFTP image downloads.

use boot_source_protocol::messages::*;
use core::cell::{Cell, UnsafeCell};
use network_hil::ethernet::Ethernet;
use network_hil::network_mbox::{
    NetworkMailbox, NetworkMailboxClient, NetworkMboxError, NetworkMboxTargetStatus, Result,
};
use network_hil::timers::Timers;

use crate::handler;
use crate::network;
use crate::toc::Toc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    /// Waiting for InitiateBootRequest.
    Idle,
    /// DHCP and TOC fetch completed. Ready to serve image requests.
    Ready,
    /// Currently streaming image chunks for an active TFTP download.
    Streaming,
}

pub struct BootSourceApp<'a, M: NetworkMailbox<'a>> {
    mbox: &'a M,
    state: Cell<AppState>,
    toc: UnsafeCell<Toc>,
    boot_flags: Cell<BootFlags>,
    /// Active TFTP transfer state: firmware ID being downloaded.
    active_firmware_id: Cell<u8>,
    /// Sequence number for the current image transfer.
    sequence_number: Cell<u16>,
    /// Byte offset into the current image transfer.
    transfer_offset: Cell<u32>,
    /// Maximum payload bytes per chunk (based on mailbox SRAM size).
    max_chunk_size: usize,
    dhcp_result: UnsafeCell<Option<network::DhcpResult>>,
}

impl<'a, M: NetworkMailbox<'a>> BootSourceApp<'a, M> {
    pub fn new(mbox: &'a M, max_chunk_size: usize) -> Self {
        Self {
            mbox,
            state: Cell::new(AppState::Idle),
            toc: UnsafeCell::new(Toc::new()),
            boot_flags: Cell::new(BootFlags(0)),
            active_firmware_id: Cell::new(0),
            sequence_number: Cell::new(0),
            transfer_offset: Cell::new(0),
            max_chunk_size,
            dhcp_result: UnsafeCell::new(None),
        }
    }

    /// Must be called once before [`run_loop`](Self::run_loop).
    pub fn init(
        &'a self,
        eth: &'static mut dyn Ethernet,
        timer: &'static dyn Timers,
    ) -> core::result::Result<(), network::NetworkError> {
        network::init_lwip(eth, timer)?;
        self.mbox.set_client(self);
        self.mbox.enable();
        Ok(())
    }

    /// Main polling loop.
    pub fn run_loop(&self, max_polls: u32) -> Result<()> {
        let mut seen_ready = false;
        for _ in 0..max_polls {
            self.mbox.poll();
            network::poll();

            let state = self.state.get();
            if state == AppState::Ready {
                seen_ready = true;
            }
            if seen_ready && state == AppState::Idle {
                return Ok(());
            }
        }
        Err(NetworkMboxError::Timeout)
    }

    /// # Safety
    /// Safe in bare-metal single-threaded context where the mailbox
    /// driver serializes callbacks.
    #[allow(clippy::mut_from_ref)]
    fn toc_mut(&self) -> &mut Toc {
        unsafe { &mut *self.toc.get() }
    }

    fn toc_ref(&self) -> &Toc {
        unsafe { &*self.toc.get() }
    }

    fn dispatch(&self, data: &[u8]) -> Result<()> {
        let msg_type_byte = match peek_message_type(data) {
            Some(b) => b,
            None => {
                return Err(NetworkMboxError::InvalidArgument);
            }
        };

        let msg_type = match MessageType::from_u8(msg_type_byte) {
            Some(mt) => mt,
            None => {
                return Err(NetworkMboxError::InvalidArgument);
            }
        };

        match (self.state.get(), msg_type) {
            (AppState::Idle, MessageType::InitiateBootRequest) => {
                let dhcp = unsafe { &mut *self.dhcp_result.get() };
                handler::handle_initiate_boot(
                    self.mbox,
                    data,
                    &self.boot_flags,
                    self.toc_mut(),
                    dhcp,
                )?;
                self.state.set(AppState::Ready);
                Ok(())
            }
            (AppState::Ready, MessageType::ImageMetadataRequest) => {
                handler::handle_image_metadata(self.mbox, data, self.toc_ref())
            }
            (AppState::Ready, MessageType::ImageDownloadRequest) => {
                let dhcp = unsafe { &*self.dhcp_result.get() };
                let dhcp = dhcp.as_ref().ok_or(NetworkMboxError::Failed)?;
                handler::handle_image_download(
                    self.mbox,
                    data,
                    self.toc_ref(),
                    dhcp,
                    &self.sequence_number,
                    &self.transfer_offset,
                )?;
                self.state.set(AppState::Streaming);
                Ok(())
            }
            (AppState::Streaming, MessageType::ChunkAck) => {
                let is_last = handler::handle_chunk_ack(
                    self.mbox,
                    data,
                    self.max_chunk_size,
                    &self.sequence_number,
                    &self.transfer_offset,
                )?;
                if is_last {
                    self.state.set(AppState::Ready);
                }
                Ok(())
            }
            (_, MessageType::Finalize) => {
                handler::handle_finalize(self.mbox, data, self.toc_mut())?;
                self.state.set(AppState::Idle);
                self.active_firmware_id.set(0);
                self.sequence_number.set(0);
                self.transfer_offset.set(0);
                Ok(())
            }
            _ => Err(NetworkMboxError::InvalidArgument),
        }
    }
}

impl<'a, M: NetworkMailbox<'a>> NetworkMailboxClient for BootSourceApp<'a, M> {
    fn request_received(
        &self,
        _command: u32,
        _user: u32,
        rx_buf: &'static mut [u32],
        dlen: usize,
    ) -> Result<()> {
        // Copy SRAM dwords to a local byte buffer. The mailbox SRAM
        // bus may only support word-aligned 32-bit reads, so we must
        // not reinterpret the SRAM pointer as a byte slice directly.
        const MAX_REQ_BYTES: usize = 256;
        let copy_len = dlen.min(MAX_REQ_BYTES);
        let mut local_buf = [0u8; MAX_REQ_BYTES];
        let dw_len = copy_len.div_ceil(4);
        for (i, &dw) in rx_buf[..dw_len].iter().enumerate() {
            let base = i * 4;
            let bytes = dw.to_le_bytes();
            for j in 0..4 {
                if base + j < copy_len {
                    local_buf[base + j] = bytes[j];
                }
            }
        }

        let result = self.dispatch(&local_buf[..copy_len]);

        self.mbox.restore_rx_buffer(rx_buf);
        result
    }

    fn response_received(
        &self,
        _status: NetworkMboxTargetStatus,
        _rx_buf: &'static mut [u32],
        _dlen: usize,
    ) {
        // Not used — this app operates in receiver mode only.
    }

    fn send_done(&self, _result: Result<()>) {
        // Not used — responses are sent synchronously.
    }
}
