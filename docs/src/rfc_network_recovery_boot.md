# [RFC] Caliptra MCU Network Recovery Boot

## Abstract

When both flash partitions on a Caliptra subsystem are corrupted, the system currently has no automated recovery path — physical intervention is required, which is costly and impractical in hyperscale data center environments. This RFC proposes a lightweight network recovery boot mechanism that enables the Caliptra subsystem to download firmware images over the network, providing a resilient fallback when flash-based boot fails.

A dedicated Network Boot Coprocessor, running within a minimal ROM environment, acts as an intermediary between remote image servers and the Caliptra subsystem. It handles network communications including DHCP configuration, TFTP server discovery, and firmware image downloads. The system supports downloading multiple firmware components — Caliptra FMC+RT, SoC Manifest, MCU Runtime, and SoC images — through a firmware ID-based mapping system and a generic `BootSourceProvider` interface that abstracts the boot source from the MCU ROM.

The recovery boot proceeds in two stages:
- **Stage 1 (ROM):** The MCU ROM uses the `BootSourceProvider` to fetch early firmware (Caliptra FMC+RT, SoC Manifest, MCU RT) and streams them to the Caliptra Recovery Interface.
- **Stage 2 (Runtime):** The MCU Runtime fetches remaining SoC images, loads them to their designated addresses, and authorizes each image through Caliptra RT.

An optional **Flash Write-Back** feature allows downloaded images to be dual-written to a staging buffer during streaming and later committed to the active flash partition, so subsequent reboots can proceed from flash without requiring another network recovery.

## Scope

### Components Affected

- **MCU ROM**: Drives recovery boot flow using the `BootSourceProvider` interface to fetch and stream early firmware to the Caliptra Recovery Interface.
- **MCU Runtime**: Handles Stage 2 boot — fetches SoC images, loads them to memory, and coordinates authorization with Caliptra RT.
- **Network Boot Coprocessor (Network ROM)**: New component implementing the `BootSourceProvider` trait. Contains a lightweight network stack (lwIP) with DHCP client, TFTP client, and firmware ID mapping from a TOC (Table of Contents) configuration file.
- **Boot Source Provider Interface**: New generic trait (`BootSourceProvider`) that abstracts boot sources (network, flash, or custom) behind a unified messaging protocol with defined message types (Initiate Boot, Get Image Metadata, Image Download, Chunk ACK, Finalize).
- **Flash Write-Back (optional)**: Staging and commit logic for persisting network-recovered images to flash. Controlled via `BootFlags` (FlashWriteBack enable, FlashCommitPolicy).

### Components Not Affected

- **Caliptra Core**: No changes to Caliptra ROM or RT firmware. The recovery interface and authorization flows are used as-is.
- **Flash Layout / TOC Format**: The TOC follows the existing flash layout specification — no format changes.
- **Existing Flash Boot Path**: The normal flash-based boot path is unchanged. Network recovery is a fallback invoked only when flash boot fails.

### Protocol Stack

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
