use std::env;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Failed to compile Windows resource: {}", e);
        }
    }
}
