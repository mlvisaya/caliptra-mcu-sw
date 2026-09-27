// Licensed under the Apache-2.0 license

// HTG940_STANDALONE_FIRMWARE_BRAM
// The original VCK190 FPGA flow delegated flash operations to a Linux-side
// realtime model.  HTG-940 has no PS, so the MCU ROM reads and writes a 2 MiB
// AXI BRAM directly.  Slot A is initialized from flash_image.bin by Vivado.

use caliptra_mcu_registers_generated::primary_flash_ctrl::regs::PrimaryFlashCtrl;
use caliptra_mcu_rom_common::flash::hil::{FlashDrvError, FlashStorage};
use caliptra_mcu_romtime::StaticRef;
use core::ops::{Index, IndexMut};

pub const FPGA_PRIMARY_FLASH_CTRL_ADDR: u32 = 0xA401_2000;
pub const FPGA_SECONDARY_FLASH_CTRL_ADDR: u32 = 0xA401_3000;
pub const FLASH_PAGE_BUFFER_SRAM_OFFSET: u32 = 0xA401_2100;

pub const PRIMARY_FLASH_CTRL_BASE: StaticRef<PrimaryFlashCtrl> =
    unsafe { StaticRef::new(FPGA_PRIMARY_FLASH_CTRL_ADDR as *const PrimaryFlashCtrl) };
pub const SECONDARY_FLASH_CTRL_BASE: StaticRef<PrimaryFlashCtrl> =
    unsafe { StaticRef::new(FPGA_SECONDARY_FLASH_CTRL_ADDR as *const PrimaryFlashCtrl) };

pub const HTG940_FLASH_BASE: usize = 0xB020_0000;
pub const HTG940_FLASH_CAPACITY: usize = 2 * 1024 * 1024;
pub const HTG940_PRIMARY_CAPACITY: usize = 1024 * 1024;
pub const HTG940_SECONDARY_BASE: usize = HTG940_FLASH_BASE + HTG940_PRIMARY_CAPACITY;
pub const HTG940_SECONDARY_CAPACITY: usize = 12 * 64 * 1024;
const PAGE_SIZE: usize = 256;

#[derive(Debug, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub enum FlashOperation {
    ReadPage = 1,
    WritePage = 2,
    ErasePage = 3,
}

#[derive(Debug)]
pub struct FpgaFlashPage(pub [u8; PAGE_SIZE]);

impl Default for FpgaFlashPage {
    fn default() -> Self {
        Self([0; PAGE_SIZE])
    }
}

impl Index<usize> for FpgaFlashPage {
    type Output = u8;
    fn index(&self, idx: usize) -> &u8 {
        &self.0[idx]
    }
}

impl IndexMut<usize> for FpgaFlashPage {
    fn index_mut(&mut self, idx: usize) -> &mut u8 {
        &mut self.0[idx]
    }
}

impl AsMut<[u8]> for FpgaFlashPage {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

pub struct FpgaFlashCtrl {
    base: usize,
    capacity: usize,
}

impl FlashStorage for FpgaFlashCtrl {
    fn read(&self, buf: &mut [u8], offset: usize) -> Result<(), FlashDrvError> {
        let end = offset.checked_add(buf.len()).ok_or(FlashDrvError::INVAL)?;
        if end > self.capacity {
            return Err(FlashDrvError::INVAL);
        }
        let src = (self.base + offset) as *const u8;
        for (index, byte) in buf.iter_mut().enumerate() {
            *byte = unsafe { core::ptr::read_volatile(src.add(index)) };
        }
        Ok(())
    }

    fn write(&self, buf: &[u8], offset: usize) -> Result<(), FlashDrvError> {
        let end = offset.checked_add(buf.len()).ok_or(FlashDrvError::INVAL)?;
        if end > self.capacity {
            return Err(FlashDrvError::INVAL);
        }
        let dst = (self.base + offset) as *mut u8;
        for (index, byte) in buf.iter().copied().enumerate() {
            unsafe { core::ptr::write_volatile(dst.add(index), byte) };
        }
        Ok(())
    }

    fn erase(&self, offset: usize, len: usize) -> Result<(), FlashDrvError> {
        let end = offset.checked_add(len).ok_or(FlashDrvError::INVAL)?;
        if end > self.capacity {
            return Err(FlashDrvError::INVAL);
        }
        let dst = (self.base + offset) as *mut u8;
        for index in 0..len {
            unsafe { core::ptr::write_volatile(dst.add(index), 0xFF) };
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.capacity
    }
}

impl FpgaFlashCtrl {
    pub fn initialize_flash_ctrl(_base: StaticRef<PrimaryFlashCtrl>) -> FpgaFlashCtrl {
        FpgaFlashCtrl {
            base: HTG940_FLASH_BASE,
            capacity: HTG940_PRIMARY_CAPACITY,
        }
    }

    pub fn initialize_flash_region(base: usize, capacity: usize) -> FpgaFlashCtrl {
        FpgaFlashCtrl { base, capacity }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[allow(dead_code)]
    fn read_page(&self, page_number: usize, buf: &mut FpgaFlashPage) -> Result<(), FlashDrvError> {
        if page_number >= self.capacity / PAGE_SIZE {
            return Err(FlashDrvError::INVAL);
        }
        self.read(&mut buf.0, page_number * PAGE_SIZE)
    }

    #[allow(dead_code)]
    fn write_page(&self, page_number: usize, buf: &FpgaFlashPage) -> Result<(), FlashDrvError> {
        if page_number >= self.capacity / PAGE_SIZE {
            return Err(FlashDrvError::INVAL);
        }
        self.write(&buf.0, page_number * PAGE_SIZE)
    }

    #[allow(dead_code)]
    fn erase_page(&self, page_number: usize) -> Result<(), FlashDrvError> {
        if page_number >= self.capacity / PAGE_SIZE {
            return Err(FlashDrvError::INVAL);
        }
        self.erase(page_number * PAGE_SIZE, PAGE_SIZE)
    }
}
