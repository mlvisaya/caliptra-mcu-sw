// Licensed under the Apache-2.0 license

#![no_std]
#![no_main]

#[cfg(not(hw_rev = "latest"))]
compile_error!("MCU mailbox ownership fixture requires latest hardware");

use caliptra_mcu_rom_common::RomEnv;
use caliptra_mcu_test_fw_mbox_ownership::*;
use tock_registers::interfaces::{ReadWriteable, Readable, Writeable};

#[no_mangle]
pub extern "C" fn main() -> ! {
    caliptra_mcu_test_harness::set_printer();
    caliptra_mcu_romtime::println!("[mbox-owner-mcu] starting");
    let env = RomEnv::new();
    let regs = &env.mci.registers;
    regs.mci_reg_mbox0_valid_axi_user[0].set(CORE_USER);
    regs.mci_reg_mbox1_valid_axi_user[0].set(CORE_USER);
    caliptra_mcu_romtime::println!("[mbox-owner-mcu] target users configured");
    let base = caliptra_mcu_test_harness::MCU_MEMORY_MAP.mci_offset as usize;
    let read =
        |offset: u32| unsafe { core::ptr::read_volatile((base + offset as usize) as *const u32) };
    let write = |offset: u32, value| unsafe {
        core::ptr::write_volatile((base + offset as usize) as *mut u32, value)
    };
    for index in 0..2 {
        let (_, csr) = addresses(index).unwrap();
        write(csr + DLEN, SRAM_BYTES as u32);
        write(csr + EXECUTE, 0);
    }
    caliptra_mcu_romtime::println!("[mbox-owner-mcu] reset locks released");
    regs.intr_block_rf_notif0_intr_en_r.modify(
        caliptra_mcu_registers_generated::mci::bits::Notif0IntrEnT::NotifMbox0TargetDoneEn::SET
            + caliptra_mcu_registers_generated::mci::bits::Notif0IntrEnT::NotifMbox1TargetDoneEn::SET,
    );
    regs.mci_reg_generic_output_wires[0].set(READY);
    regs.mci_reg_generic_output_wires[1].set(0);
    env.mci.caliptra_boot_go();
    while !env.soc.ready_for_fuses() {}
    env.soc.set_ss_caliptra_dma_axi_user(CORE_USER);
    env.soc.fuse_write_done();
    caliptra_mcu_romtime::println!("[mbox-owner-mcu] ready");
    let mut previous = 0;
    loop {
        let tag = regs.mci_reg_generic_input_wires[0].get();
        if tag == previous {
            continue;
        }
        let value = regs.mci_reg_generic_input_wires[1].get();
        let (sram, csr) = addresses((tag >> 8) & 1).unwrap();
        let result = match tag & 0xff {
            MCU_ACQUIRE => {
                while read(csr + LOCK) != 0 {}
                write(csr + DLEN, SRAM_BYTES as u32);
                write(csr + COMMAND, READY);
                0
            }
            MCU_FILL => {
                write(sram, value);
                write(sram + SRAM_BYTES as u32 - 4, !value);
                read(sram)
            }
            MCU_GRANT => {
                write(csr + TARGET_USER, CORE_USER);
                write(csr + TARGET_VALID, 1);
                write(csr + EXECUTE, 1);
                (read(csr + HW_STATUS) >> 2) & 3
            }
            MCU_READ => read(sram),
            MCU_WRITE => {
                write(sram, value);
                write(sram + SRAM_BYTES as u32 - 4, value);
                read(sram)
            }
            MCU_RECLAIM => {
                write(csr + TARGET_USER, ROOT_USER);
                write(csr + TARGET_VALID, 0);
                write(csr + DLEN, 4);
                write(csr + COMMAND, value);
                write(csr + TARGET_STATUS, 3);
                write(csr + EXECUTE, 0);
                (read(csr + HW_STATUS) >> 2) & 3
            }
            MCU_RELEASE => {
                write(csr + EXECUTE, 0);
                0
            }
            MCU_ZERO_CHECK => {
                let mut combined = 0;
                for offset in (0..SRAM_BYTES as u32).step_by(4) {
                    combined |= read(sram + offset);
                }
                combined
            }
            MCU_NOTIFICATION => regs.intr_block_rf_notif0_internal_intr_r.get(),
            MCU_CLEAR_NOTIFICATION => {
                regs.intr_block_rf_notif0_internal_intr_r
                    .set(1 << (3 + ((tag >> 8) & 1)));
                0
            }
            _ => panic!("unknown ownership test GPIO command"),
        };
        regs.mci_reg_generic_output_wires[0].set(result);
        regs.mci_reg_generic_output_wires[1].set(tag);
        previous = tag;
    }
}
