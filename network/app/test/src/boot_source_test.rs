/*++

Licensed under the Apache-2.0 license.

File Name:

    boot_source_test.rs

Abstract:

    Network Coprocessor integration test for the boot-source protocol.

    This test app creates a BootSourceApp with a real lwIP network stack,
    so the MCU's InitiateBootRequest triggers a full DHCP + TFTP flow to
    retrieve the TOC from the dnsmasq server set up by the integration
    test. After the TOC is parsed, ImageMetadataRequest and Finalize
    messages are exercised as before.

--*/

use network_app_boot_source::app::{AppState, BootSourceApp};
use network_drivers::network_mbox::NetworkMboxDriver;
use network_drivers::TimerDriver;
use network_drivers::EthernetDriver;
use network_drivers::{exit_emulator, println};
use network_hil::network_mbox::NetworkMailbox;

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

    // Poll until the app transitions through Ready (after InitiateBoot
    // succeeds with DHCP + TFTP) and then returns to Idle (after Finalize).
    let max_polls: u32 = 50_000_000;
    let mut seen_ready = false;
    let mut last_state = app.state();
    for _ in 0..max_polls {
        driver.poll();
        network_app_boot_source::network::poll();

        let state = app.state();
        if state != last_state {
            println!("[boot-src] State changed: {:?} -> {:?}", last_state, state);
            last_state = state;
        }
        if state == AppState::Ready {
            seen_ready = true;
        }
        if seen_ready && state == AppState::Idle {
            println!("[boot-src] State returned to Idle — test PASSED!");
            exit_emulator(0x00);
        }
    }

    println!("[boot-src] ERROR: Timed out waiting for Finalize");
    exit_emulator(0x01);
}
