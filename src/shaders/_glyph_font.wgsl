// =====================================================
// RustTracker Shared 3x5 Bitmap Font
// Used by visualizers that render channel labels on the GPU.
// =====================================================

fn glyph_bitmap(ch: u32) -> u32 {
    // 3x5 pixel font. 15 bits per glyph, MSB = top-left.
    // Index: 0-9=digits, 10=L, 11=R, 12=C, 13=S, 14=F, 15=E, 16=T, 17=M, 18=W
    switch ch {
        case 0u  { return 31599u; } // 0
        case 1u  { return 11415u; } // 1
        case 2u  { return 29671u; } // 2
        case 3u  { return 29647u; } // 3
        case 4u  { return 23497u; } // 4
        case 5u  { return 31183u; } // 5
        case 6u  { return 31215u; } // 6
        case 7u  { return 29330u; } // 7
        case 8u  { return 31727u; } // 8
        case 9u  { return 31695u; } // 9
        case 10u { return 18727u; } // L: #.. #.. #.. #.. ###
        case 11u { return 31733u; } // R: ### #.# ### ##. #.#
        case 12u { return 31015u; } // C: ### #.. #.. #.. ###
        case 13u { return 31183u; } // S (same as 5)
        case 14u { return 31204u; } // F: ### #.. ### #.. #..
        case 15u { return 31207u; } // E: ### #.. ### #.. ###
        case 16u { return 29842u; } // T: ### .#. .#. .#. .#.
        case 17u { return 24429u; } // M: #.# ### #.# #.# #.#
        case 18u { return 23421u; } // W: #.# #.# #.# ### #.#
        default  { return 0u; }     // space
    }
}

fn draw_label_char(ch: u32, frag: vec2<f32>, origin: vec2<f32>, px: f32) -> f32 {
    let local = frag - origin;
    if local.x < 0.0 || local.x >= px * 3.0 || local.y < 0.0 || local.y >= px * 5.0 { return 0.0; }
    let col = u32(floor(local.x / px));
    let row = u32(floor(local.y / px));
    let bit = (4u - row) * 3u + (2u - col);
    return f32((glyph_bitmap(ch) >> bit) & 1u);
}
