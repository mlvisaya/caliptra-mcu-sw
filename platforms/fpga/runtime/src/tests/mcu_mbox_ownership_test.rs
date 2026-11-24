// Licensed under the Apache-2.0 license

#[cfg(not(hw_rev = "latest"))]
compile_error!("MCU mailbox SRAM ownership tests require 2.2 hardware");

use caliptra_api::{calc_checksum, SocManager};
use caliptra_mcu_registers_generated::mci;
use caliptra_mcu_romtime::{CaliptraSoC, StaticRef};
use tock_registers::interfaces::{ReadWriteable, Readable, Writeable};

const COMMAND: u32 = 0x4f57_4e48; // OWNH: checksum, mailbox index, SRAM pattern
const HOLDING: u32 = 0x484f_4c44;
const RESUME: u32 = 0x5253_4d00;
const SRAM_WORDS: usize = 4096;

fn wait(mut condition: impl FnMut() -> bool, message: &str) {
    for _ in 0..2_000_000 {
        if condition() {
            return;
        }
        core::hint::spin_loop();
    }
    panic!("{message}");
}

pub(crate) fn run(attempt_denied_access: bool) -> Option<u32> {
    let regs = unsafe { StaticRef::new(crate::MCU_MEMORY_MAP.mci_offset as *const mci::regs::Mci) };
    let mut soc = CaliptraSoC::new(
        Some(crate::MCU_MEMORY_MAP.soc_offset),
        None,
        Some(crate::MCU_MEMORY_MAP.mbox_offset),
    );
    wait(
        || soc.soc_ifc().cptra_flow_status().read().ready_for_runtime(),
        "Core runtime did not become ready",
    );
    let core_user = soc.soc_ifc().ss_caliptra_dma_axi_user().read();
    regs.intr_block_rf_notif0_intr_en_r.modify(
        mci::bits::Notif0IntrEnT::NotifMbox0TargetDoneEn::SET
            + mci::bits::Notif0IntrEnT::NotifMbox1TargetDoneEn::SET,
    );

    for mailbox in 0..2 {
        let base = crate::MCU_MEMORY_MAP.mci_offset as usize;
        let sram = base + 0x400000 + mailbox * 0x400000;
        let csr = base + 0x600000 + mailbox * 0x400000;
        let read = |offset: usize| unsafe { (offset as *const u32).read_volatile() };
        let write = |offset: usize, value: u32| unsafe {
            (offset as *mut u32).write_volatile(value);
        };
        wait(|| read(csr) == 0, "MCU could not acquire mailbox SRAM");
        let root_user = read(csr + 4);
        assert_ne!(
            root_user, core_user,
            "Core and MCU must have distinct AXI users"
        );
        assert_eq!((read(csr + 36) >> 2) & 3, 1);
        let pattern = 0x1234_5678 ^ mailbox as u32;
        for index in 0..SRAM_WORDS {
            write(sram + index * 4, pattern ^ index as u32);
        }
        regs.intr_block_rf_notif0_internal_intr_r
            .set(1 << (3 + mailbox));
        regs.mci_reg_generic_output_wires[0].set(0);
        write(csr + 16, COMMAND);
        write(csr + 20, (SRAM_WORDS * 4) as u32);
        write(csr + 8, core_user);
        write(csr + 12, 1);
        write(csr + 24, 1);
        assert_eq!((read(csr + 36) >> 2) & 3, 3);

        let mut request = [0u8; 12];
        request[4..8].copy_from_slice(&(mailbox as u32).to_le_bytes());
        request[8..12].copy_from_slice(&pattern.to_le_bytes());
        let checksum = calc_checksum(COMMAND, &request[4..]);
        request[..4].copy_from_slice(&checksum.to_le_bytes());
        soc.start_mailbox_exec(COMMAND, &request).unwrap();
        wait(
            || {
                assert!(soc.is_mailbox_busy(), "Core finished before the MCU probe");
                soc.soc_ifc().cptra_generic_output_wires().at(1).read() == HOLDING | mailbox as u32
            },
            "Core did not acknowledge holding SRAM",
        );
        if attempt_denied_access {
            for index in 0..SRAM_WORDS {
                assert_eq!(read(sram + index * 4), 0, "MCU read Core-owned SRAM");
                write(sram + index * 4, 0xdead_beef);
                assert_eq!(read(sram + index * 4), 0, "MCU read back a denied write");
            }
            assert_eq!((read(csr + 36) >> 2) & 3, 3);
        }
        assert!(soc.is_mailbox_busy(), "Core completed during the MCU probe");
        regs.mci_reg_generic_output_wires[0].set(RESUME | mailbox as u32);
        let mut response = [0u8; 8];
        let response = soc.finish_mailbox_exec(&mut response).unwrap().unwrap();
        assert_eq!(response.len(), 8);
        assert_eq!(
            u32::from_le_bytes(response[..4].try_into().unwrap()),
            calc_checksum(0, &response[4..])
        );
        assert_eq!(&response[4..], &[0; 4]);
        assert_eq!((read(csr + 36) >> 2) & 3, 2);
        assert_eq!(read(csr + 28), 2);
        assert_eq!(read(csr + 8), 0);
        assert_eq!(read(csr + 12), 0);
        assert_ne!(
            regs.intr_block_rf_notif0_internal_intr_r.get() & (1 << (3 + mailbox)),
            0,
            "MCU did not receive target-done notification"
        );
        for index in 0..SRAM_WORDS {
            assert_eq!(read(sram + index * 4), !pattern ^ index as u32);
            write(sram + index * 4, pattern ^ index as u32);
            assert_eq!(read(sram + index * 4), pattern ^ index as u32);
        }
        write(csr + 24, 0);
        caliptra_mcu_romtime::println!(
            "[mbox-ownership] mailbox {} passed (denied-access probe={})",
            mailbox,
            attempt_denied_access
        );
    }
    Some(0)
}
