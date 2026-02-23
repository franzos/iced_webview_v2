fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.lock");

    // When the cef feature is enabled, add the CEF distribution directory
    // to the binary's RPATH so libcef.so can be found at runtime without
    // requiring LD_LIBRARY_PATH.
    if std::env::var("CARGO_FEATURE_CEF").is_ok() {
        if let Ok(out_dir) = std::env::var("OUT_DIR") {
            let build_dir = std::path::Path::new(&out_dir)
                .ancestors()
                .find(|p| p.file_name().map_or(false, |n| n == "build"));

            if let Some(build_dir) = build_dir {
                if let Ok(entries) = std::fs::read_dir(build_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        if name.to_string_lossy().starts_with("cef-dll-sys-") {
                            let cef_dir = entry.path().join("out").join(format!(
                                "cef_{}_{}",
                                std::env::consts::OS,
                                std::env::consts::ARCH
                            ));
                            if cef_dir.exists() {
                                println!("cargo:rustc-link-arg=-Wl,-rpath,{}", cef_dir.display());
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}
