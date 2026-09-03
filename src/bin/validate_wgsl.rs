use std::path::Path;

fn main() {
    let common = std::fs::read_to_string("src/shaders/_common.wgsl")
        .expect("Failed to read _common.wgsl");
    let glyph_font = std::fs::read_to_string("src/shaders/_glyph_font.wgsl")
        .expect("Failed to read _glyph_font.wgsl");

    let shader_dir = Path::new("src/shaders");
    let entries = std::fs::read_dir(shader_dir).expect("Failed to read src/shaders directory");

    let mut failed = false;
    let mut validated_count = 0;

    for entry in entries {
        let entry = entry.expect("Failed to read entry");
        let path = entry.path();
        if path.is_file()
            && let Some(ext) = path.extension()
            && ext == "wgsl" {
            let filename = path.file_name().unwrap().to_string_lossy().into_owned();
                    // Skip the helper header files themselves as they are incomplete standalone WGSL
                    if filename.starts_with('_') {
                        continue;
                    }

                    let source = std::fs::read_to_string(&path)
                        .unwrap_or_else(|_| panic!("Failed to read shader {:?}", path));
                    
                    let full_source = source
                        .replace("// INCLUDE: common", &common)
                        .replace("// INCLUDE: glyph_font", &glyph_font);

                    let mut frontend = naga::front::wgsl::Frontend::new();
                    match frontend.parse(&full_source) {
                        Ok(module) => {
                            let mut validator = naga::valid::Validator::new(
                                naga::valid::ValidationFlags::all(),
                                naga::valid::Capabilities::all(),
                            );
                            match validator.validate(&module) {
                                Ok(_) => {
                                    println!("Successfully validated: {}", filename);
                                    validated_count += 1;
                                }
                                Err(e) => {
                                    eprintln!("Validation error in {}: {:?}", filename, e);
                                    failed = true;
                                }
                            }
                        }
                        Err(e) => {
                            let err_str = e.emit_to_string_with_path(&full_source, &filename);
                            eprintln!("Parse error in {}:\n{}", filename, err_str);
                            failed = true;
                        }
                    }
        }
    }

    if failed {
        eprintln!("\nSome shaders failed validation.");
        std::process::exit(1);
    } else {
        println!("\nAll {} shaders validated successfully!", validated_count);
    }
}
