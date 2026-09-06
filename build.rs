use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.set("FileDescription", "RustTracker Vulkan Visualizer");
        res.set("ProductName", "RustTracker");
        res.set("OriginalFilename", "rusttracker.exe");
        res.set("LegalCopyright", "GPL-3.0-or-later");
        res.set_manifest_file("rusttracker.manifest");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Failed to compile Windows resource: {}", e);
        }
    } else if target_os == "android" {
        println!("cargo:rustc-link-lib=mediandk");
    }
}
