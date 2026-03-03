# [RFC] Caliptra MCU Network Recovery Boot

## Abstract
The Caliptra subsystem network recovery boot is designed for systems that integrate the Caliptra subsystem and use flash as the primary boot source. When both flash partitions are corrupted and no local boot path remains viable, network recovery provides an automated fallback to restore the system to a bootable state — without requiring physical intervention that is costly and impractical in hyperscale data center environments.

This RFC proposes a lightweight network recovery boot mechanism that enables the Caliptra subsystem to download firmware images over the network. The MCU ROM fetches firmware through a generic BootSourceProvider trait that abstracts the underlying boot source — whether flash, network, or USB. For network recovery, a Network Boot Coprocessor is provided as a reference implementation of a network-based boot source provider. The Network Boot Coprocessor runs within a minimal ROM environment and **operates outside the native Caliptra subsystem boundary**. It acts as an intermediary between remote image servers and the Caliptra subsystem, handling network configuration, server discovery, and firmware image downloads for multiple firmware components (Caliptra FMC+RT, SoC Manifest, MCU Runtime, and SoC images) through a firmware ID-based mapping system.

The recovery boot proceeds in two stages:
- **Stage 1 (ROM):** The MCU ROM uses the `BootSourceProvider` to fetch early firmware (Caliptra FMC+RT, SoC Manifest, MCU RT) and streams them to the Caliptra Recovery Interface. The early firmware image authentication stays the same as the existing streaming boot model.
- **Stage 2 (Runtime):** The MCU Runtime fetches remaining SoC images, loads them to their designated addresses, and authorizes each image through Caliptra RT.

## Scope

### Components Affected

- **MCU ROM**: Drives recovery boot flow using a new generic `BootSourceProvider` trait that abstracts boot sources (network, flash, or custom) behind a unified messaging protocol. Includes optional flash write-back logic for staging and committing network-recovered images to flash.
- **MCU Runtime**: Handles Stage 2 boot — fetches SoC images, loads them to memory, and coordinates authorization with Caliptra RT. Also implements flash write-back staging and commit for remainder firmware images.
- **Network Boot Coprocessor (Network ROM)**: New component, operating outside the Caliptra subsystem boundary, that provides a reference implementation of the `BootSourceProvider` trait for network-based recovery. Contains a lightweight network stack (lwIP) with DHCP client, TFTP client, and firmware ID mapping from a TOC (Table of Contents) configuration file.

### Components Not Affected

- **Caliptra Core**: No changes to Caliptra ROM or RT firmware. The recovery interface and authorization flows are used as-is.
- **Flash Layout / TOC Format**: The TOC follows the existing flash layout specification — no format changes.
- **Existing Flash Boot Path**: The normal flash-based boot path is unchanged. Network recovery is a fallback invoked only when flash boot fails.

### Protocol Stack (Network Boot Coprocessor)

The following protocols are implemented within the Network Boot Coprocessor, which operates outside the Caliptra subsystem boundary:

- **DHCP (DHCPv4 / DHCPv6)**: Automatic network configuration — IP address, gateway, and boot server discovery. DHCPv4 Options 66/67 and DHCPv6 Options 59/60 are used for TFTP server and bootfile discovery.
- **TFTP**: Lightweight UDP-based file transfer for downloading the TOC configuration file and firmware images. Minimal code footprint (~5–10 KB) suitable for ROM environments.
- **IPv4 / IPv6 Dual-Stack**: Full dual-stack support throughout discovery and transfer. Prefer IPv6 when available; fall back to IPv4.

## Rationale

- **Flash Dependency Risk**: The current architecture has no automated recovery if both flash partitions are corrupted. This is a single point of failure for boot.
- **Hyperscale Recovery Cost**: Physical intervention (e.g., re-flashing via JTAG or board swap) is prohibitively expensive in large-scale deployments. An in-band network recovery path eliminates this cost.
- **Minimal ROM Footprint**: The design uses a dedicated coprocessor with a lightweight network stack (lwIP) rather than adding network capability to the MCU ROM itself, keeping the ROM small and auditable.
- **Consistency with OCP Streaming Boot**: The approach aligns with the OCP streaming boot model for early firmware delivery, using standard protocols (DHCP + TFTP) that are well-understood in data center infrastructure.
- **Generic Boot Source Abstraction**: The `BootSourceProvider` trait decouples the MCU ROM from the specific boot source implementation, enabling future boot sources (e.g., USB, BMC-assisted) without ROM changes.
- **Flash Write-Back Reduces Re-Recovery**: Without flash write-back, every reboot after a network recovery would require another network recovery. The optional staging-and-commit flow restores the flash to a bootable state, making the recovery self-healing.

## Design Overview

For the full design details — including boot flow diagrams, messaging protocol packet formats, `BootSourceProvider` trait definition, flash write-back configuration, and network stack implementation — see the [Network Recovery Boot Design Document](https://github.com/chipsalliance/caliptra-mcu-sw/blob/main/docs/src/network_boot.md).

## Maintenance

- **Unit Tests**: Tests for the `BootSourceProvider` trait implementation, message serialization/deserialization, firmware ID mapping, and TOC parsing.
- **Integration Tests**: End-to-end network recovery boot tests using an emulated network environment with DHCP and TFTP servers, covering:
  - Successful Stage 1 and Stage 2 recovery boot.
  - Flash write-back with both commit policies.
  - Error cases: DHCP timeout, TFTP failure, checksum mismatch, image not found, staging write failure.
  - IPv4-only, IPv6-only, and dual-stack configurations.
- **Emulator Support**: The existing emulator infrastructure will be extended with network boot coprocessor emulation for CI/CD testing.
- **Error Handling Verification**: Tests for all defined error codes (0x00–0xFF), ensuring graceful degradation — flash write-back failures must not block the primary recovery path.
