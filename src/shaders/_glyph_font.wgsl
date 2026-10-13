// =====================================================
// RustTracker Shared 3x5 Bitmap Font & SMPTE Channel Labels
// Used by visualizers that render channel labels on the GPU.
// =====================================================

const GLYPH_BITMAPS: array<u32, 19> = array<u32, 19>(
    31599u, // 0
    11415u, // 1
    29671u, // 2
    29647u, // 3
    23497u, // 4
    31183u, // 5
    31215u, // 6
    29330u, // 7
    31727u, // 8
    31695u, // 9
    18727u, // 10: L
    31733u, // 11: R
    31015u, // 12: C
    31183u, // 13: S
    31204u, // 14: F
    31207u, // 15: E
    29842u, // 16: T
    24429u, // 17: M
    23421u  // 18: W
);

fn glyph_bitmap(ch: u32) -> u32 {
    if (ch < 19u) {
        return GLYPH_BITMAPS[ch];
    }
    return 0u;
}

fn get_smpte_channel_label(spatial_idx: u32) -> vec4<u32> {
    // Returns (char0, char1, char2, num_chars)
    switch spatial_idx {
        case 0u  { return vec4<u32>(10u, 20u, 20u, 1u); } // L
        case 1u  { return vec4<u32>(11u, 20u, 20u, 1u); } // R
        case 2u  { return vec4<u32>(12u, 20u, 20u, 1u); } // C
        case 3u  { return vec4<u32>(10u, 14u, 15u, 3u); } // LFE
        case 4u  { return vec4<u32>(10u, 13u, 20u, 2u); } // LS
        case 5u  { return vec4<u32>(11u, 13u, 20u, 2u); } // RS
        case 6u  { return vec4<u32>(10u, 11u, 13u, 3u); } // LRS
        case 7u  { return vec4<u32>(11u, 11u, 13u, 3u); } // RRS
        case 8u  { return vec4<u32>(10u, 16u, 14u, 3u); } // LTF
        case 9u  { return vec4<u32>(11u, 16u, 14u, 3u); } // RTF
        case 10u { return vec4<u32>(10u, 16u, 11u, 3u); } // LTR
        case 11u { return vec4<u32>(11u, 16u, 11u, 3u); } // RTR
        case 12u { return vec4<u32>(10u, 18u, 20u, 2u); } // LW
        case 13u { return vec4<u32>(11u, 18u, 20u, 2u); } // RW
        case 14u { return vec4<u32>(10u, 16u, 17u, 3u); } // LTM
        default  { return vec4<u32>(11u, 16u, 17u, 3u); } // RTM
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
