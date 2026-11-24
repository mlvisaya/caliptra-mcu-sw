// Licensed under the Apache-2.0 license

fn main() {
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"runtime\"))");
    caliptra_mcu_hw_revision::HardwareRevision::from_env()
        .unwrap_or_else(|error| panic!("{error}"))
        .emit_build_configuration();
    if std::env::var_os("CARGO_FEATURE_CORE").is_some()
        && std::env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "riscv32"
    {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../core/caliptra-sw/test-harness/scripts/rom.ld");
        println!("cargo:rerun-if-changed={}", script.display());
        println!(
            "cargo:rustc-link-arg-bin=caliptra-mcu-test-fw-mbox-ownership-core=-T{}",
            script.display()
        );
    }
}
