// Licensed under the Apache-2.0 license

//! NXP LPCIP3511 Device 0 initialization and EP0 enumeration.
//!
//! OCP Recovery class requests are claimed by dedicated hardware after USB
//! configuration, so this driver intentionally stops at enumeration instead
//! of implementing the software [`UsbDeviceDriver`] command path.

use caliptra_mcu_ocp::usb::descriptors::*;
use caliptra_mcu_ocp::usb::setup::{SetupPacket, StandardRequest, SETUP_PACKET_LEN};
use caliptra_mcu_registers_generated::{usb_combo, usb_dev0_mem};
use caliptra_mcu_romtime::StaticRef;
use tock_registers::interfaces::{Readable, Writeable};
use zerocopy::IntoBytes;

const MAX_TRANSFER_SIZE: u16 = 64;

const EP0_OUT_DESCRIPTOR: usize = 0;
const SETUP_DESCRIPTOR: usize = 1;
const EP0_IN_DESCRIPTOR: usize = 2;
const FIRST_GENERIC_DESCRIPTOR: usize = 4;
const DESCRIPTOR_COUNT: usize = 64;

const SETUP_BUFFER_OFFSET: usize = 0x100;
const OUT_BUFFER_OFFSET: usize = 0x140;
const IN_BUFFER_OFFSET: usize = 0x180;

const BUFFER_OFFSET_MASK: u32 = 0x7ff;
const NBYTES_SHIFT: u32 = 11;
const STALL: u32 = 1 << 29;
const DISABLED: u32 = 1 << 30;
const ACTIVE: u32 = 1 << 31;
const IMPLEMENTED_INTERRUPTS: u32 = 0xc000_ffff;
const DEFAULT_ULPI_POLL_LIMIT: u32 = 1_000_000;

const USB3320_VENDOR_ID: [u8; 2] = [0x24, 0x04];
const USB3320_PRODUCT_ID: [u8; 2] = [0x07, 0x00];
const USB3320_FUNCTION_CTRL: u8 = 0x04;
const USB3320_OTG_CTRL: u8 = 0x0a;
const USB3320_SCRATCH: u8 = 0x16;
const USB3320_SCRATCH_SET: u8 = 0x17;
const USB3320_SCRATCH_CLEAR: u8 = 0x18;
const USB3320_HS_DEVICE_FUNCTION_CTRL: u8 = 0x40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LpcipUsbError {
    Timeout,
    UnsupportedPhy,
    PhyIdentityMismatch,
    PhyScratchMismatch,
}

pub struct LpcipUsbDriver {
    regs: StaticRef<usb_combo::regs::UsbCombo>,
    memory: StaticRef<usb_dev0_mem::regs::UsbDev0Mem>,
    poll_limit: Option<u32>,
}

impl LpcipUsbDriver {
    pub const fn new(
        regs: StaticRef<usb_combo::regs::UsbCombo>,
        memory: StaticRef<usb_dev0_mem::regs::UsbDev0Mem>,
    ) -> Self {
        Self {
            regs,
            memory,
            poll_limit: None,
        }
    }

    pub const fn with_poll_limit(mut self, poll_limit: u32) -> Self {
        self.poll_limit = Some(poll_limit);
        self
    }

