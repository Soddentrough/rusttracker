// EarCut Triangulator with proper coincidence filtering for hole bridging

use std::path::Path;

struct GlyphContour {
    points: Vec<[f32; 2]>,
}

struct ContourBuilder {
    contours: Vec<GlyphContour>,
    current_contour: Vec<[f32; 2]>,
    start_point: [f32; 2],
    last_point: [f32; 2],
}

impl ContourBuilder {
    fn new() -> Self {
        Self {
            contours: Vec::new(),
            current_contour: Vec::new(),
            start_point: [0.0, 0.0],
            last_point: [0.0, 0.0],
        }
    }

    fn finish(mut self) -> Vec<GlyphContour> {
        if !self.current_contour.is_empty() {
            self.contours.push(GlyphContour { points: self.current_contour });
        }
        self.contours
    }
}

impl ttf_parser::OutlineBuilder for ContourBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        if !self.current_contour.is_empty() {
            self.contours.push(GlyphContour { points: std::mem::take(&mut self.current_contour) });
        }
        self.start_point = [x, y];
        self.last_point = [x, y];
        self.current_contour.push([x, y]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.last_point = [x, y];
        self.current_contour.push([x, y]);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last_point;
        let p1 = [x1, y1];
        let p2 = [x, y];
        let steps = 4;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let it = 1.0 - t;
            let qx = it * it * p0[0] + 2.0 * it * t * p1[0] + t * t * p2[0];
            let qy = it * it * p0[1] + 2.0 * it * t * p1[1] + t * t * p2[1];
            self.current_contour.push([qx, qy]);
        }
        self.last_point = [x, y];
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last_point;
        let p1 = [x1, y1];
        let p2 = [x2, y2];
        let p3 = [x, y];
        let steps = 6;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let it = 1.0 - t;
            let cx = it * it * it * p0[0] + 3.0 * it * it * t * p1[0] + 3.0 * it * t * t * p2[0] + t * t * t * p3[0];
            let cy = it * it * it * p0[1] + 3.0 * it * it * t * p1[1] + 3.0 * it * t * t * p2[1] + t * t * t * p3[1];
            self.current_contour.push([cx, cy]);
        }
        self.last_point = [x, y];
    }

    fn close(&mut self) {
        if let Some(last) = self.current_contour.last() {
            let dx = last[0] - self.start_point[0];
            let dy = last[1] - self.start_point[1];
            if (dx * dx + dy * dy).sqrt() < 0.001 && self.current_contour.len() > 1 {
                self.current_contour.pop();
            }
        }
        if !self.current_contour.is_empty() {
            self.contours.push(GlyphContour { points: std::mem::take(&mut self.current_contour) });
        }
    }
}

fn signed_area(pts: &[[f32; 2]]) -> f32 {
    let n = pts.len();
    if n < 3 { return 0.0; }
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += pts[i][0] * pts[j][1] - pts[j][0] * pts[i][1];
    }
    area * 0.5
}

