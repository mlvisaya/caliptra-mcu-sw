// Licensed under the Apache-2.0 license

//! Integration test for the boot-source protocol over the network mailbox.
//!
//! The MCU ROM sends `ImageMetadataRequest` and `Finalize` commands to
//! the Network CoP, which runs the `BootSourceApp` with a pre-populated
//! TOC.  Both sides verify their respective messages and print PASSED.

#[cfg(test)]
mod test {
    use crate::test::{start_runtime_hw_model, TestParams, TEST_LOCK};
    use mcu_hw_model::McuHwModel;

    #[test]
    #[cfg_attr(feature = "fpga_realtime", ignore)]
    fn test_boot_source_protocol() {
        let lock = TEST_LOCK.lock().unwrap();
        lock.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut hw = start_runtime_hw_model(TestParams {
            include_network_rom: true,
            rom_only: true,
            rom_feature: Some("test-boot-source"),
            network_rom_feature: Some("test-boot-source"),
            ..Default::default()
        });

        assert!(hw.has_network_cpu(), "Network CPU should be present");

        const MAX_CYCLES: u64 = 100_000_000;
        let mut net_passed = false;

        hw.step_until(|m| {
            if m.cycle_count() >= MAX_CYCLES {
                return true;
            }

            if !net_passed {
                if let Some(net_output) = m.network_uart_output() {
                    if net_output.contains("test PASSED!") {
                        net_passed = true;
                    }
                }
            }

            net_passed
        });

        if let Some(net_output) = hw.network_uart_output() {
            println!("Network CPU UART output:\n{}", net_output);
        }

        assert!(
            net_passed,
            "Network CoP should report boot source test PASSED"
        );

        lock.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}