    /// Print the Device 0 controller registers and readable USB3320 registers.
    pub fn dump_registers(&self) {
        caliptra_mcu_romtime::println!("[usb] LPCIP Device 0 registers:");
        caliptra_mcu_romtime::println!(
            "[usb]   DEVCMDSTAT   = 0x{:08x}",
            self.regs.dev0_csr_devcmdstat.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   INFO         = 0x{:08x}",
            self.regs.dev0_csr_info.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   EPLISTSTART  = 0x{:08x}",
            self.regs.dev0_csr_epliststart.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   DATABUFSTART = 0x{:08x}",
            self.regs.dev0_csr_databufstart.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   LPM          = 0x{:08x}",
            self.regs.dev0_csr_lpm.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   EPSKIP       = 0x{:08x}",
            self.regs.dev0_csr_epskip.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   EPINUSE      = 0x{:08x}",
            self.regs.dev0_csr_epinuse.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   EPBUFCFG     = 0x{:08x}",
            self.regs.dev0_csr_epbufcfg.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   INTSTAT      = 0x{:08x}",
            self.regs.dev0_csr_intstat.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   INTEN        = 0x{:08x}",
            self.regs.dev0_csr_inten.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   INTSETSTAT   = 0x{:08x}",
            self.regs.dev0_csr_intsetstat.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   INTROUTE     = 0x{:08x}",
            self.regs.dev0_csr_introute.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   CONFIG       = 0x{:08x}",
            self.regs.dev0_csr_config.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   EPTOGGLE     = 0x{:08x}",
            self.regs.dev0_csr_eptoggle.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   CLKCTRL      = 0x{:08x}",
            self.regs.dev0_csr_clkctrl.get()
        );
        caliptra_mcu_romtime::println!(
            "[usb]   ULPIDEBUG    = 0x{:08x}",
            self.regs.dev0_csr_ulpidebug.get()
        );

        caliptra_mcu_romtime::println!("[usb] USB3320 registers:");
        for (name, address) in [
            ("VENDOR_ID_LOW  ", 0x00),
            ("VENDOR_ID_HIGH ", 0x01),
            ("PRODUCT_ID_LOW ", 0x02),
            ("PRODUCT_ID_HIGH", 0x03),
            ("FUNCTION_CTRL  ", 0x04),
            ("INTERFACE_CTRL ", 0x07),
            ("OTG_CTRL       ", 0x0a),
            ("USB_INT_EN_RISE", 0x0d),
            ("USB_INT_EN_FALL", 0x10),
            ("USB_INT_STATUS ", 0x13),
            ("USB_INT_LATCH  ", 0x14),
            ("DEBUG          ", 0x15),
            ("SCRATCH        ", 0x16),
        ] {
            match self.ulpi_read(address) {
                Ok(value) => caliptra_mcu_romtime::println!(
                    "[usb]   {} [0x{:02x}] = 0x{:02x}",
                    name,
                    address,
                    value
                ),
                Err(error) => {
                    caliptra_mcu_romtime::println!(
                        "[usb]   ULPI read failed at 0x{:02x}: {:?}",
                        address,
                        error
                    );
                    break;
                }
            }
        }
    }

    /// Initialize Device 0 and service standard EP0 requests until configured.
    ///
    /// The platform must enable and release reset for the external USB PHY and
    /// controller clock before calling this method.
    pub fn init_and_enumerate(&mut self) -> Result<(), LpcipUsbError> {
        caliptra_mcu_romtime::println!("[usb] Disconnecting and initializing Device 0");
        self.disconnect_and_initialize();
        caliptra_mcu_romtime::println!("[usb] Initializing USB3320 PHY");
        self.initialize_phy()?;
        caliptra_mcu_romtime::println!("[usb] Waiting for VBUS");
        self.wait_for_vbus()?;
        caliptra_mcu_romtime::println!("[usb] Enabling Device 0 interrupts");
        self.enable_interrupts();
        caliptra_mcu_romtime::println!("[usb] Connecting Device 0");
        self.connect();
        caliptra_mcu_romtime::println!("[usb] Waiting for USB bus reset");
        self.wait_for_bus_reset()?;
        caliptra_mcu_romtime::println!("[usb] Servicing EP0 enumeration requests");
        let mut configured = false;

        loop {
            self.wait_for_setup()?;
            let setup = self.read_setup();
            self.clear_setup_received();

            match setup.standard_request() {
                Some(StandardRequest::GetDescriptor) => {
                    self.send_descriptor(&setup)?;
                    if configured
                        && setup.descriptor_type() == Some(DescriptorType::String)
                        && setup.descriptor_index() == OCP_INTERFACE_STRING_INDEX
                    {
                        return Ok(());
                    }
                }
                Some(StandardRequest::SetAddress) => self.set_address(&setup)?,
                Some(StandardRequest::SetConfiguration) => {
                    if setup.w_value != [1, 0] {
                        self.stall_ep0();
                        continue;
                    }
                    self.send_zlp_in()?;
                    configured = true;
                }
                Some(StandardRequest::GetConfiguration) => {
                    self.send_control_read(&[0], setup.data_length() as usize)?;
                }
                Some(StandardRequest::GetStatus) => {
                    self.send_control_read(&[1, 0], setup.data_length() as usize)?;
                }
                _ => {
                    self.stall_ep0();
                }
            }
        }
    }

