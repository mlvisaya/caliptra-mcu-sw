# USB PHY Emulation and ROM Alignment Plan

## Purpose

This document records the plan to align the MCU ROM USB initialization and emulator behavior with the current Caliptra SS USB architecture and the external Microchip USB3320 ULPI PHY used by the Janus FPGA platform.

The plan builds on the existing [USB Recovery Emulation Design](./usb_recovery_emulation.md). It preserves USB/IP as the host-facing transport while adding the software-visible behavior of an external ULPI PHY and correcting the Device 0 OCP Recovery path.

## Current architecture

The physical path is:

```text
Recovery Agent
  -> host USB stack and controller
  -> USB cable
  -> USB3320 PHY
  -> ULPI
  -> Caliptra SS compound USB controller
  -> Device 0 EP0 classifier
       -> standard request: legacy DMA and MCU ROM
       -> OCP Recovery request: hardware recovery registers and FIFO
```

The USB3320 handles the electrical USB interface and translates it to ULPI. It does not process URBs, descriptors, or OCP commands. URBs exist in the host software stack; the Caliptra SS controller receives USB packets produced from those URBs.

The compound controller contains Device 0, Device 1, hub support, packet memories, and hardware-assisted OCP Recovery. OCP Recovery is claimable only on Device 0 EP0. The current top-level disables hub mode, so Device 0 is the upstream device.

## Current gaps

### Emulator

The Linux integration path already exercises:

```text
libusb -> Linux USB core -> VHCI -> USB/IP -> emulator
```

Below the USB/IP adapter, however, the model is transaction-level:

- standard EP0 requests are translated directly into LPCIP descriptor and register events;
- OCP requests are offered directly to `UsbRecoveryHost` before reaching the LPCIP Device 0 path;
- `ULPIDEBUG` has no USB3320 transaction behavior;
- no PHY identity, scratch register, Function Control, OTG Control, or ULPI access completion is modeled; and
- the mirrored-SETUP classifier and ownership transition are not represented as one unified path.

Consequently, the current end-to-end test validates the recovery agent, USB/IP, ROM enumeration, and recovery data flow, but it does not validate the ROM's external-PHY initialization or the hardware's Device 0 EP0 claim boundary.

### ROM

The LPCIP ROM driver initializes Device 0 and enumerates the OCP interface, but it assumes that the platform has already enabled and released reset for the external PHY and controller clock. It does not currently:

- verify that the controller reports ULPI support;
- select ULPI mode through `ULPIDEBUG`;
- verify the USB3320 vendor and product IDs;
- configure USB3320 Function Control and OTG Control;
- run the USB3320 Scratch-register test; or
- explicitly represent the platform PHY-reset and clock contract.

The existing Device 0 SRAM and register initialization must remain based on the current Caliptra SS programming model. Old Janus addresses and FPGA-specific register interpretations must not be copied into ROM.

## Design decision

The emulator will implement a behavioral USB3320 and ULPI register gateway, not pin-cycle ULPI or analog USB signaling.

The behavioral model will validate the same firmware-visible contract used on FPGA:

- `ULPIDEBUG.PHY_MODE` selection;
- ULPI register reads and writes;
- `PHY_ACCESS` completion;
- USB3320 identification registers;
- Function Control, OTG Control, and Scratch behavior;
- VBUS presence and selected operating speed; and
- controller attach only after PHY and Device 0 initialization.

The model will continue to translate USB/IP control URBs into transaction-level USB operations. It will not model the ULPI 60 MHz waveform, D+/D- electrical behavior, PID encoding, CRC generation, bit stuffing, chirp timing, signal integrity, or compliance behavior.

## Ownership split

### Platform integration

Platform code owns operations that are outside the USB controller register contract:

- routing ULPI pins;
- asserting and releasing USB3320 `RESETB`;
- providing or accepting the 60 MHz ULPI clock;
- releasing controller reset and clock gating; and
- reporting that the PHY is ready before ROM uses the controller.

The emulator represents this contract through initial PHY/reset/clock state rather than board GPIOs.

### MCU ROM

ROM owns software-visible USB initialization:

1. Confirm that Device 0 reports ULPI support.
2. Select the ULPI interface through `ULPIDEBUG.PHY_MODE`.
3. Read and verify the USB3320 vendor and product IDs.
4. Run the USB3320 Scratch write, set, clear, and readback test.
5. Configure Function Control for high-speed peripheral operation.
6. Configure OTG Control for device mode without host pull-downs or VBUS drive.
7. Keep Device 0 disconnected while clearing controller and interrupt state.
8. Initialize the endpoint command/status list and EP0 buffers in Device 0 packet SRAM.
9. Program `EPLISTSTART` and `DATABUFSTART` according to the current local packet-memory contract.
10. Enable only the required Device 0 interrupts.
11. Enable the controller, preserve the required disconnect interval, and then assert `DCON`.
12. Service standard enumeration until configuration is complete.
13. Leave claimed OCP DATA and STATUS stages to the hardware recovery path.

