// Licensed under the Apache-2.0 license

#![no_std]
#![no_main]

#[cfg(not(hw_rev = "latest"))]
compile_error!("MCU mailbox ownership fixture requires latest hardware");

use caliptra_drivers::{AxiAddr, Dma, Mailbox, SocIfc};
use caliptra_mcu_test_fw_mbox_ownership::*;
use caliptra_registers::{mbox::MboxCsr, soc_ifc::SocIfcReg};
use caliptra_test_harness::test_suite;
use zerocopy::{FromBytes, IntoBytes};

test_suite! { serve, }

fn serve() {
    let soc_regs = unsafe { SocIfcReg::new() };
    assert_eq!(
        u32::from(soc_regs.regs().ss_caliptra_dma_axi_user().read()),
        CORE_USER
    );
    let mut soc = SocIfc::new(soc_regs);
    let dma = Dma::default();
    let base = AxiAddr::from(soc.mci_base_addr());
    let mut mailbox = unsafe { Mailbox::new(MboxCsr::new()) };
    soc.flow_status_set_ready_for_mb_processing();
    loop {
        let Some(mut transaction) = mailbox.try_start_recv_txn() else {
            continue;
        };
        let mut bytes = [0; core::mem::size_of::<Request>()];
        if transaction.dlen() as usize != bytes.len()
            || transaction.recv_request(&mut bytes).is_err()
        {
            transaction.complete(false).unwrap();
            continue;
        }
        let request = Request::read_from_bytes(&bytes).unwrap();
        let Some((sram, csr)) = addresses(request.mailbox) else {
            transaction.complete(false).unwrap();
            continue;
        };
        let read = |offset: u32| dma.read_dword(base + offset as usize);
        let owner = (read(csr + HW_STATUS) >> 2) & 3;
        match transaction.cmd() {
            CORE_READ => {}
            CORE_WRITE => {
                dma.write_dword(base + sram as usize, request.first);
                dma.write_dword(base + (sram as usize + SRAM_BYTES - 4), request.last);
            }
            CORE_RETURN
                if owner == 3
                    && read(csr + TARGET_USER) == CORE_USER
                    && (1..=3).contains(&request.first)
                    && request.last == 0 =>
            {
                dma.write_dword(base + (csr + TARGET_STATUS) as usize, request.first);
            }
            CORE_RELEASE if owner == 3 && request.first == 0 && request.last == 0 => {
                dma.write_dword(base + (csr + EXECUTE) as usize, 0);
            }
            _ => {
                transaction.complete(false).unwrap();
                continue;
            }
        }
        let response = Response {
            first: read(sram),
            last: read(sram + SRAM_BYTES as u32 - 4),
            owner: (read(csr + HW_STATUS) >> 2) & 3,
            target_status: read(csr + TARGET_STATUS),
            dlen: read(csr + DLEN),
            command: read(csr + COMMAND),
        };
        transaction.send_response(response.as_bytes()).unwrap();
    }
}
