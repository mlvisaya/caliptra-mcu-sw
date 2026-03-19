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
    #[allow(unused_imports)]
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

    // Default: run the boot-source application.
    // When a test feature is active the test block above handles execution;
    // otherwise the Network CoP serves boot-source protocol requests.
    #[cfg(not(any(
        feature = "test-network-rom-dhcp-discover",
        feature = "test-network-rom-lwip-dhcp",
        feature = "test-network-rom-lwip-dhcp6",
        feature = "test-network-rom-lwip-tftp",
        feature = "test-network-rom-lwip-tftpv6",
        feature = "test-network-mbox-comm",
    )))]
    {
        run_boot_source_app();
    }

    exit_emulator(0x00);
}

/// Exception handler - called when CPU encounters an exception
#[no_mangle]
pub extern "C" fn exception_handler() {
    println!("EXCEPTION: Network ROM encountered an error!");
    exit_emulator(0x01);
}

/// Run the boot-source provider application.
///
/// Initializes the lwIP network stack, creates the [`BootSourceApp`],
/// and enters the main polling loop. Exits on completion or error.
#[cfg(target_arch = "riscv32")]
fn run_boot_source_app() {
    use network_app_boot_source::app::BootSourceApp;
    use network_drivers::network_mbox::NetworkMboxDriver;
    use network_drivers::{EthernetDriver, TimerDriver};

    static mut ETH_STORAGE: Option<EthernetDriver> = None;
    static mut TIMER_STORAGE: Option<TimerDriver> = None;
    unsafe {
        *core::ptr::addr_of_mut!(ETH_STORAGE) = Some(EthernetDriver::new());
        *core::ptr::addr_of_mut!(TIMER_STORAGE) = Some(TimerDriver::new());
    }
    let eth_ref: &'static mut dyn network_hil::ethernet::Ethernet =
        unsafe { (*core::ptr::addr_of_mut!(ETH_STORAGE)).as_mut().unwrap() };
    let timer_ref: &'static dyn network_hil::timers::Timers =
        unsafe { (*core::ptr::addr_of!(TIMER_STORAGE)).as_ref().unwrap() };

    let driver = NetworkMboxDriver::new();
    let app = BootSourceApp::new(&driver, 1024);

    if let Err(e) = app.init(eth_ref, timer_ref) {
        println!("[boot-src] ERROR: init failed: {:?}", e);
        exit_emulator(0x01);
    }

    println!("[boot-src] Initialized, waiting for requests...");

    match app.run_loop(50_000_000) {
        Ok(()) => {
            println!("[boot-src] Protocol complete");
        }
        Err(e) => {
            println!("[boot-src] ERROR: run_loop failed: {:?}", e);
            exit_emulator(0x01);
        }
    }
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