The ROM implementation should use the generated Caliptra SS register fields. It must not copy the old Janus `0x4940_0000` controller address, `0x2400_2000` DMA address, host `PORTMODE` dependency, or FPGA-specific `PLLON` aliases.

### OCP recovery hardware model

The emulator's Device 0 path will mirror the RTL ownership rules:

1. Every Device 0 EP0 SETUP reaches the legacy LPCIP/DMA model.
2. The same completed eight-byte SETUP is classified for OCP Recovery.
3. A valid enabled OCP request claims subsequent DATA, STATUS, and PING stages.
4. Claimed requests use the recovery register or FIFO model without per-command MCU handling.
5. Standard and non-OCP requests remain owned by ROM.
6. Bus reset, Device 0 reset/disconnect, replacement SETUP, or explicit claim abort releases recovery ownership as defined by the RTL contract.

## Implementation phases

### Phase 1: USB3320 behavioral model

Status: complete as of 2026-09-25.

- Add a USB3320 register file and reset defaults to the emulator peripheral.
- Implement ULPI read/write transactions initiated through Device 0 `ULPIDEBUG`.
- Clear `PHY_ACCESS` when each modeled transaction completes.
- Implement vendor ID, product ID, Function Control, OTG Control, Scratch, SET, and CLEAR aliases.
- Add focused tests for identification, read/write completion, Scratch behavior, reset state, and invalid accesses.

The implementation is in `emulator/periph/src/usb.rs`. It models synchronous
gateway completion, fixed USB3320 identification, writable control registers,
read-only identification registers, Function Control soft reset, and the ULPI
write/set/clear aliases used by the NXP initialization sequence. The complete
`caliptra-mcu-emulator-periph` test suite validates this phase.

### Phase 2: ROM PHY initialization

- Add a small no-allocation USB3320 driver around the generated `ULPIDEBUG` register.
- Add bounded polling and explicit errors for timeout, identity mismatch, and Scratch failure.
- Add a platform-ready contract for external reset and clock setup.
- Reorder controller initialization so attach occurs only after PHY, packet memory, EP0, and interrupt state are valid.
- Preserve the current OCP descriptors and standard enumeration behavior.

### Phase 3: Unified Device 0 and OCP path

- Route every USB/IP EP0 SETUP through the LPCIP Device 0 model first.
- Move OCP classification into the modeled post-SETUP arbitration point.
- Replace the USB/IP adapter's direct pre-controller `UsbRecoveryHost` dispatch.
- Model claimed DATA and STATUS ownership, STALL, replacement SETUP, reset, and abort behavior.
- Keep the recovery register/FIFO implementation shared with Caliptra event handling.

### Phase 4: End-to-end and FPGA parity

- Run existing LPCIP enumeration and recovery unit tests.
- Run the Linux `libusb` over USB/IP streaming-boot test.
- Add negative tests for PHY identity failure, ULPI timeout, Scratch failure, disabled OCP path, malformed OCP SETUP, reset during a claim, and non-OCP class requests.
- Compare ROM register traces against the NXP bring-up sequence and current Caliptra SS programmer's guide.
- Run the same recovery-agent image sequence against FPGA hardware through the USB3320 daughtercard.

## Acceptance criteria

The work is complete when:

1. ROM refuses to attach Device 0 if external-PHY initialization fails.
2. The emulator executes and validates the same USB3320 register sequence as FPGA firmware.
3. Standard enumeration still traverses Device 0 and ROM.
4. Valid OCP Recovery requests traverse Device 0 SETUP handling and are then claimed by the hardware recovery model.
5. Non-OCP requests never enter the recovery model.
6. Existing USB/IP and direct emulator tests pass without a host-agent API change.
7. The production-style `libusb` recovery agent completes streaming boot against both the emulator and FPGA.
8. Remaining differences are limited to explicitly excluded PHY electrical and timing behavior.

## References

- [USB Recovery Emulation Design](./usb_recovery_emulation.md)
- Caliptra SS USB2 Programmer's Guide: `hw/caliptra-ss/third_party/usb2/docs/USB2_Programmers_Guide.md`
- Caliptra SS OCP Recovery microarchitecture: `hw/caliptra-ss/docs/usb2_recovery_spec/README.md`
- Current ROM LPCIP driver: `platforms/emulator/rom/usb/src/lpcip.rs`
- Current emulator USB compound peripheral: `emulator/periph/src/usb.rs`
- USB/IP adapter: `tests/integration/src/usb/hwmodel.rs`
- NXP USB3320 reference initialization: `janus_fpga_sw/janus_fpga_sw/middleware/usb/phy/USB3320_phy.c`