fn point_in_polygon(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let pi = poly[i];
        let pj = poly[j];
        if (pi[1] > p[1]) != (pj[1] > p[1]) {
            let x_int = pi[0] + (p[1] - pi[1]) * (pj[0] - pi[0]) / (pj[1] - pi[1]);
            if p[0] < x_int {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_strictly_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    // Check if p is coincident with any vertex
    if (p[0] - a[0]).abs() < 1e-4 && (p[1] - a[1]).abs() < 1e-4 { return false; }
    if (p[0] - b[0]).abs() < 1e-4 && (p[1] - b[1]).abs() < 1e-4 { return false; }
    if (p[0] - c[0]).abs() < 1e-4 && (p[1] - c[1]).abs() < 1e-4 { return false; }

    let cross1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let cross2 = (c[0] - b[0]) * (p[1] - b[1]) - (c[1] - b[1]) * (p[0] - b[0]);
    let cross3 = (a[0] - c[0]) * (p[1] - c[1]) - (a[1] - c[1]) * (p[0] - c[0]);

    // Strictly inside (all cross products strictly positive for CCW triangle)
    cross1 > 1e-6 && cross2 > 1e-6 && cross3 > 1e-6
}

// Mapbox-style EarCut Triangulator
fn earcut_triangulate_polygon(outer: &[[f32; 2]], holes: &[Vec<[f32; 2]>]) -> (Vec<[f32; 2]>, Vec<[usize; 3]>) {
    let mut ring: Vec<[f32; 2]> = outer.to_vec();
    if signed_area(&ring) < 0.0 {
        ring.reverse(); // Ensure CCW
    }

    let mut sorted_holes: Vec<Vec<[f32; 2]>> = holes.to_vec();
    for h in &mut sorted_holes {
        if signed_area(h) > 0.0 {
            h.reverse(); // Ensure CW for holes
        }
    }
    sorted_holes.sort_by(|a, b| {
        let max_a = a.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        let max_b = b.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        max_b.partial_cmp(&max_a).unwrap()
    });

    for h in &sorted_holes {
        if h.len() < 3 { continue; }
        let mut best_h_idx = 0;
        let mut max_hx = f32::NEG_INFINITY;
        for (i, p) in h.iter().enumerate() {
            if p[0] > max_hx {
                max_hx = p[0];
                best_h_idx = i;
            }
        }
        let h_pt = h[best_h_idx];

        let mut min_x_intersect = f32::INFINITY;
        let mut best_m_idx = 0;
        let n_ring = ring.len();

        for i in 0..n_ring {
            let p0 = ring[i];
            let p1 = ring[(i + 1) % n_ring];

            let (low, high) = if p0[1] < p1[1] { (p0, p1) } else { (p1, p0) };
            if h_pt[1] >= low[1] && h_pt[1] <= high[1] && (high[1] - low[1]).abs() > 1e-6 {
                let t = (h_pt[1] - low[1]) / (high[1] - low[1]);
                let ix = low[0] + t * (high[0] - low[0]);
                if ix >= h_pt[0] && ix < min_x_intersect {
                    min_x_intersect = ix;
                    best_m_idx = if p0[0] > p1[0] { i } else { (i + 1) % n_ring };
                }
            }
        }

        if min_x_intersect.is_infinite() {
            let mut min_dist_sq = f32::INFINITY;
            for (i, p) in ring.iter().enumerate() {
                let dx = p[0] - h_pt[0];
                let dy = p[1] - h_pt[1];
                let d2 = dx * dx + dy * dy;
                if d2 < min_dist_sq {
                    min_dist_sq = d2;
                    best_m_idx = i;
                }
            }
        }

        let mut new_ring = Vec::with_capacity(ring.len() + h.len() + 2);
        new_ring.extend_from_slice(&ring[..=best_m_idx]);
        for k in 0..h.len() {
            new_ring.push(h[(best_h_idx + k) % h.len()]);
        }
        new_ring.push(h[best_h_idx]);
        new_ring.extend_from_slice(&ring[best_m_idx..]);
        ring = new_ring;
    }

    let verts = ring;
    let mut indices_map: Vec<usize> = (0..verts.len()).collect();
    let mut triangles = Vec::new();

    let mut count = 0;
    while indices_map.len() > 2 && count < 2000 {
        count += 1;
        let n = indices_map.len();
        let mut ear_found = false;

        for i in 0..n {
            let prev_idx = indices_map[(i + n - 1) % n];
            let curr_idx = indices_map[i];
            let next_idx = indices_map[(i + 1) % n];

            let a = verts[prev_idx];
            let b = verts[curr_idx];
            let c = verts[next_idx];

            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if cross <= 1e-7 {
                continue;
            }

            let mut inside = false;
            for j in 0..n {
                if j == (i + n - 1) % n || j == i || j == (i + 1) % n {
                    continue;
                }
                let test_pt = verts[indices_map[j]];
                if point_strictly_in_triangle(test_pt, a, b, c) {
                    inside = true;
                    break;
                }
            }

            if !inside {
                triangles.push([prev_idx, curr_idx, next_idx]);
                indices_map.remove(i);
                ear_found = true;
                break;
            }
        }

        if !ear_found {
            // Find convex vertex with non-zero angle
            let mut best_cut = 0;
            let mut max_cross = f32::NEG_INFINITY;
            for i in 0..n {
                let prev_idx = indices_map[(i + n - 1) % n];
                let curr_idx = indices_map[i];
                let next_idx = indices_map[(i + 1) % n];
                let a = verts[prev_idx];
                let b = verts[curr_idx];
                let c = verts[next_idx];
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                if cross > max_cross {
                    max_cross = cross;
                    best_cut = i;
                }
            }
            if n > 2 && max_cross > -1.0 {
                let prev_idx = indices_map[(best_cut + n - 1) % n];
                let curr_idx = indices_map[best_cut];
                let next_idx = indices_map[(best_cut + 1) % n];
                triangles.push([prev_idx, curr_idx, next_idx]);
                indices_map.remove(best_cut);
            } else {
                break;
            }
        }
    }

    (verts, triangles)
}

fn triangulate_glyph_contours(contours: &[Vec<[f32; 2]>]) -> (Vec<[f32; 2]>, Vec<[usize; 3]>) {
    let mut outers = Vec::new();
    let mut holes = Vec::new();

    for c in contours {
        if c.len() < 3 { continue; }
        let area = signed_area(c);
        if area < 0.0 {
            outers.push(c.clone());
        } else {
            holes.push(c.clone());
        }
    }

    if outers.is_empty() && !holes.is_empty() {
        outers = holes;
        holes = Vec::new();
    }

    let mut all_verts = Vec::new();
    let mut all_tris = Vec::new();

    for outer in &outers {
        let matching_holes: Vec<Vec<[f32; 2]>> = holes
            .iter()
            .filter(|h| h.first().map_or(false, |p| point_in_polygon(*p, outer)))
            .cloned()
            .collect();

        let (verts, tris) = earcut_triangulate_polygon(outer, &matching_holes);
        let base_idx = all_verts.len();
        all_verts.extend(verts);
        for t in tris {
            all_tris.push([base_idx + t[0], base_idx + t[1], base_idx + t[2]]);
        }
    }

    (all_verts, all_tris)
}

struct Simple3DMesh {
    vertices: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

fn generate_3d_glyph_mesh(face: &ttf_parser::Face, ch: char, depth: f32, bevel: f32) -> Option<Simple3DMesh> {
    let glyph_id = face.glyph_index(ch)?;
    let mut builder = ContourBuilder::new();
    let _bbox = face.outline_glyph(glyph_id, &mut builder)?;
    let contours = builder.finish();

    let scale = 1.0 / face.units_per_em() as f32;

    let scaled_contours: Vec<Vec<[f32; 2]>> = contours
        .iter()
        .filter(|c| c.points.len() >= 3)
        .map(|c| c.points.iter().map(|p| [p[0] * scale, p[1] * scale]).collect())
        .collect();

    if scaled_contours.is_empty() {
        return None;
    }

    let (face_verts_2d, face_tris) = triangulate_glyph_contours(&scaled_contours);

    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let hz = depth * 0.5;

    // 1. Front and Back Faces
    let start_front = vertices.len() as u32;
    for p in &face_verts_2d {
        vertices.push([p[0], p[1], hz]);
        normals.push([0.0, 0.0, 1.0]);
    }
    for tri in &face_tris {
        indices.push(start_front + tri[0] as u32);
        indices.push(start_front + tri[1] as u32);
        indices.push(start_front + tri[2] as u32);
    }

    let start_back = vertices.len() as u32;
    for p in &face_verts_2d {
        vertices.push([p[0], p[1], -hz]);
        normals.push([0.0, 0.0, -1.0]);
    }
    for tri in &face_tris {
        indices.push(start_back + tri[0] as u32);
        indices.push(start_back + tri[2] as u32);
        indices.push(start_back + tri[1] as u32);
    }

    // 2. Extruded Sidewalls and Bevel Chamfers
    for pts in &scaled_contours {
        let n_pts = pts.len();
        let area = signed_area(pts);
        let is_outer = area < 0.0;

        for i in 0..n_pts {
            let p0 = pts[i];
            let p1 = pts[(i + 1) % n_pts];

            let dx = p1[0] - p0[0];
            let dy = p1[1] - p0[1];
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 { continue; }

            let mut nx = dy / len;
            let mut ny = -dx / len;
            if !is_outer {
                nx = -nx;
                ny = -ny;
            }

            let start_v = vertices.len() as u32;

            // Side quad
            let norm_side = [nx, ny, 0.0];
            vertices.push([p0[0], p0[1], -hz + bevel]); normals.push(norm_side);
            vertices.push([p1[0], p1[1], -hz + bevel]); normals.push(norm_side);
            vertices.push([p1[0], p1[1], hz - bevel]);  normals.push(norm_side);
            vertices.push([p0[0], p0[1], hz - bevel]);  normals.push(norm_side);

            indices.extend_from_slice(&[start_v, start_v + 1, start_v + 2, start_v, start_v + 2, start_v + 3]);

            // Top Chamfer quad (+Z)
            let start_c1 = vertices.len() as u32;
            let norm_c1 = [nx * 0.7071, ny * 0.7071, 0.7071];
            vertices.push([p0[0], p0[1], hz - bevel]); normals.push(norm_c1);
            vertices.push([p1[0], p1[1], hz - bevel]); normals.push(norm_c1);
            vertices.push([p1[0] - nx * bevel, p1[1] - ny * bevel, hz]); normals.push(norm_c1);
            vertices.push([p0[0] - nx * bevel, p0[1] - ny * bevel, hz]); normals.push(norm_c1);

            indices.extend_from_slice(&[start_c1, start_c1 + 1, start_c1 + 2, start_c1, start_c1 + 2, start_c1 + 3]);

            // Bot Chamfer quad (-Z)
            let start_c2 = vertices.len() as u32;
            let norm_c2 = [nx * 0.7071, ny * 0.7071, -0.7071];
            vertices.push([p0[0] - nx * bevel, p0[1] - ny * bevel, -hz]); normals.push(norm_c2);
            vertices.push([p1[0] - nx * bevel, p1[0] - ny * bevel, -hz]); normals.push(norm_c2);
            vertices.push([p1[0], p1[1], -hz + bevel]); normals.push(norm_c2);
            vertices.push([p0[0], p0[1], -hz + bevel]); normals.push(norm_c2);

            indices.extend_from_slice(&[start_c2, start_c2 + 1, start_c2 + 2, start_c2, start_c2 + 2, start_c2 + 3]);
        }
    }

    Some(Simple3DMesh { vertices, normals, indices })
}

fn render_font_preview(font_path: &str, output_path: &str) {
    let font_bytes = std::fs::read(font_path).expect("Failed to read font");
    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Failed to parse font");

    let grid_chars = [
        "ABCDEF",
        "GHIJKL",
        "MNOPQR",
        "STUVWX",
        "YZ0123",
        "456789",
        "!?'-.,",
    ];

    let img_w = 1200;
    let img_h = 1200;
    let mut img = image::RgbImage::new(img_w, img_h);

    for pixel in img.pixels_mut() {
        *pixel = image::Rgb([8, 12, 16]);
    }

    let mut z_buf = vec![f32::INFINITY; (img_w * img_h) as usize];

    let cols = 6;
    let rows = grid_chars.len();
    let cell_w = img_w as f32 / cols as f32;
    let cell_h = img_h as f32 / rows as f32;

    for (row, line) in grid_chars.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if let Some(mesh) = generate_3d_glyph_mesh(&face, ch, 0.22, 0.02) {
                let center_x = (col as f32 + 0.5) * cell_w;
                let center_y = (row as f32 + 0.6) * cell_h;

                let char_scale = cell_h * 0.68;

                for tri_idx in 0..(mesh.indices.len() / 3) {
                    let i0 = mesh.indices[tri_idx * 3] as usize;
                    let i1 = mesh.indices[tri_idx * 3 + 1] as usize;
                    let i2 = mesh.indices[tri_idx * 3 + 2] as usize;

                    let v0 = mesh.vertices[i0];
                    let v1 = mesh.vertices[i1];
                    let v2 = mesh.vertices[i2];
                    let n0 = mesh.normals[i0];

                    let proj = |v: [f32; 3]| -> [f32; 3] {
                        let sx = center_x + (v[0] - 0.3) * char_scale + v[2] * 22.0;
                        let sy = center_y - (v[1] - 0.4) * char_scale - v[2] * 22.0;
                        let sz = -v[2];
                        [sx, sy, sz]
                    };

                    let p0 = proj(v0);
                    let p1 = proj(v1);
                    let p2 = proj(v2);

                    let light_dir = [0.0f32, 0.8, 0.6];
                    let ndotl = (n0[0] * light_dir[0] + n0[1] * light_dir[1] + n0[2] * light_dir[2]).max(0.0);
                    let intensity = 0.35 + 0.65 * ndotl;

                    let r = (210.0 * intensity + 45.0 * n0[2].max(0.0)).clamp(0.0, 255.0) as u8;
                    let g = (235.0 * intensity + 20.0 * n0[2].max(0.0)).clamp(0.0, 255.0) as u8;
                    let b = (255.0 * intensity).clamp(0.0, 255.0) as u8;

                    let min_x = p0[0].min(p1[0]).min(p2[0]).max(0.0) as u32;
                    let max_x = p0[0].max(p1[0]).max(p2[0]).min(img_w as f32 - 1.0) as u32;
                    let min_y = p0[1].min(p1[1]).min(p2[1]).max(0.0) as u32;
                    let max_y = p0[1].max(p1[1]).max(p2[1]).min(img_h as f32 - 1.0) as u32;

                    let is_point_in_tri = |p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]| -> bool {
                        let cross1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                        let cross2 = (c[0] - b[0]) * (p[1] - b[1]) - (c[1] - b[1]) * (p[0] - b[0]);
                        let cross3 = (a[0] - c[0]) * (p[1] - c[1]) - (a[1] - c[1]) * (p[0] - c[0]);
                        let has_neg = (cross1 < -1e-6) || (cross2 < -1e-6) || (cross3 < -1e-6);
                        let has_pos = (cross1 > 1e-6) || (cross2 > 1e-6) || (cross3 > 1e-6);
                        !(has_neg && has_pos)
                    };

                    for py in min_y..=max_y {
                        for px in min_x..=max_x {
                            let p = [px as f32 + 0.5, py as f32 + 0.5];
                            if is_point_in_tri(p, [p0[0], p0[1]], [p1[0], p1[1]], [p2[0], p2[1]]) {
                                let idx = (py * img_w + px) as usize;
                                if p0[2] < z_buf[idx] {
                                    z_buf[idx] = p0[2];
                                    img.put_pixel(px, py, image::Rgb([r, g, b]));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    img.save(output_path).expect("Failed to save preview");
    println!("Saved preview to {}", output_path);
}

fn render_phrase_preview(font_path: &str, output_path: &str, phrase: &str) {
    let font_bytes = std::fs::read(font_path).expect("Failed to read font");
    let face = ttf_parser::Face::parse(&font_bytes, 0).expect("Failed to parse font");

    let img_w = 1400;
    let img_h = 400;
    let mut img = image::RgbImage::new(img_w, img_h);

    for pixel in img.pixels_mut() {
        *pixel = image::Rgb([8, 12, 16]);
    }

    let mut z_buf = vec![f32::INFINITY; (img_w * img_h) as usize];

    let em = face.units_per_em() as f32;
    let base_scale = 1.0 / em;
    let mut total_advance = 0.0f32;
    for ch in phrase.chars() {
        if let Some(glyph_id) = face.glyph_index(ch) {
            let adv = face.glyph_hor_advance(glyph_id).unwrap_or(face.units_per_em()) as f32 * base_scale;
            total_advance += adv;
        } else {
            total_advance += 0.5;
        }
    }

    let text_scale = 7.5 / total_advance.max(1.0);
    let start_x = -total_advance * text_scale * 0.5;
    let depth = 0.22 * text_scale;
    let bevel = 0.024 * text_scale;
    let mut curr_x = start_x;

    let world_to_screen = |v: [f32; 3]| -> [f32; 3] {
        let sx = (img_w as f32 * 0.5) + v[0] * 140.0 + v[2] * 20.0;
        let sy = (img_h as f32 * 0.65) - v[1] * 140.0 - v[2] * 20.0;
        let sz = -v[2];
        [sx, sy, sz]
    };

    for ch in phrase.chars() {
        if let Some(glyph_id) = face.glyph_index(ch) {
            let adv = face.glyph_hor_advance(glyph_id).unwrap_or(face.units_per_em()) as f32 * base_scale * text_scale;
            let mut builder = ContourBuilder::new();
            if let Some(_bbox) = face.outline_glyph(glyph_id, &mut builder) {
                let contours = builder.finish();
                let scaled_contours: Vec<Vec<[f32; 2]>> = contours
                    .iter()
                    .filter(|c| c.points.len() >= 3)
                    .map(|c| {
                        c.points.iter().map(|p| {
                            [
                                curr_x + p[0] * base_scale * text_scale,
                                p[1] * base_scale * text_scale,
                            ]
                        }).collect()
                    })
                    .collect();

                if !scaled_contours.is_empty() {
                    let (face_verts_2d, face_tris) = triangulate_glyph_contours(&scaled_contours);
                    let hz = depth * 0.5;

                    let mut vertices = Vec::new();
                    let mut normals = Vec::new();
                    let mut indices = Vec::new();

                    // Front & Back
                    let start_f = vertices.len() as u32;
                    for p in &face_verts_2d {
                        vertices.push([p[0], p[1], hz]);
                        normals.push([0.0, 0.0, 1.0]);
                    }
                    for tri in &face_tris {
                        indices.extend_from_slice(&[start_f + tri[0] as u32, start_f + tri[1] as u32, start_f + tri[2] as u32]);
                    }

                    // Sidewalls & Bevels
                    for pts in &scaled_contours {
                        let n_pts = pts.len();
                        let area = signed_area(pts);
                        let is_outer = area < 0.0;

                        for i in 0..n_pts {
                            let p0 = pts[i];
                            let p1 = pts[(i + 1) % n_pts];

                            let dx = p1[0] - p0[0];
                            let dy = p1[1] - p0[1];
                            let len = (dx * dx + dy * dy).sqrt();
                            if len < 1e-6 { continue; }

                            let mut nx = dy / len;
                            let mut ny = -dx / len;
                            if !is_outer {
                                nx = -nx;
                                ny = -ny;
                            }

                            let start_v = vertices.len() as u32;

                            // Side quad
                            let norm_side = [nx, ny, 0.0];
                            vertices.push([p0[0], p0[1], -hz + bevel]); normals.push(norm_side);
                            vertices.push([p1[0], p1[1], -hz + bevel]); normals.push(norm_side);
                            vertices.push([p1[0], p1[1], hz - bevel]);  normals.push(norm_side);
                            vertices.push([p0[0], p0[1], hz - bevel]);  normals.push(norm_side);
                            indices.extend_from_slice(&[start_v, start_v + 1, start_v + 2, start_v, start_v + 2, start_v + 3]);

                            // Top Chamfer quad (+Z)
                            let start_c1 = vertices.len() as u32;
                            let norm_c1 = [nx * 0.7071, ny * 0.7071, 0.7071];
                            vertices.push([p0[0], p0[1], hz - bevel]); normals.push(norm_c1);
                            vertices.push([p1[0], p1[1], hz - bevel]); normals.push(norm_c1);
                            vertices.push([p1[0] - nx * bevel, p1[1] - ny * bevel, hz]); normals.push(norm_c1);
                            vertices.push([p0[0] - nx * bevel, p0[1] - ny * bevel, hz]); normals.push(norm_c1);
                            indices.extend_from_slice(&[start_c1, start_c1 + 1, start_c1 + 2, start_c1, start_c1 + 2, start_c1 + 3]);
                        }
                    }

                    // Rasterize mesh
                    for tri_idx in 0..(indices.len() / 3) {
                        let i0 = indices[tri_idx * 3] as usize;
                        let i1 = indices[tri_idx * 3 + 1] as usize;
                        let i2 = indices[tri_idx * 3 + 2] as usize;

                        let p0 = world_to_screen(vertices[i0]);
                        let p1 = world_to_screen(vertices[i1]);
                        let p2 = world_to_screen(vertices[i2]);
                        let n0 = normals[i0];

                        let light_dir = [0.0f32, 0.8, 0.6];
                        let ndotl = (n0[0] * light_dir[0] + n0[1] * light_dir[1] + n0[2] * light_dir[2]).max(0.0);
                        let intensity = 0.35 + 0.65 * ndotl;

                        let r = (210.0 * intensity + 45.0 * n0[2].max(0.0)).clamp(0.0, 255.0) as u8;
                        let g = (235.0 * intensity + 20.0 * n0[2].max(0.0)).clamp(0.0, 255.0) as u8;
                        let b = (255.0 * intensity).clamp(0.0, 255.0) as u8;

                        let min_x = p0[0].min(p1[0]).min(p2[0]).max(0.0) as u32;
                        let max_x = p0[0].max(p1[0]).max(p2[0]).min(img_w as f32 - 1.0) as u32;
                        let min_y = p0[1].min(p1[1]).min(p2[1]).max(0.0) as u32;
                        let max_y = p0[1].max(p1[1]).max(p2[1]).min(img_h as f32 - 1.0) as u32;

                        let is_point_in_tri = |p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]| -> bool {
                            let cross1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                            let cross2 = (c[0] - b[0]) * (p[1] - b[1]) - (c[1] - b[1]) * (p[0] - b[0]);
                            let cross3 = (a[0] - c[0]) * (p[1] - c[1]) - (a[1] - c[1]) * (p[0] - c[0]);
                            let has_neg = (cross1 < -1e-6) || (cross2 < -1e-6) || (cross3 < -1e-6);
                            let has_pos = (cross1 > 1e-6) || (cross2 > 1e-6) || (cross3 > 1e-6);
                            !(has_neg && has_pos)
                        };

                        for py in min_y..=max_y {
                            for px in min_x..=max_x {
                                let p = [px as f32 + 0.5, py as f32 + 0.5];
                                if is_point_in_tri(p, [p0[0], p0[1]], [p1[0], p1[1]], [p2[0], p2[1]]) {
                                    let idx = (py * img_w + px) as usize;
                                    if p0[2] < z_buf[idx] {
                                        z_buf[idx] = p0[2];
                                        img.put_pixel(px, py, image::Rgb([r, g, b]));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            curr_x += adv;
        } else {
            curr_x += 0.4 * text_scale;
        }
    }

    img.save(output_path).expect("Failed to save phrase preview");
    println!("Saved phrase preview to {}", output_path);
}

fn main() {
    render_font_preview("assets/Orbitron-Black.ttf", "/home/naoki/.gemini/antigravity/brain/81080094-7121-4c13-a627-684df1a03458/alphabet_orbitron.png");
    let dejavu_path = "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf";
    if Path::new(dejavu_path).exists() {
        render_font_preview(dejavu_path, "/home/naoki/.gemini/antigravity/brain/81080094-7121-4c13-a627-684df1a03458/alphabet_dejavu.png");
        render_phrase_preview(dejavu_path, "/home/naoki/.gemini/antigravity/brain/81080094-7121-4c13-a627-684df1a03458/phrase_preview.png", "AND SO YOU'RE BACK");
    }
}
