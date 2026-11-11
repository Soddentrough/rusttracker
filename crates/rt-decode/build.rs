use std::env;
use std::path::PathBuf;
use std::process::Command;

fn build_libmpeghe() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let root_dir = manifest_dir.join("../..");
    let encoder_dir = root_dir.join("third_party/libmpeghe/encoder");
    println!("cargo:rerun-if-changed={}", encoder_dir.display());
    println!(
        "cargo:rerun-if-changed={}",
        root_dir.join("third_party/libmpeghe/LICENSE").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root_dir.join("third_party/libmpeghe/LICENSE2").display()
    );

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
        .warnings(true)
        .define("_LIB", None);

    if target_env == "msvc" {
        build
            .define("__USE_40__", None)
            .define("_CRT_SECURE_NO_WARNINGS", None)
            .define("WIN32", None)
            .define("_WINDOWS", None);
    } else {
        // Keep full -Wall -Wextra diagnostics active for real C bugs (uninitialized, prototypes,
        // bounds, aliasing, pointer types) while quieting upstream Ittiam C style noise.
        build
            .flag_if_supported("-Wno-unused-parameter")
            .flag_if_supported("-Wno-unused-but-set-variable")
            .flag_if_supported("-Wno-sign-compare");
        if target_arch == "aarch64" || target_arch == "arm" {
            build.define("_ARM_", None);
        } else {
            build.define("_X86_", None);
        }
    }

    build.compile("mpeghe");
}

fn link_libopenmpt() {
    for env_key in [
        "OPENMPT_DIR",
        "PKG_CONFIG",
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "PKG_CONFIG_SYSROOT_DIR",
        "PKG_CONFIG_ALLOW_CROSS",
        "LIBRARY_PATH",
    ] {
        println!("cargo:rerun-if-env-changed={env_key}");
    }

    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    let target_underscored = target.replace('-', "_");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_{target_underscored}");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH_{target_underscored}");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR_{target_underscored}");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "android" {
        let manifest_dir =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
        let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let abi_dir = match target_arch.as_str() {
            "aarch64" => "arm64-v8a",
            "x86_64" => "x86_64",
            "arm" => "armeabi-v7a",
            _ => "arm64-v8a",
        };
        let android_lib_dir = manifest_dir.join("../../local_lib/android").join(abi_dir);
        println!(
            "cargo:rustc-link-search=native={}",
            android_lib_dir.display()
        );
        println!("cargo:rustc-link-lib=static=openmpt");
        return;
    }

    if let Ok(openmpt_dir) = env::var("OPENMPT_DIR")
        && !openmpt_dir.trim().is_empty()
    {
        let root = PathBuf::from(openmpt_dir);
        let lib_dir = root.join("lib");
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
        } else {
            println!("cargo:rustc-link-search=native={}", root.display());
        }
        println!("cargo:rustc-link-lib=openmpt");
        return;
    }

    let target_pkg_config = env::var(format!("PKG_CONFIG_{target_underscored}")).ok();
    let can_use_pkg_config = host == target
        || target_pkg_config.is_some()
        || env::var("PKG_CONFIG_ALLOW_CROSS").as_deref() == Ok("1")
        || env::var_os(format!("PKG_CONFIG_PATH_{target_underscored}")).is_some()
        || env::var_os(format!("PKG_CONFIG_LIBDIR_{target_underscored}")).is_some()
        || env::var_os("PKG_CONFIG_LIBDIR").is_some()
        || env::var_os("PKG_CONFIG_SYSROOT_DIR").is_some();

    let mut linked = false;
    if can_use_pkg_config {
        let pkg_config_bin = target_pkg_config
            .or_else(|| env::var("PKG_CONFIG").ok())
            .unwrap_or_else(|| "pkg-config".to_string());
        if let Ok(output) = Command::new(pkg_config_bin)
            .args(["--libs", "libopenmpt"])
            .output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for token in stdout.split_whitespace() {
                if let Some(dir) = token.strip_prefix("-L") {
                    println!("cargo:rustc-link-search=native={dir}");
                } else if let Some(lib) = token.strip_prefix("-l") {
                    println!("cargo:rustc-link-lib={lib}");
                    linked = true;
                }
            }
        }
    }

    if !linked {
        if target_os == "macos" {
            for candidate in ["/opt/homebrew/lib", "/usr/local/lib"] {
                if std::path::Path::new(candidate).exists() {
                    println!("cargo:rustc-link-search=native={candidate}");
                }
            }
        }
        println!("cargo:rustc-link-lib=openmpt");
    }
}

fn main() {
    build_libmpeghe();
    link_libopenmpt();
}
