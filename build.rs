use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Failed to compile Windows resource: {}", e);
        }
    } else if target_os == "android" {
        println!("cargo:rustc-link-lib=mediandk");
    }
}