    fn initialize_phy(&self) -> Result<(), LpcipUsbError> {
        if !self
            .regs
            .dev0_csr_config
            .is_set(usb_combo::bits::ConfigT::Ulpi)
        {
            return Err(LpcipUsbError::UnsupportedPhy);
        }

        self.regs
            .dev0_csr_ulpidebug
            .write(usb_combo::bits::UlpidebugT::PhyMode::SET);

        let vendor_id = [self.ulpi_read(0x00)?, self.ulpi_read(0x01)?];
        let product_id = [self.ulpi_read(0x02)?, self.ulpi_read(0x03)?];
        if vendor_id != USB3320_VENDOR_ID || product_id != USB3320_PRODUCT_ID {
            return Err(LpcipUsbError::PhyIdentityMismatch);
        }

        self.verify_phy_scratch()?;
        self.ulpi_write(USB3320_FUNCTION_CTRL, USB3320_HS_DEVICE_FUNCTION_CTRL)?;
        self.ulpi_write(USB3320_OTG_CTRL, 0)?;
        Ok(())
    }

    fn verify_phy_scratch(&self) -> Result<(), LpcipUsbError> {
        self.ulpi_write(USB3320_SCRATCH, 0xa5)?;
        if self.ulpi_read(USB3320_SCRATCH)? != 0xa5 {
            return Err(LpcipUsbError::PhyScratchMismatch);
        }

        self.ulpi_write(USB3320_SCRATCH_SET, 0x0f)?;
        if self.ulpi_read(USB3320_SCRATCH)? != 0xaf {
            return Err(LpcipUsbError::PhyScratchMismatch);
        }

        self.ulpi_write(USB3320_SCRATCH_CLEAR, 0xf0)?;
        if self.ulpi_read(USB3320_SCRATCH)? != 0x0f {
            return Err(LpcipUsbError::PhyScratchMismatch);
        }
        Ok(())
    }

    fn ulpi_read(&self, address: u8) -> Result<u8, LpcipUsbError> {
        let transaction = Self::ulpi_transaction(address);
        self.regs.dev0_csr_ulpidebug.set(transaction);
        self.regs
            .dev0_csr_ulpidebug
            .set(transaction | usb_combo::bits::UlpidebugT::PhyAccess::SET.value);
        self.wait_for_ulpi_access()?;
        Ok(self
            .regs
            .dev0_csr_ulpidebug
            .read(usb_combo::bits::UlpidebugT::PhyRdata) as u8)
    }

    fn ulpi_write(&self, address: u8, value: u8) -> Result<(), LpcipUsbError> {
        let transaction = Self::ulpi_transaction(address)
            | usb_combo::bits::UlpidebugT::PhyWdata
                .val(u32::from(value))
                .value
            | usb_combo::bits::UlpidebugT::PhyRw::SET.value;
        self.regs.dev0_csr_ulpidebug.set(transaction);
        self.regs
            .dev0_csr_ulpidebug
            .set(transaction | usb_combo::bits::UlpidebugT::PhyAccess::SET.value);
        self.wait_for_ulpi_access()
    }

    fn ulpi_transaction(address: u8) -> u32 {
        usb_combo::bits::UlpidebugT::PhyMode::SET.value
            | usb_combo::bits::UlpidebugT::PhyAddr
                .val(u32::from(address & 0x0f))
                .value
            | usb_combo::bits::UlpidebugT::PhyAddrHigh
                .val(u32::from(address >> 4))
                .value
    }

    fn wait_for_ulpi_access(&self) -> Result<(), LpcipUsbError> {
        let poll_limit = self.poll_limit.unwrap_or(DEFAULT_ULPI_POLL_LIMIT);
        for _ in 0..poll_limit {
            if !self
                .regs
                .dev0_csr_ulpidebug
                .is_set(usb_combo::bits::UlpidebugT::PhyAccess)
            {
                return Ok(());
            }
        }
        Err(LpcipUsbError::Timeout)
    }

    fn disconnect_and_initialize(&self) {
        self.regs.dev0_csr_devcmdstat.set(0);
        self.regs.dev0_csr_inten.set(0);
        self.regs.dev0_csr_intstat.set(IMPLEMENTED_INTERRUPTS);
        self.regs.dev0_csr_epliststart.set(0);
        self.regs.dev0_csr_databufstart.set(0);

        for descriptor in 0..DESCRIPTOR_COUNT {
            self.write_memory_word(descriptor, 0);
        }
        self.write_memory_word(SETUP_DESCRIPTOR, Self::buffer_offset(SETUP_BUFFER_OFFSET));
        self.arm_out(SETUP_PACKET_LEN);
        for descriptor in FIRST_GENERIC_DESCRIPTOR..DESCRIPTOR_COUNT {
            self.write_memory_word(descriptor, DISABLED);
        }
    }

