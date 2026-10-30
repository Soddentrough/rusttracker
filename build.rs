use std::env;
use std::path::PathBuf;

fn build_libmpeghe() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let encoder_dir = manifest_dir.join("third_party/libmpeghe/encoder");
    println!("cargo:rerun-if-changed={}", encoder_dir.display());

    let mut c_files: Vec<PathBuf> = std::fs::read_dir(&encoder_dir)
        .expect("Failed to read third_party/libmpeghe/encoder directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("c"))
        .collect();
    c_files.sort();

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .include(&encoder_dir)
        .files(&c_files)
        .opt_level(3)
        .warnings(false)
        .define("_LIB", None);

    if target_env == "msvc" {
        build
            .define("__USE_40__", None)
            .define("_CRT_SECURE_NO_WARNINGS", None)
            .define("WIN32", None)
            .define("_WINDOWS", None);
    } else {
        build.flag_if_supported("-w");
        if target_arch == "aarch64" || target_arch == "arm" {
            build.define("_ARM_", None);
        } else {
            build.define("_X86_", None);
        }
    }

    build.compile("mpeghe");
}

fn main() {
    build_libmpeghe();
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
            println!("cargo:rustc-link-arg-cdylib=-Wl,--exclude-all-symbols");
        } else if target_env == "msvc" {
            println!(
                "cargo:rustc-link-arg-bin=rusttracker={}/resource.lib",
                out_dir
            );
        }
    } else if target_os == "android" {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
        let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let abi_dir = match target_arch.as_str() {
            "aarch64" => "arm64-v8a",
            "x86_64" => "x86_64",
            "arm" => "armeabi-v7a",
            _ => "arm64-v8a",
        };
        println!(
            "cargo:rustc-link-search=native={}/local_lib/android/{}",
            manifest_dir, abi_dir
        );
        println!("cargo:rustc-link-lib=static=openmpt");
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=mediandk");
        println!("cargo:rustc-link-arg=-lopenmpt");
        println!("cargo:rustc-link-arg=-lz");
        println!("cargo:rustc-link-arg=-lc++_static");
        println!("cargo:rustc-link-arg=-lc++abi");
    }
}
