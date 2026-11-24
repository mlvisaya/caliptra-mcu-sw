// Licensed under the Apache-2.0 license

use crate::test::{
    finish_runtime_hw_model, start_runtime_hw_model, CustomCaliptraFw, TestParams, TEST_LOCK,
};
use anyhow::{ensure, Context, Result};
use caliptra_mcu_builder::FirmwareBinaries;

fn run(feature: &str) -> Result<()> {
    let _lock = TEST_LOCK.lock().unwrap();
    let binaries = FirmwareBinaries::from_env()?;
    ensure!(
        !binaries.caliptra_fw_mbox_ownership.is_empty(),
        "Core ownership test runtime missing: rebuild the latest FPGA firmware bundle"
    );
    let custom = CustomCaliptraFw {
        fw_bytes: binaries.caliptra_fw_mbox_ownership.clone(),
        vendor_pk_hash: binaries
            .vendor_pk_hash()
            .context("Core firmware vendor key hash missing")?,
        soc_manifest: binaries
            .test_soc_manifest(feature)
            .context("ownership test MCU runtime manifest missing")?,
    };
    let mut hw = start_runtime_hw_model(TestParams {
        feature: Some(feature),
        custom_caliptra_fw: Some(custom),
        caliptra_dma_axi_user: Some(2),
        ..Default::default()
    });
    ensure!(
        finish_runtime_hw_model(&mut hw) == 0,
        "MCU runtime ownership test failed: {feature}"
    );
    Ok(())
}

#[test]
fn test_mcu_mailbox_sram_denies_mcu_access_while_core_spins() -> Result<()> {
    run("test-mcu-mbox-sram-denied")
}

#[test]
fn test_mcu_mailbox_sram_mcu_access_after_core_completion() -> Result<()> {
    run("test-mcu-mbox-sram-return")
}
