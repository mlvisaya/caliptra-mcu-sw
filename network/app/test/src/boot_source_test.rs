/*++

Licensed under the Apache-2.0 license.

File Name:

    boot_source_test.rs

Abstract:

    Network Coprocessor integration test for the boot-source protocol.

    This test app creates a BootSourceApp with a real lwIP network stack,
    so the MCU's InitiateBootRequest triggers a full DHCP + TFTP flow to
    retrieve the TOC from the dnsmasq server set up by the integration
    test. The app's run_loop() handles all protocol exchanges and returns
    when the state transitions through Ready back to Idle after Finalize.

--*/

use network_app_boot_source::app::BootSourceApp;
use network_drivers::network_mbox::NetworkMboxDriver;
use network_drivers::TimerDriver;
use network_drivers::EthernetDriver;
use network_drivers::{exit_emulator, println};

pub fn run(eth: EthernetDriver) {
    println!();
    println!("=====================================");
    println!("  Boot Source Protocol Test Started! ");
    println!("=====================================");
    println!();

    // Store in statics so we can hand out 'static references.
    static mut ETH_STORAGE: Option<EthernetDriver> = None;
    static mut TIMER_STORAGE: Option<TimerDriver> = None;
    unsafe {
        *core::ptr::addr_of_mut!(ETH_STORAGE) = Some(eth);
        *core::ptr::addr_of_mut!(TIMER_STORAGE) = Some(TimerDriver::new());
    }
    let eth_ref: &'static mut dyn network_hil::ethernet::Ethernet =
        unsafe { (*core::ptr::addr_of_mut!(ETH_STORAGE)).as_mut().unwrap() };
    let timer_ref: &'static dyn network_hil::timers::Timers =
        unsafe { (*core::ptr::addr_of!(TIMER_STORAGE)).as_ref().unwrap() };

    let driver = NetworkMboxDriver::new();
    let app = BootSourceApp::new(&driver, 1024);

    // Initialize with real lwIP network stack — DHCP + TFTP will be
    // driven by the InitiateBootRequest handler.
    if let Err(e) = app.init(eth_ref, timer_ref) {
        println!("[boot-src] ERROR: init failed: {:?}", e);
        exit_emulator(0x01);
    }

    println!("[boot-src] Initialized, waiting for requests...");

    match app.run_loop(50_000_000) {
        Ok(()) => {
            println!("[boot-src] State returned to Idle — test PASSED!");
            exit_emulator(0x00);
        }
        Err(e) => {
            println!("[boot-src] ERROR: run_loop failed: {:?}", e);
            exit_emulator(0x01);
        }
    }
}
