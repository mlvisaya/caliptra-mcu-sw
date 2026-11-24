// Licensed under the Apache-2.0 license

use crate::test::TEST_LOCK;
use anyhow::{bail, ensure, Context, Result};
use caliptra_api::SocManager;
use caliptra_mcu_builder::{firmware, FirmwareBinaries};
use caliptra_mcu_hw_model::{new_unbooted, DefaultHwModel, InitParams, McuHwModel};
use caliptra_mcu_registers_generated::mci;
use caliptra_mcu_romtime::StaticRef;
use caliptra_mcu_test_fw_mbox_ownership::*;
use std::time::{Duration, Instant};
use tock_registers::interfaces::{Readable, Writeable};
use zerocopy::{FromBytes, IntoBytes};

struct Fixture {
    hw: DefaultHwModel,
    registers: StaticRef<mci::regs::Mci>,
    sequence: u32,
}

impl Fixture {
    fn new() -> Result<Self> {
        let binaries = FirmwareBinaries::from_env()?;
        let mcu_rom = binaries
            .test_rom(&firmware::MBOX_OWNERSHIP_MCU)
            .context("MCU ownership test ROM missing from firmware bundle")?;
        let core_rom = binaries
            .caliptra_test_rom(&firmware::MBOX_OWNERSHIP_CORE)
            .context("Core ownership test ROM missing from firmware bundle")?;
        let mut hw = new_unbooted(InitParams {
            mcu_rom: &mcu_rom,
            caliptra_rom: &core_rom,
            check_booted_to_runtime: false,
            enable_mcu_uart_log: true,
            debug_intent: true,
            ..Default::default()
        })?;
        hw.boot()?;
        let mapped = hw.base.devs[1]
            .map_size(3)
            .map_err(|error| anyhow::anyhow!("read MCI mapping size: {error:?}"))?;
        ensure!(
            mapped >= 0xa00028,
            "MCI mapping does not cover both mailbox CSRs"
        );
        let mci = hw.base.mmio.mci().context("MCI MMIO disabled")?;
        let registers = unsafe { StaticRef::new(mci.ptr.cast::<mci::regs::Mci>()) };
        let mut fixture = Self {
            hw,
            registers,
            sequence: 0,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while fixture.registers.mci_reg_generic_output_wires[0].get() != READY
            || !fixture
                .hw
                .base
                .soc_ifc()
                .cptra_flow_status()
                .read()
                .ready_for_mb_processing()
        {
            fixture.hw.step();
            if Instant::now() >= deadline {
                bail!(
                    "ownership fixtures not ready: MCU GPIO={:08x?}, MCI flow={:#x}, MCU fatal={:?}, Core flow={:#x}, Core fatal={:#x}, Core nonfatal={:#x}, Core HW fatal={:#x}",
                    [
                        fixture.registers.mci_reg_generic_output_wires[0].get(),
                        fixture.registers.mci_reg_generic_output_wires[1].get()
                    ],
                    fixture.registers.mci_reg_hw_flow_status.get(),
                    fixture.hw.mci_fw_fatal_error(),
                    u32::from(fixture.hw.base.soc_ifc().cptra_flow_status().read()),
                    fixture.hw.base.soc_ifc().cptra_fw_error_fatal().read(),
                    fixture.hw.base.soc_ifc().cptra_fw_error_non_fatal().read(),
                    u32::from(fixture.hw.base.soc_ifc().cptra_hw_error_fatal().read())
                );
            }
        }
        ensure!(
            fixture.hw.base.wrapper.regs().lsu_user.get() == ROOT_USER,
            "unexpected MCU Root AXI user"
        );
        Ok(fixture)
    }

    fn mcu(&mut self, command: u32, mailbox: u32, value: u32) -> Result<u32> {
        self.sequence += 1;
        let tag = (self.sequence << 16) | (mailbox << 8) | command;
        // Publish the value before the command tag, which wakes the MCU fixture.
        self.hw.base.wrapper.regs().mci_generic_input_wires[1].set(value);
        self.hw.base.wrapper.regs().mci_generic_input_wires[0].set(tag);
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.registers.mci_reg_generic_output_wires[1].get() != tag {
            self.hw.step();
            ensure!(
                self.hw.mci_fw_fatal_error().is_none(),
                "MCU fixture fatal error: {:?}",
                self.hw.mci_fw_fatal_error()
            );
            ensure!(
                Instant::now() < deadline,
                "MCU ownership command {command} timed out for mailbox {mailbox}"
            );
        }
        Ok(self.registers.mci_reg_generic_output_wires[0].get())
    }

    fn core(&mut self, command: u32, mailbox: u32, first: u32, last: u32) -> Result<Response> {
        let request = Request {
            mailbox,
            first,
            last,
        };
        let response = self
            .hw
            .caliptra_mailbox_execute(command, request.as_bytes())
            .with_context(|| format!("Core ownership command {command:#x}"))?
            .context("Core ownership command returned no data")?;
        Response::read_from_bytes(&response)
            .map_err(|error| anyhow::anyhow!("invalid Core ownership response: {error:?}"))
    }

    fn csr(&self, mailbox: u32, offset: u32) -> u32 {
        let (_, base) = addresses(mailbox).unwrap();
        let pointer = (&*self.registers as *const mci::regs::Mci).cast::<u8>();
        unsafe {
            pointer
                .add((base + offset) as usize)
                .cast::<u32>()
                .read_volatile()
        }
    }

    fn owner(&self, mailbox: u32) -> u32 {
        (self.csr(mailbox, HW_STATUS) >> 2) & 3
    }

    fn prepare(&mut self, mailbox: u32, pattern: u32) -> Result<()> {
        self.mcu(MCU_ACQUIRE, mailbox, 0)?;
        ensure!(self.owner(mailbox) == 1, "MCU is not the lock holder");
        ensure!(
            self.csr(mailbox, USER) == ROOT_USER,
            "wrong lock holder AXI user"
        );
        self.mcu(MCU_CLEAR_NOTIFICATION, mailbox, 0)?;
        ensure!(
            self.mcu(MCU_FILL, mailbox, pattern)? == pattern,
            "MCU write failed"
        );
        let denied = self.core(CORE_WRITE, mailbox, !pattern, pattern)?;
        ensure!(
            denied.first == 0 && denied.last == 0,
            "Core accessed SRAM before ownership handoff"
        );
        ensure!(
            self.mcu(MCU_READ, mailbox, 0)? == pattern,
            "denied Core write changed MCU-owned memory"
        );
        Ok(())
    }

    fn grant_and_check_denial(&mut self, mailbox: u32, pattern: u32) -> Result<()> {
        ensure!(
            self.mcu(MCU_GRANT, mailbox, 0)? == 3,
            "Core did not receive ownership"
        );
        ensure!(
            self.csr(mailbox, TARGET_STATUS) == 0,
            "target status was not reset to BUSY"
        );
        ensure!(
            self.mcu(MCU_READ, mailbox, 0)? == 0,
            "MCU read Core-owned SRAM"
        );
        ensure!(
            self.mcu(MCU_WRITE, mailbox, 0xdead_beef)? == 0,
            "MCU accessed Core-owned SRAM"
        );
        ensure!(
            self.mcu(MCU_RECLAIM, mailbox, 0xfeed_cafe)? == 3,
            "MCU forcibly reclaimed Core-owned SRAM"
        );
        let response = self.core(CORE_READ, mailbox, 0, 0)?;
        ensure!(
            response.first == pattern && response.last == !pattern,
            "denied MCU writes changed the staged data: {response:?}"
        );
        ensure!(
            response.owner == 3
                && response.dlen == SRAM_BYTES as u32
                && response.command == READY
                && response.target_status == 0,
            "denied MCU CSR writes changed Core-owned mailbox state: {response:?}"
        );
        ensure!(
            self.csr(mailbox, TARGET_USER) == CORE_USER,
            "target configuration changed"
        );
        ensure!(
            self.csr(mailbox, TARGET_VALID) == 1,
            "target configuration was unlocked"
        );
        Ok(())
    }

    fn finish_and_check_access(&mut self, mailbox: u32, status: u32, pattern: u32) -> Result<()> {
        let written = self.core(CORE_WRITE, mailbox, pattern, !pattern)?;
        ensure!(
            written.first == pattern && written.last == !pattern,
            "Core could not update its owned SRAM"
        );
        let returned = self.core(CORE_RETURN, mailbox, status, 0)?;
        ensure!(
            returned.owner == 2 && returned.target_status == status,
            "terminal target status did not return ownership: {returned:?}"
        );
        ensure!(
            self.csr(mailbox, TARGET_USER) == 0 && self.csr(mailbox, TARGET_VALID) == 0,
            "hardware did not clear target configuration"
        );
        let notification = 1 << (3 + mailbox);
        ensure!(
            self.mcu(MCU_NOTIFICATION, mailbox, 0)? & notification != 0,
            "MCU did not receive hardware target-done notification"
        );
        ensure!(
            self.mcu(MCU_READ, mailbox, 0)? == pattern,
            "MCU could not read the returned SRAM"
        );
        let denied = self.core(CORE_READ, mailbox, 0, 0)?;
        ensure!(
            denied.first == 0 && denied.last == 0,
            "Core retained access after returning ownership"
        );
        Ok(())
    }

    fn release_and_check_zeroization(&mut self, mailbox: u32) -> Result<()> {
        self.mcu(MCU_RELEASE, mailbox, 0)?;
        self.mcu(MCU_ACQUIRE, mailbox, 0)?;
        ensure!(
            self.owner(mailbox) == 1,
            "MCU did not reacquire the cleared mailbox"
        );
        ensure!(
            self.mcu(MCU_ZERO_CHECK, mailbox, 0)? == 0,
            "mailbox SRAM was not fully zeroized"
        );
        self.mcu(MCU_RELEASE, mailbox, 0)?;
        Ok(())
    }
}

#[test]
fn test_mcu_mailbox_sram_core_handoff_and_return() -> Result<()> {
    let _lock = TEST_LOCK.lock().unwrap();
    let mut fixture = Fixture::new()?;
    for mailbox in 0..2 {
        let pattern = 0x1234_5678 ^ mailbox;
        fixture.prepare(mailbox, pattern)?;
        fixture.grant_and_check_denial(mailbox, pattern)?;
        fixture.finish_and_check_access(mailbox, 2, !pattern)?;
        fixture.release_and_check_zeroization(mailbox)?;
    }
    Ok(())
}

#[test]
fn test_mcu_mailbox_sram_data_ready_and_failure_return_ownership() -> Result<()> {
    let _lock = TEST_LOCK.lock().unwrap();
    let mut fixture = Fixture::new()?;
    for mailbox in 0..2 {
        for status in [1, 3] {
            let pattern = 0xa5a5_0000 | (mailbox << 8) | status;
            fixture.prepare(mailbox, pattern)?;
            fixture.grant_and_check_denial(mailbox, pattern)?;
            fixture.finish_and_check_access(mailbox, status, !pattern)?;
            fixture.release_and_check_zeroization(mailbox)?;
        }
    }
    Ok(())
}

#[test]
fn test_mcu_mailbox_sram_core_release_zeroizes() -> Result<()> {
    let _lock = TEST_LOCK.lock().unwrap();
    let mut fixture = Fixture::new()?;
    for mailbox in 0..2 {
        fixture.prepare(mailbox, 0x1122_3344)?;
        fixture.grant_and_check_denial(mailbox, 0x1122_3344)?;
        fixture.core(CORE_RELEASE, mailbox, 0, 0)?;
        fixture.mcu(MCU_ACQUIRE, mailbox, 0)?;
        ensure!(
            fixture.owner(mailbox) == 1,
            "mailbox remained unavailable after Core release"
        );
        ensure!(
            fixture.mcu(MCU_ZERO_CHECK, mailbox, 0)? == 0,
            "Core release did not zeroize SRAM"
        );
        fixture.mcu(MCU_RELEASE, mailbox, 0)?;
    }
    Ok(())
}

#[test]
fn test_mcu_mailbox_sram_regrant_clears_stale_target_status() -> Result<()> {
    let _lock = TEST_LOCK.lock().unwrap();
    let mut fixture = Fixture::new()?;
    for mailbox in 0..2 {
        let pattern = 0x5566_7788;
        fixture.prepare(mailbox, pattern)?;
        fixture.grant_and_check_denial(mailbox, pattern)?;
        fixture.finish_and_check_access(mailbox, 1, pattern)?;
        fixture.mcu(MCU_CLEAR_NOTIFICATION, mailbox, 0)?;
        fixture.grant_and_check_denial(mailbox, pattern)?;
        fixture.finish_and_check_access(mailbox, 2, pattern)?;
        fixture.release_and_check_zeroization(mailbox)?;
    }
    Ok(())
}
