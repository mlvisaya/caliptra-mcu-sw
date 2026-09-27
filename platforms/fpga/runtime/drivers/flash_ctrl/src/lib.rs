// Licensed under the Apache-2.0 license

//! HTG940_STANDALONE_FIRMWARE_BRAM
//! Writable 2 MiB AXI BRAM flash facade for the standalone HTG-940 build.

#![cfg_attr(target_arch = "riscv32", no_std)]

use caliptra_mcu_registers_generated::mci;
use caliptra_mcu_romtime::StaticRef;
use core::ops::{Index, IndexMut};
use kernel::deferred_call::{DeferredCall, DeferredCallClient};
use kernel::hil;
use kernel::utilities::cells::{OptionalCell, TakeCell};
use kernel::ErrorCode;

pub const PAGE_SIZE: usize = 256;
pub const ERASE_SECTOR_SIZE: usize = 4096;
pub const HTG940_FLASH_BASE: usize = 0xB020_0000;
pub const HTG940_FLASH_CAPACITY: usize = 2 * 1024 * 1024;
pub const FLASH_MAX_PAGES: usize = HTG940_FLASH_CAPACITY / PAGE_SIZE;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum FlashOperation {
    ReadPage = 1,
    WritePage = 2,
    ErasePage = 3,
}

pub struct EmulatedFlashPage(pub [u8; PAGE_SIZE]);

impl Default for EmulatedFlashPage {
    fn default() -> Self {
        Self([0; PAGE_SIZE])
    }
}

impl Index<usize> for EmulatedFlashPage {
    type Output = u8;
    fn index(&self, idx: usize) -> &u8 {
        &self.0[idx]
    }
}

impl IndexMut<usize> for EmulatedFlashPage {
    fn index_mut(&mut self, idx: usize) -> &mut u8 {
        &mut self.0[idx]
    }
}

impl AsMut<[u8]> for EmulatedFlashPage {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

pub struct EmulatedFlashCtrl<'a> {
    flash_client: OptionalCell<&'a dyn hil::flash::Client<EmulatedFlashCtrl<'a>>>,
    read_buf: TakeCell<'static, EmulatedFlashPage>,
    write_buf: TakeCell<'static, EmulatedFlashPage>,
    pending_op: OptionalCell<FlashOperation>,
    deferred_call: DeferredCall,
}

impl<'a> EmulatedFlashCtrl<'a> {
    pub fn new(_registers: StaticRef<mci::regs::Mci>) -> EmulatedFlashCtrl<'a> {
        EmulatedFlashCtrl {
            flash_client: OptionalCell::empty(),
            read_buf: TakeCell::empty(),
            write_buf: TakeCell::empty(),
            pending_op: OptionalCell::empty(),
            deferred_call: DeferredCall::new(),
        }
    }

    pub fn init(&self) {}

    fn page_address(page_number: usize) -> usize {
        HTG940_FLASH_BASE + page_number * PAGE_SIZE
    }

    fn submit_io(&self, op: FlashOperation, page_number: usize) -> Result<(), ErrorCode> {
        if page_number >= FLASH_MAX_PAGES || self.pending_op.is_some() {
            return Err(if page_number >= FLASH_MAX_PAGES {
                ErrorCode::INVAL
            } else {
                ErrorCode::BUSY
            });
        }

        let address = Self::page_address(page_number);
        match op {
            FlashOperation::ReadPage => {
                let buf = self.read_buf.take().ok_or(ErrorCode::INVAL)?;
                let src = address as *const u8;
                for index in 0..PAGE_SIZE {
                    buf.0[index] = unsafe { core::ptr::read_volatile(src.add(index)) };
                }
                self.read_buf.replace(buf);
            }
            FlashOperation::WritePage => {
                let buf = self.write_buf.take().ok_or(ErrorCode::INVAL)?;
                let dst = address as *mut u8;
                for index in 0..PAGE_SIZE {
                    unsafe { core::ptr::write_volatile(dst.add(index), buf.0[index]) };
                }
                self.write_buf.replace(buf);
            }
            FlashOperation::ErasePage => {
                let dst = address as *mut u8;
                for index in 0..PAGE_SIZE {
                    unsafe { core::ptr::write_volatile(dst.add(index), 0xFF) };
                }
            }
        }

        self.pending_op.set(op);
        self.deferred_call.set();
        Ok(())
    }

    fn handle_io_completion(&self) {
        let Some(op) = self.pending_op.take() else {
            return;
        };
        match op {
            FlashOperation::ReadPage => {
                let buf = self.read_buf.take().expect("missing flash read buffer");
                self.flash_client
                    .map(|client| client.read_complete(buf, Ok(())));
            }
            FlashOperation::WritePage => {
                let buf = self.write_buf.take().expect("missing flash write buffer");
                self.flash_client
                    .map(|client| client.write_complete(buf, Ok(())));
            }
            FlashOperation::ErasePage => {
                self.flash_client
                    .map(|client| client.erase_complete(Ok(())));
            }
        }
    }
}

impl DeferredCallClient for EmulatedFlashCtrl<'_> {
    fn register(&'static self) {
        self.deferred_call.register(self);
    }
    fn handle_deferred_call(&self) {
        self.handle_io_completion();
    }
}

impl<C: hil::flash::Client<Self>> hil::flash::HasClient<'static, C> for EmulatedFlashCtrl<'_> {
    fn set_client(&self, client: &'static C) {
        self.flash_client.set(client);
    }
}

impl hil::flash::Flash for EmulatedFlashCtrl<'_> {
    type Page = EmulatedFlashPage;

    fn read_page(
        &self,
        page_number: usize,
        buf: &'static mut Self::Page,
    ) -> Result<(), (ErrorCode, &'static mut Self::Page)> {
        if page_number >= FLASH_MAX_PAGES || self.pending_op.is_some() {
            return Err((
                if page_number >= FLASH_MAX_PAGES {
                    ErrorCode::INVAL
                } else {
                    ErrorCode::BUSY
                },
                buf,
            ));
        }
        self.read_buf.replace(buf);
        self.submit_io(FlashOperation::ReadPage, page_number)
            .map_err(|error| (error, self.read_buf.take().unwrap()))
    }

    fn write_page(
        &self,
        page_number: usize,
        buf: &'static mut Self::Page,
    ) -> Result<(), (ErrorCode, &'static mut Self::Page)> {
        if page_number >= FLASH_MAX_PAGES || self.pending_op.is_some() {
            return Err((
                if page_number >= FLASH_MAX_PAGES {
                    ErrorCode::INVAL
                } else {
                    ErrorCode::BUSY
                },
                buf,
            ));
        }
        self.write_buf.replace(buf);
        self.submit_io(FlashOperation::WritePage, page_number)
            .map_err(|error| (error, self.write_buf.take().unwrap()))
    }

    fn erase_page(&self, page_number: usize) -> Result<(), ErrorCode> {
        self.submit_io(FlashOperation::ErasePage, page_number)
    }
}
