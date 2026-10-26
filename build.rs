use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        println!("cargo:rerun-if-changed=icon.ico");
        println!("cargo:rerun-if-changed=rusttracker.manifest");
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.set("FileDescription", "RustTracker Vulkan Visualizer");
        res.set("ProductName", "RustTracker");
        res.set("OriginalFilename", "rusttracker.exe");
        res.set("LegalCopyright", "GPL-3.0-or-later");
        res.set_manifest_file("rusttracker.manifest");
        if let Err(e) = res.compile() {
            panic!("Failed to compile Windows resource: {}", e);
        }
        // Because Cargo.toml defines both [lib] and [[bin]], `cargo:rustc-link-lib=static=resource`
        // emitted by `winres` attaches `libresource.a` to `librusttracker.rlib`. Since `resource.o`
        // contains only a `.rsrc` section and zero exported symbols, the linker discards it when
        // linking `rusttracker.exe` unless passed directly as a binary linker argument.
        let out_dir = env::var("OUT_DIR").expect("OUT_DIR must be set");
        let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
        if target_env == "gnu" {
            println!(
                "cargo:rustc-link-arg-bin=rusttracker={}/resource.o",
                out_dir
            );
        } else if target_env == "msvc" {
            println!(
                "cargo:rustc-link-arg-bin=rusttracker={}/resource.lib",
                out_dir
            );
        }
    } else if target_os == "android" {
        println!("cargo:rustc-link-lib=mediandk");
    }
}
