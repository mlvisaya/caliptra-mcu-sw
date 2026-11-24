// Licensed under the Apache-2.0 license

#![no_std]

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

pub const CORE_USER: u32 = 2;
pub const ROOT_USER: u32 = 1;
pub const SRAM_BYTES: usize = 16 * 1024;
pub const READY: u32 = 0x4f57_4e52;

pub const CORE_READ: u32 = 0x4f57_5244;
pub const CORE_WRITE: u32 = 0x4f57_5752;
pub const CORE_RETURN: u32 = 0x4f57_444e;
pub const CORE_RELEASE: u32 = 0x4f57_524c;

pub const MCU_ACQUIRE: u32 = 1;
pub const MCU_FILL: u32 = 2;
pub const MCU_GRANT: u32 = 3;
pub const MCU_READ: u32 = 4;
pub const MCU_WRITE: u32 = 5;
pub const MCU_RECLAIM: u32 = 6;
pub const MCU_RELEASE: u32 = 7;
pub const MCU_ZERO_CHECK: u32 = 8;
pub const MCU_NOTIFICATION: u32 = 9;
pub const MCU_CLEAR_NOTIFICATION: u32 = 10;

pub const LOCK: u32 = 0;
pub const USER: u32 = 4;
pub const TARGET_USER: u32 = 8;
pub const TARGET_VALID: u32 = 12;
pub const COMMAND: u32 = 16;
pub const DLEN: u32 = 20;
pub const EXECUTE: u32 = 24;
pub const TARGET_STATUS: u32 = 28;
pub const HW_STATUS: u32 = 36;

pub fn addresses(mailbox: u32) -> Option<(u32, u32)> {
    if mailbox < 2 {
        Some((0x400000 + mailbox * 0x400000, 0x600000 + mailbox * 0x400000))
    } else {
        None
    }
}

#[repr(C)]
#[derive(Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct Request {
    pub mailbox: u32,
    pub first: u32,
    pub last: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct Response {
    pub first: u32,
    pub last: u32,
    pub owner: u32,
    pub target_status: u32,
    pub dlen: u32,
    pub command: u32,
}