    fn wait_for_vbus(&self) -> Result<(), LpcipUsbError> {
        self.wait_until(|| {
            self.regs
                .dev0_csr_devcmdstat
                .is_set(usb_combo::bits::DevcmdstatT::VbusDebounced)
        })
    }

    fn enable_interrupts(&self) {
        self.regs.dev0_csr_inten.set(
            usb_combo::bits::IntenT::EpIntEn.val(0x3).value
                | usb_combo::bits::IntenT::DevIntEn::SET.value,
        );
        self.regs.dev0_csr_intstat.set(IMPLEMENTED_INTERRUPTS);
    }

    fn connect(&self) {
        self.regs.dev0_csr_devcmdstat.set(
            usb_combo::bits::DevcmdstatT::DevEn::SET.value
                | usb_combo::bits::DevcmdstatT::Dcon::SET.value,
        );
    }

    fn wait_for_bus_reset(&self) -> Result<(), LpcipUsbError> {
        self.wait_until(|| {
            self.regs
                .dev0_csr_devcmdstat
                .is_set(usb_combo::bits::DevcmdstatT::DresC)
        })?;
        self.regs.dev0_csr_devcmdstat.set(
            usb_combo::bits::DevcmdstatT::DevEn::SET.value
                | usb_combo::bits::DevcmdstatT::Dcon::SET.value
                | usb_combo::bits::DevcmdstatT::DresC::SET.value,
        );
        self.regs
            .dev0_csr_intstat
            .set(usb_combo::bits::IntstatT::DevInt::SET.value);
        self.arm_out(SETUP_PACKET_LEN);
        Ok(())
    }

    fn wait_for_setup(&self) -> Result<(), LpcipUsbError> {
        self.wait_until(|| {
            self.regs
                .dev0_csr_devcmdstat
                .is_set(usb_combo::bits::DevcmdstatT::Setup)
        })
    }

    fn clear_setup_received(&self) {
        let value =
            self.regs.dev0_csr_devcmdstat.get() | usb_combo::bits::DevcmdstatT::Setup::SET.value;
        self.regs.dev0_csr_devcmdstat.set(value);
        self.clear_endpoint_interrupt(EP0_OUT_DESCRIPTOR);
    }

    fn read_setup(&self) -> SetupPacket {
        let first = self.read_memory_word(SETUP_BUFFER_OFFSET / 4).to_le_bytes();
        let second = self
            .read_memory_word(SETUP_BUFFER_OFFSET / 4 + 1)
            .to_le_bytes();
        let raw: [u8; SETUP_PACKET_LEN] = [
            first[0], first[1], first[2], first[3], second[0], second[1], second[2], second[3],
        ];
        zerocopy::transmute!(raw)
    }

    fn send_descriptor(&self, setup: &SetupPacket) -> Result<(), LpcipUsbError> {
        match setup.descriptor_type() {
            Some(DescriptorType::Device) => {
                let descriptor = DeviceDescriptor::ocp(0x0200, 0x1209, 0x0001, 0x0100, 0, 0, 0);
                self.send_control_read(descriptor.as_bytes(), setup.data_length() as usize)
            }
            Some(DescriptorType::Configuration) => {
                let descriptor = ConfigurationTree::ocp(
                    BmAttributes::new(true, false),
                    MaxPower2mA(50),
                    None,
                    MAX_TRANSFER_SIZE,
                    MAX_TRANSFER_SIZE,
                );
                self.send_control_read(descriptor.as_bytes(), setup.data_length() as usize)
            }
            Some(DescriptorType::String) if setup.descriptor_index() == 0 => {
                let descriptor = StringDescriptorZero::ocp();
                self.send_control_read(descriptor.as_bytes(), setup.data_length() as usize)
            }
            Some(DescriptorType::String)
                if setup.descriptor_index() == OCP_INTERFACE_STRING_INDEX =>
            {
                let descriptor = OcpInterfaceStringDescriptor::ocp();
                self.send_control_read(descriptor.as_bytes(), setup.data_length() as usize)
            }
            _ => {
                self.stall_ep0();
                Ok(())
            }
        }
    }

    fn set_address(&self, setup: &SetupPacket) -> Result<(), LpcipUsbError> {
        let address = u32::from(setup.w_value[0] & 0x7f);
        let value =
            self.regs.dev0_csr_devcmdstat.get() & !usb_combo::bits::DevcmdstatT::DevAddr.mask;
        self.regs
            .dev0_csr_devcmdstat
            .set(value | usb_combo::bits::DevcmdstatT::DevAddr.val(address).value);
        self.send_zlp_in()
    }

