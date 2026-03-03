// Licensed under the Apache-2.0 license

use emulator_registers_generated::network_mbox::NetworkMboxPeripheral;

pub trait NetworkManager {
    /// Returns a mutable reference to the network mailbox peripheral.
    fn mbox(&mut self) -> &mut dyn NetworkMboxPeripheral;
}
