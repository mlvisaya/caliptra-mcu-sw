/*++

Licensed under the Apache-2.0 license.

File Name:

    main.rs

Abstract:

    File contains main entry point for Network Coprocessor ROM.

--*/

#![cfg_attr(target_arch = "riscv32", no_std)]
#![no_main]

#[cfg(target_arch = "riscv32")]
use core::panic::PanicInfo;

#[cfg(target_arch = "riscv32")]
use core::arch::global_asm;

use network_drivers::{exit_emulator, println};

// Provide a critical-section implementation for single-threaded bare-metal use.
#[cfg(feature = "test-boot-source")]
mod cs_impl {
    use critical_section::RawRestoreState;
    struct SingleCoreCriticalSection;
    critical_section::set_impl!(SingleCoreCriticalSection);
    unsafe impl critical_section::Impl for SingleCoreCriticalSection {
        unsafe fn acquire() -> RawRestoreState {}
        unsafe fn release(_token: RawRestoreState) {}
    }
}

// Include the startup assembly code
#[cfg(target_arch = "riscv32")]
global_asm!(include_str!("start.s"));

/// Main entry point called from assembly startup code
#[cfg(target_arch = "riscv32")]
#[no_mangle]
pub extern "C" fn main() -> ! {
    use network_drivers::EthernetDriver;

    println!();
    println!("=====================================");
    println!("  Network Coprocessor ROM Started!  ");
    println!("=====================================");
    println!();

    #[cfg(feature = "test-network-rom-dhcp-discover")]
    {
        let eth = EthernetDriver::new();
        network_app_rom_test::dhcp_test::run(eth);
    }

    #[cfg(feature = "test-network-rom-lwip-dhcp")]
    {
        let eth = EthernetDriver::new();
        network_app_rom_test::lwip_dhcp_test::run(eth);
    }

    #[cfg(feature = "test-network-rom-lwip-dhcp6")]
    {
        let eth = EthernetDriver::new();
        network_app_rom_test::lwip_dhcpv6_test::run(eth);
    }

    #[cfg(feature = "test-network-rom-lwip-tftp")]
    {
        let eth = EthernetDriver::new();
        network_app_rom_test::lwip_tftp_test::run(eth);
    }

    #[cfg(feature = "test-network-rom-lwip-tftpv6")]
    {
        let eth = EthernetDriver::new();
        network_app_rom_test::lwip_tftpv6_test::run(eth);
    }

    #[cfg(feature = "test-network-mbox-comm")]
    {
        network_app_rom_test::network_mbox_test::run();
    }

    #[cfg(feature = "test-boot-source")]
    {
        let eth = EthernetDriver::new();
        network_app_rom_test::boot_source_test::run(eth);
    }

    exit_emulator(0x00);
}

/// Exception handler - called when CPU encounters an exception
#[no_mangle]
pub extern "C" fn exception_handler() {
    println!("EXCEPTION: Network ROM encountered an error!");
    exit_emulator(0x01);
}

/// Panic handler for no_std environment
#[cfg(target_arch = "riscv32")]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("PANIC: Network ROM panicked!");
    exit_emulator(0x01);
}

// Dummy main for non-RISC-V targets (for cargo check on host)
#[cfg(not(target_arch = "riscv32"))]
#[no_mangle]
pub extern "C" fn main() {
    println!("Network ROM (host build - no-op)");
}