    fn send_control_read(&self, data: &[u8], requested: usize) -> Result<(), LpcipUsbError> {
        let length = core::cmp::min(data.len(), requested);
        self.write_buffer(IN_BUFFER_OFFSET, &data[..length]);
        self.arm_in(length);
        self.wait_descriptor_inactive(EP0_IN_DESCRIPTOR)?;
        self.arm_out(0);
        self.wait_descriptor_inactive(EP0_OUT_DESCRIPTOR)
    }

    fn send_zlp_in(&self) -> Result<(), LpcipUsbError> {
        self.arm_in(0);
        self.wait_descriptor_inactive(EP0_IN_DESCRIPTOR)
    }

    fn arm_out(&self, length: usize) {
        self.write_memory_word(
            EP0_OUT_DESCRIPTOR,
            Self::buffer_descriptor(OUT_BUFFER_OFFSET, length) | ACTIVE,
        );
    }

    fn arm_in(&self, length: usize) {
        self.write_memory_word(
            EP0_IN_DESCRIPTOR,
            Self::buffer_descriptor(IN_BUFFER_OFFSET, length) | ACTIVE,
        );
    }

    fn stall_ep0(&self) {
        self.write_memory_word(
            EP0_OUT_DESCRIPTOR,
            Self::buffer_descriptor(OUT_BUFFER_OFFSET, 0) | STALL,
        );
        self.write_memory_word(
            EP0_IN_DESCRIPTOR,
            Self::buffer_descriptor(IN_BUFFER_OFFSET, 0) | STALL,
        );
    }

    fn wait_descriptor_inactive(&self, descriptor: usize) -> Result<(), LpcipUsbError> {
        self.wait_until(|| self.read_memory_word(descriptor) & ACTIVE == 0)?;
        self.clear_endpoint_interrupt(descriptor);
        Ok(())
    }

    fn clear_endpoint_interrupt(&self, descriptor: usize) {
        let interrupt = match descriptor {
            EP0_OUT_DESCRIPTOR => usb_combo::bits::IntstatT::Ep0out::SET.value,
            EP0_IN_DESCRIPTOR => usb_combo::bits::IntstatT::Ep0in::SET.value,
            _ => 0,
        };
        self.regs.dev0_csr_intstat.set(interrupt);
    }

    fn wait_until(&self, mut condition: impl FnMut() -> bool) -> Result<(), LpcipUsbError> {
        match self.poll_limit {
            Some(poll_limit) => {
                for _ in 0..poll_limit {
                    if condition() {
                        return Ok(());
                    }
                }
                Err(LpcipUsbError::Timeout)
            }
            None => loop {
                if condition() {
                    return Ok(());
                }
            },
        }
    }

    fn buffer_offset(offset: usize) -> u32 {
        ((offset >> 6) as u32) & BUFFER_OFFSET_MASK
    }

    fn buffer_descriptor(offset: usize, length: usize) -> u32 {
        Self::buffer_offset(offset) | ((length as u32) << NBYTES_SHIFT)
    }

    fn write_buffer(&self, offset: usize, data: &[u8]) {
        for (index, chunk) in data.chunks(4).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_memory_word(offset / 4 + index, u32::from_le_bytes(word));
        }
    }

    fn read_memory_word(&self, index: usize) -> u32 {
        self.memory.usb_dev0_mem[index].get()
    }

    fn write_memory_word(&self, index: usize, value: u32) {
        self.memory.usb_dev0_mem[index].set(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_offsets_are_encoded_in_64_byte_units() {
        assert_eq!(LpcipUsbDriver::buffer_offset(SETUP_BUFFER_OFFSET), 4);
        assert_eq!(LpcipUsbDriver::buffer_offset(OUT_BUFFER_OFFSET), 5);
        assert_eq!(LpcipUsbDriver::buffer_offset(IN_BUFFER_OFFSET), 6);
    }

    #[test]
    fn setup_descriptor_encodes_length_and_offset() {
        assert_eq!(
            LpcipUsbDriver::buffer_descriptor(OUT_BUFFER_OFFSET, SETUP_PACKET_LEN),
            (8 << NBYTES_SHIFT) | 5
        );
    }

    #[test]
    fn ulpi_transaction_encodes_full_register_address_and_mode() {
        assert_eq!(LpcipUsbDriver::ulpi_transaction(0x16), 0x8000_0016);
    }
}
