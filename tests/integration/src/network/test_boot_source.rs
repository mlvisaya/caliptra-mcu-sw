// Licensed under the Apache-2.0 license

//! Integration test for the boot-source protocol over the network mailbox.

#[cfg(test)]
mod test {
    use crate::test::{start_runtime_hw_model, TestParams, TEST_LOCK};
    use emulator_periph::LinuxTapDevice;
    use flash_image::{
        FlashHeader, ImageHeader, HEADER_VERSION, MCU_RT_IDENTIFIER, TFTP_IMAGE_MAGIC_NUMBER,
    };
    use mcu_hw_model::McuHwModel;
    use std::sync::{Arc, Mutex};
    use zerocopy::IntoBytes;

    const TAP_INTERFACE: &str = "tap0";
    const TAP_IP_ADDR: &str = "192.168.100.1";
    const TFTP_BOOT_FILENAME: &str = "toc.bin";
    const TEST_IMAGE_SIZE: u32 = 4096;

    /// Build a TOC-only flash image (FlashHeader + ImageHeaders, no image data).
    fn build_toc_image() -> Vec<u8> {
        let header_size = core::mem::size_of::<FlashHeader>();
        let img_hdr_size = core::mem::size_of::<ImageHeader>();

        // Dummy image data to compute a valid image_checksum.
        let dummy_data: Vec<u8> = (0..TEST_IMAGE_SIZE as usize)
            .map(|i| (i & 0xFF) as u8)
            .collect();
        let image_checksum = mcu_builder::flash_image::calculate_checksum(&dummy_data);

        // Build filename field.
        let mut filename = [0u8; 64];
        let fname = b"mcu_rt.bin";
        filename[..fname.len()].copy_from_slice(fname);

        let mut img_hdr = ImageHeader {
            identifier: MCU_RT_IDENTIFIER,
            offset: (header_size + img_hdr_size) as u32, // points past headers
            size: TEST_IMAGE_SIZE,
            filename,
            image_checksum,
            image_header_checksum: 0,
        };
        // Compute image header checksum.
        let hdr_bytes_for_cksum =
            &img_hdr.as_bytes()[..core::mem::offset_of!(ImageHeader, image_header_checksum)];
        img_hdr.image_header_checksum =
            mcu_builder::flash_image::calculate_checksum(hdr_bytes_for_cksum);

        // Build the flash header.
        let mut flash_hdr = FlashHeader {
            magic: TFTP_IMAGE_MAGIC_NUMBER.into(),
            version: HEADER_VERSION,
            image_count: 1,
            image_headers_offset: header_size as u32,
            header_checksum: 0,
        };
        let fh_bytes_for_cksum =
            &flash_hdr.as_bytes()[..core::mem::offset_of!(FlashHeader, header_checksum)];
        flash_hdr.header_checksum =
            mcu_builder::flash_image::calculate_checksum(fh_bytes_for_cksum);

        // Concatenate: FlashHeader + ImageHeader (no image data).
        let mut out = Vec::with_capacity(header_size + img_hdr_size);
        out.extend_from_slice(flash_hdr.as_bytes());
        out.extend_from_slice(img_hdr.as_bytes());
        out
    }

    #[test]
    #[cfg_attr(feature = "fpga_realtime", ignore)]
    fn test_boot_source_protocol() {
        use xtask::network::{server, server::ServerOptions, tap};

        let lock = TEST_LOCK.lock().unwrap();
        lock.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // --- Prerequisites ---
        if !tap::has_sudo_access() {
            eprintln!("SKIP: No passwordless sudo access");
            return;
        }
        if !tap::interface_exists(TAP_INTERFACE) {
            println!("TAP interface {} not found, setting up...", TAP_INTERFACE);
            if let Err(e) = tap::setup(TAP_INTERFACE, TAP_IP_ADDR, true) {
                eprintln!("Failed to set up TAP interface: {}", e);
                return;
            }
        }
        if !server::is_installed() {
            eprintln!("SKIP: dnsmasq not installed");
            return;
        }
        if server::is_running() {
            let _ = server::stop();
        }

        // --- Build TOC image and write to TFTP directory ---
        let tftp_dir = std::env::temp_dir().join("boot-source-tftp-test");
        std::fs::create_dir_all(&tftp_dir).expect("Failed to create TFTP directory");
        let toc_bytes = build_toc_image();
        std::fs::write(tftp_dir.join(TFTP_BOOT_FILENAME), &toc_bytes)
            .expect("Failed to write TOC file");

        // Write the firmware image file referenced by the TOC.
        let firmware_data: Vec<u8> = (0..TEST_IMAGE_SIZE as usize)
            .map(|i| (i & 0xFF) as u8)
            .collect();
        std::fs::write(tftp_dir.join("mcu_rt.bin"), &firmware_data)
            .expect("Failed to write firmware image file");

        // --- Start dnsmasq with DHCP + TFTP ---
        let server_options = ServerOptions {
            interface: TAP_INTERFACE.to_string(),
            enable_tftp: true,
            tftp_root: Some(tftp_dir.clone()),
            boot_file: TFTP_BOOT_FILENAME.to_string(),
            ..Default::default()
        };
        if let Err(e) = server::start(&server_options) {
            eprintln!("Failed to start dnsmasq: {}", e);
            let _ = std::fs::remove_dir_all(&tftp_dir);
            return;
        }

        // --- Open TAP device ---
        let tap_device = match LinuxTapDevice::open(TAP_INTERFACE) {
            Ok(tap) => Arc::new(Mutex::new(
                Box::new(tap) as Box<dyn emulator_periph::TapDevice>
            )),
            Err(e) => {
                eprintln!("Failed to open TAP device: {}", e);
                let _ = server::stop();
                let _ = std::fs::remove_dir_all(&tftp_dir);
                return;
            }
        };

        // --- Launch emulator ---
        let mut hw = start_runtime_hw_model(TestParams {
            include_network_rom: true,
            rom_only: true,
            rom_feature: Some("test-boot-source"),
            network_rom_feature: None,
            network_tap_device: Some(tap_device),
            ..Default::default()
        });

        assert!(hw.has_network_cpu(), "Network CPU should be present");

        // DHCP + TFTP + protocol exchange needs more cycles than mailbox-only tests.
        // The DHCP timeout alone can consume ~100M cycles, and then the TFTP
        // and protocol exchanges need additional headroom.
        const MAX_CYCLES: u64 = 500_000_000;
        let mut net_passed = false;

        hw.step_until(|m| {
            if m.cycle_count() >= MAX_CYCLES {
                return true;
            }

            if !net_passed {
                if let Some(net_output) = m.network_uart_output() {
                    if net_output.contains("Protocol complete") {
                        net_passed = true;
                    }
                }
            }

            net_passed
        });

        // --- Cleanup ---
        let _ = server::stop();
        let _ = std::fs::remove_dir_all(&tftp_dir);

        if let Some(net_output) = hw.network_uart_output() {
            println!("Network CPU UART output:\n{}", net_output);
        }

        assert!(net_passed, "Network CoP should report protocol complete");

        lock.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}
