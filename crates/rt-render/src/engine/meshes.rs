use super::Vertex;

pub fn generate_lamp_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Helper to add a box
    let mut add_box = |center: [f32; 3], size: [f32; 3], color_flag: f32| {
        let half_x = size[0] / 2.0;
        let half_y = size[1] / 2.0;
        let half_z = size[2] / 2.0;

        let local_verts = [
            // front
            [-half_x, -half_y, half_z],
            [half_x, -half_y, half_z],
            [half_x, half_y, half_z],
            [-half_x, half_y, half_z],
            // back
            [-half_x, -half_y, -half_z],
            [-half_x, half_y, -half_z],
            [half_x, half_y, -half_z],
            [half_x, -half_y, -half_z],
        ];

        let normals = [
            [0.0, 0.0, 1.0],  // front
            [0.0, 0.0, -1.0], // back
            [-1.0, 0.0, 0.0], // left
            [1.0, 0.0, 0.0],  // right
            [0.0, 1.0, 0.0],  // top
            [0.0, -1.0, 0.0], // bottom
        ];

        let face_indices = [
            [0, 1, 2, 0, 2, 3], // front
            [4, 5, 6, 4, 6, 7], // back
            [4, 0, 3, 4, 3, 5], // left
            [1, 7, 6, 1, 6, 2], // right
            [3, 2, 6, 3, 6, 5], // top
            [4, 7, 1, 4, 1, 0], // bottom
        ];

        // Add vertices and indices for each face to have clean normals
        for (face_idx, &_indices_map) in face_indices.iter().enumerate() {
            let start_v = vertices.len() as u32;
            let normal = normals[face_idx];

            let unique_vert_indices = match face_idx {
                0 => [0, 1, 2, 3], // front
                1 => [4, 5, 6, 7], // back
                2 => [4, 0, 3, 5], // left
                3 => [1, 7, 6, 2], // right
                4 => [3, 2, 6, 5], // top
                5 => [4, 7, 1, 0], // bottom
                _ => unreachable!(),
            };

            for &vi in &unique_vert_indices {
                let p = local_verts[vi];
                vertices.push(Vertex {
                    position: [p[0] + center[0], p[1] + center[1], p[2] + center[2]],
                    normal,
                    tex_coords: [color_flag, (p[1] + center[1]) / 11.0],
                });
            }

            indices.push(start_v);
            indices.push(start_v + 1);
            indices.push(start_v + 2);
            indices.push(start_v);
            indices.push(start_v + 2);
            indices.push(start_v + 3);
        }
    };

    // 1. Pole: Vertical box
    add_box([0.0, 5.5, 0.0], [0.24, 11.0, 0.24], 0.0);

    // 2. Overhang arm: Horizontal box
    // Extends by 2.0 units in +X direction (towards road)
    add_box([1.0, 11.0, 0.0], [2.0, 0.2, 0.2], 0.0);

    // 3. Lamp Fitting/Head: Box at the end of overhang
    add_box([2.0, 10.8, 0.0], [0.8, 0.3, 0.5], 2.0); // Flag 2.0 for fixture

    // 4. Lamp Bulb/Emissive source: Smaller glowing box underneath the fitting
    add_box([2.0, 10.6, 0.0], [0.4, 0.1, 0.3], 1.0); // Flag 1.0 for emissive bulb

    (vertices, indices)
}

#[allow(clippy::too_many_arguments)]
fn room_add_quad(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    normal: [f32; 3],
    ch: f32,
    mat: f32,
) {
    let start = vertices.len() as u32;
    vertices.push(Vertex {
        position: p0,
        normal,
        tex_coords: [ch, mat],
    });
    vertices.push(Vertex {
        position: p1,
        normal,
        tex_coords: [ch, mat],
    });
    vertices.push(Vertex {
        position: p2,
        normal,
        tex_coords: [ch, mat],
    });
    vertices.push(Vertex {
        position: p3,
        normal,
        tex_coords: [ch, mat],
    });
    indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

fn room_add_box(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    size: [f32; 3],
    rot_y: f32,
    ch: f32,
    mat: f32,
) {
    let hx = size[0] / 2.0;
    let hy = size[1] / 2.0;
    let hz = size[2] / 2.0;
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();

    let rotate_pt = |p: [f32; 3]| -> [f32; 3] {
        let rx = p[0] * cos_r + p[2] * sin_r;
        let rz = -p[0] * sin_r + p[2] * cos_r;
        [rx + center[0], p[1] + center[1], rz + center[2]]
    };
    let rotate_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    // Front face (+z)
    room_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, -hy, hz]),
        rotate_pt([hx, -hy, hz]),
        rotate_pt([hx, hy, hz]),
        rotate_pt([-hx, hy, hz]),
        rotate_norm([0.0, 0.0, 1.0]),
        ch,
        mat,
    );
    // Back face (-z)
    room_add_quad(
        vertices,
        indices,
        rotate_pt([hx, -hy, -hz]),
        rotate_pt([-hx, -hy, -hz]),
        rotate_pt([-hx, hy, -hz]),
        rotate_pt([hx, hy, -hz]),
        rotate_norm([0.0, 0.0, -1.0]),
        ch,
        mat,
    );
    // Left face (-x)
    room_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, -hy, -hz]),
        rotate_pt([-hx, -hy, hz]),
        rotate_pt([-hx, hy, hz]),
        rotate_pt([-hx, hy, -hz]),
        rotate_norm([-1.0, 0.0, 0.0]),
        ch,
        mat,
    );
    // Right face (+x)
    room_add_quad(
        vertices,
        indices,
        rotate_pt([hx, -hy, hz]),
        rotate_pt([hx, -hy, -hz]),
        rotate_pt([hx, hy, -hz]),
        rotate_pt([hx, hy, hz]),
        rotate_norm([1.0, 0.0, 0.0]),
        ch,
        mat,
    );
    // Top face (+y)
    room_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, hy, hz]),
        rotate_pt([hx, hy, hz]),
        rotate_pt([hx, hy, -hz]),
        rotate_pt([-hx, hy, -hz]),
        rotate_norm([0.0, 1.0, 0.0]),
        ch,
        mat,
    );
    // Bottom face (-y)
    room_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, -hy, -hz]),
        rotate_pt([hx, -hy, -hz]),
        rotate_pt([hx, -hy, hz]),
        rotate_pt([-hx, -hy, hz]),
        rotate_norm([0.0, -1.0, 0.0]),
        ch,
        mat,
    );
}

#[allow(clippy::too_many_arguments)]
fn room_add_cone(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    radius: f32,
    depth: f32,
    rot_y: f32,
    ch: f32,
    mat: f32,
) {
    let segs = 18;
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();
    let rotate_pt = |p: [f32; 3]| -> [f32; 3] {
        let rx = p[0] * cos_r + p[2] * sin_r;
        let rz = -p[0] * sin_r + p[2] * cos_r;
        [rx + center[0], p[1] + center[1], rz + center[2]]
    };
    let rotate_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    let n = rotate_norm([0.0, 0.0, 1.0]);
    let apex = rotate_pt([0.0, 0.0, 0.02]);
    let r_cap = radius * 0.40;
    let z_throat = -depth;

    for i in 0..segs {
        let a0 = (i as f32) / (segs as f32) * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32) / (segs as f32) * std::f32::consts::TAU;
        let c0 = a0.cos();
        let s0 = a0.sin();
        let c1 = a1.cos();
        let s1 = a1.sin();

        // 1. Inner protruding dustcap dome (high excursion weight: 0.72..1.0)
        let cap0 = rotate_pt([c0 * r_cap, s0 * r_cap, z_throat]);
        let cap1 = rotate_pt([c1 * r_cap, s1 * r_cap, z_throat]);
        let start = vertices.len() as u32;
        vertices.push(Vertex {
            position: apex,
            normal: n,
            tex_coords: [ch, mat + 0.25],
        });
        vertices.push(Vertex {
            position: cap0,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        vertices.push(Vertex {
            position: cap1,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        indices.extend_from_slice(&[start, start + 1, start + 2]);

        // 2. Conical ribbed diaphragm from dustcap throat out to surround rim (weight: 0.72 -> 0.0)
        let p0 = rotate_pt([c0 * radius, s0 * radius, 0.0]);
        let p1 = rotate_pt([c1 * radius, s1 * radius, 0.0]);
        let d_start = vertices.len() as u32;
        vertices.push(Vertex {
            position: cap0,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        vertices.push(Vertex {
            position: p0,
            normal: n,
            tex_coords: [ch, mat],
        });
        vertices.push(Vertex {
            position: p1,
            normal: n,
            tex_coords: [ch, mat],
        });
        vertices.push(Vertex {
            position: cap1,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        indices.extend_from_slice(&[
            d_start,
            d_start + 1,
            d_start + 2,
            d_start,
            d_start + 2,
            d_start + 3,
        ]);

        // 3. Bold emissive neon basket bezel ring (mat = 5.2)
        let r_out = radius * 1.22;
        let p0_out = rotate_pt([c0 * r_out, s0 * r_out, 0.015]);
        let p1_out = rotate_pt([c1 * r_out, s1 * r_out, 0.015]);
        room_add_quad(vertices, indices, p0, p1, p1_out, p0_out, n, ch, 5.2);
    }
}

fn room_add_ceiling_cone(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    radius: f32,
    depth: f32,
    ch: f32,
    mat: f32,
) {
    let segs = 16;
    let n = [0.0, -1.0, 0.0];
    let apex = [center[0], center[1] - 0.02, center[2]];
    let r_cap = radius * 0.40;
    for i in 0..segs {
        let a0 = (i as f32) / (segs as f32) * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32) / (segs as f32) * std::f32::consts::TAU;
        let c0 = a0.cos();
        let s0 = a0.sin();
        let c1 = a1.cos();
        let s1 = a1.sin();

        let cap0 = [
            center[0] + c0 * r_cap,
            center[1] + depth,
            center[2] + s0 * r_cap,
        ];
        let cap1 = [
            center[0] + c1 * r_cap,
            center[1] + depth,
            center[2] + s1 * r_cap,
        ];
        let start = vertices.len() as u32;
        vertices.push(Vertex {
            position: apex,
            normal: n,
            tex_coords: [ch, mat + 0.25],
        });
        vertices.push(Vertex {
            position: cap0,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        vertices.push(Vertex {
            position: cap1,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        indices.extend_from_slice(&[start, start + 1, start + 2]);

        let p0 = [center[0] + c0 * radius, center[1], center[2] + s0 * radius];
        let p1 = [center[0] + c1 * radius, center[1], center[2] + s1 * radius];
        let d_start = vertices.len() as u32;
        vertices.push(Vertex {
            position: cap0,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        vertices.push(Vertex {
            position: p0,
            normal: n,
            tex_coords: [ch, mat],
        });
        vertices.push(Vertex {
            position: p1,
            normal: n,
            tex_coords: [ch, mat],
        });
        vertices.push(Vertex {
            position: cap1,
            normal: n,
            tex_coords: [ch, mat + 0.18],
        });
        indices.extend_from_slice(&[
            d_start,
            d_start + 1,
            d_start + 2,
            d_start,
            d_start + 2,
            d_start + 3,
        ]);

        let r_out = radius * 1.24;
        let p0_out = [
            center[0] + c0 * r_out,
            center[1] - 0.01,
            center[2] + s0 * r_out,
        ];
        let p1_out = [
            center[0] + c1 * r_out,
            center[1] - 0.01,
            center[2] + s1 * r_out,
        ];
        room_add_quad(vertices, indices, p0, p1, p1_out, p0_out, n, ch, 5.2);
    }
}

fn room_add_speaker_tower(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    pos: [f32; 3],
    rot_y: f32,
    ch: f32,
    is_tower: bool,
) {
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();
    let local_to_world = |lx: f32, ly: f32, lz: f32| -> [f32; 3] {
        let rx = lx * cos_r + lz * sin_r;
        let rz = -lx * sin_r + lz * cos_r;
        [rx + pos[0], ly + pos[1], rz + pos[2]]
    };

    if is_tower {
        // Pedestal base & Cabinet (mat = 1.2 for active speaker cabinet)
        room_add_box(
            vertices,
            indices,
            [pos[0], pos[1] - 1.12, pos[2]],
            [0.98, 0.12, 1.05],
            rot_y,
            ch,
            1.2,
        );
        room_add_box(vertices, indices, pos, [0.84, 2.2, 0.90], rot_y, ch, 1.2);

        // Bipolar Front (+0.46) and Rear (-0.46) Driver Arrays so cones are visible from all camera orbit angles
        for &(face_z, face_rot) in &[(0.46f32, rot_y), (-0.46f32, rot_y + std::f32::consts::PI)] {
            // Lower Bass Woofer (mat = 2.0)
            room_add_cone(
                vertices,
                indices,
                local_to_world(0.0, -0.52, face_z),
                0.31,
                0.13,
                face_rot,
                ch,
                2.0,
            );
            // Midrange Cone (mat = 3.0)
            room_add_cone(
                vertices,
                indices,
                local_to_world(0.0, 0.22, face_z),
                0.22,
                0.09,
                face_rot,
                ch,
                3.0,
            );
            // Tweeter Dome (mat = 4.0)
            room_add_cone(
                vertices,
                indices,
                local_to_world(0.0, 0.76, face_z),
                0.11,
                0.04,
                face_rot,
                ch,
                4.0,
            );
        }

        // Emissive vertical neon accent rails on all 4 vertical cabinet corners & base halo (mat = 5.2)
        for &sx in &[-0.43f32, 0.43f32] {
            for &sz in &[-0.43f32, 0.43f32] {
                room_add_box(
                    vertices,
                    indices,
                    local_to_world(sx, 0.0, sz),
                    [0.04, 2.18, 0.04],
                    rot_y,
                    ch,
                    5.2,
                );
            }
        }
        room_add_box(
            vertices,
            indices,
            local_to_world(0.0, -1.08, 0.0),
            [1.02, 0.04, 1.08],
            rot_y,
            ch,
            5.2,
        );
    } else {
        // Stand-mounted surround monitor
        room_add_box(
            vertices,
            indices,
            [pos[0], pos[1] - 0.9, pos[2]],
            [0.10, 1.6, 0.10],
            rot_y,
            ch,
            1.2,
        );
        room_add_box(
            vertices,
            indices,
            [pos[0], pos[1] - 1.7, pos[2]],
            [0.75, 0.06, 0.75],
            rot_y,
            ch,
            1.2,
        );
        // Monitor box (mat = 1.2)
        room_add_box(vertices, indices, pos, [0.72, 1.18, 0.68], rot_y, ch, 1.2);

        // Bipolar Front (+0.35) and Rear (-0.35) drivers
        for &(face_z, face_rot) in &[(0.35f32, rot_y), (-0.35f32, rot_y + std::f32::consts::PI)] {
            // Woofer
            room_add_cone(
                vertices,
                indices,
                local_to_world(0.0, -0.16, face_z),
                0.25,
                0.10,
                face_rot,
                ch,
                2.0,
            );
            // Tweeter
            room_add_cone(
                vertices,
                indices,
                local_to_world(0.0, 0.34, face_z),
                0.10,
                0.04,
                face_rot,
                ch,
                4.0,
            );
        }
        // Neon perimeter halos & corner strips (mat = 5.2)
        room_add_box(
            vertices,
            indices,
            local_to_world(0.0, -0.60, 0.0),
            [0.76, 0.04, 0.72],
            rot_y,
            ch,
            5.2,
        );
        room_add_box(
            vertices,
            indices,
            local_to_world(0.0, 0.60, 0.0),
            [0.76, 0.04, 0.72],
            rot_y,
            ch,
            5.2,
        );
    }
}

pub fn generate_neon_room_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Front-to-back draw order for Early-Z culling: draw interior stage riser, speakers,
    // acoustic diffuser slats, and observation window BEFORE the enclosing room floor/walls/ceiling.
    // Front stage platform riser
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -1.48, 6.2],
        [10.5, 0.24, 4.8],
        0.0,
        0.0,
        1.0,
    );
    // Neon edge along stage riser
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -1.35, 3.8],
        [10.6, 0.04, 0.04],
        0.0,
        3.0,
        5.0,
    );

    // 3. ACOUSTIC DIFFUSER SLATS (mat = 6.0) on back wall
    for i in 0..14 {
        let x = -6.5 + (i as f32) * 1.0;
        let depth_offset = ((i * 7 + 3) % 5) as f32 * 0.06;
        room_add_box(
            &mut vertices,
            &mut indices,
            [x, 1.6, 8.8 - depth_offset],
            [0.55, 4.2, 0.15 + depth_offset],
            0.0,
            0.0,
            6.0,
        );
    }

    // Rear studio control room observation window & diffusers
    // Soundproof glass observation pane (mat = 7.0)
    room_add_quad(
        &mut vertices,
        &mut indices,
        [-4.5, 0.4, -8.85],
        [-4.5, 3.4, -8.85],
        [4.5, 3.4, -8.85],
        [4.5, 0.4, -8.85],
        [0.0, 0.0, 1.0],
        6.0,
        7.0,
    );
    // Window neon illuminated perimeter frame
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, 3.45, -8.82],
        [9.2, 0.08, 0.08],
        0.0,
        6.0,
        5.0,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, 0.35, -8.82],
        [9.2, 0.08, 0.08],
        0.0,
        7.0,
        5.0,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [-4.55, 1.9, -8.82],
        [0.08, 3.1, 0.08],
        0.0,
        6.0,
        5.0,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [4.55, 1.9, -8.82],
        [0.08, 3.1, 0.08],
        0.0,
        7.0,
        5.0,
    );

    // Rear flank acoustic diffuser panels
    for i in 0..4 {
        let x_left = -8.8 + (i as f32) * 0.9;
        let x_right = 6.1 + (i as f32) * 0.9;
        let depth_offset = ((i * 5 + 2) % 4) as f32 * 0.05;
        room_add_box(
            &mut vertices,
            &mut indices,
            [x_left, 1.6, -8.8 + depth_offset],
            [0.5, 3.6, 0.12 + depth_offset],
            0.0,
            6.0,
            6.0,
        );
        room_add_box(
            &mut vertices,
            &mut indices,
            [x_right, 1.6, -8.8 + depth_offset],
            [0.5, 3.6, 0.12 + depth_offset],
            0.0,
            7.0,
            6.0,
        );
    }

    // Side acoustic panels with illuminated neon perimeter backlights
    for i in 0..4 {
        let z = -4.5 + (i as f32) * 3.0;
        // Left side panel + halo
        room_add_box(
            &mut vertices,
            &mut indices,
            [-10.8, 1.2, z],
            [0.18, 2.4, 1.8],
            0.0,
            4.0,
            6.0,
        );
        room_add_box(
            &mut vertices,
            &mut indices,
            [-10.85, 1.2, z],
            [0.06, 2.5, 1.9],
            0.0,
            4.0,
            5.0,
        );

        // Right side panel + halo
        room_add_box(
            &mut vertices,
            &mut indices,
            [10.8, 1.2, z],
            [0.18, 2.4, 1.8],
            0.0,
            5.0,
            6.0,
        );
        room_add_box(
            &mut vertices,
            &mut indices,
            [10.85, 1.2, z],
            [0.06, 2.5, 1.9],
            0.0,
            5.0,
            5.0,
        );
    }

    // Overhead neon rail trusses
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, 4.8, 4.0],
        [21.0, 0.08, 0.08],
        0.0,
        0.0,
        5.0,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, 4.8, -3.0],
        [21.0, 0.08, 0.08],
        0.0,
        6.0,
        5.0,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [-8.0, 4.8, 0.0],
        [0.08, 0.08, 16.0],
        0.0,
        4.0,
        5.0,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [8.0, 4.8, 0.0],
        [0.08, 0.08, 16.0],
        0.0,
        5.0,
        5.0,
    );

    // 4. SPATIAL SPEAKERS (7.1.4 Surround Placement — all oriented inward toward the listening sweet spot / camera)
    // Channel 0: Front Left (Tower) — angled toward room center & camera (-Z, +X)
    room_add_speaker_tower(
        &mut vertices,
        &mut indices,
        [-3.8, -0.25, 5.5],
        std::f32::consts::PI - 0.38,
        0.0,
        true,
    );
    // Channel 1: Front Right (Tower) — angled toward room center & camera (-Z, -X)
    room_add_speaker_tower(
        &mut vertices,
        &mut indices,
        [3.8, -0.25, 5.5],
        -std::f32::consts::PI + 0.38,
        1.0,
        true,
    );

    // Channel 3: Massive LFE Dual Subwoofer (base of stacked front center array, flush on stage riser y = -1.36)
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -0.90, 5.50],
        [2.5, 0.92, 1.1],
        0.0,
        3.0,
        1.2,
    );
    room_add_cone(
        &mut vertices,
        &mut indices,
        [-0.64, -0.90, 4.94],
        0.38,
        0.17,
        std::f32::consts::PI,
        3.0,
        2.0,
    );
    room_add_cone(
        &mut vertices,
        &mut indices,
        [0.64, -0.90, 4.94],
        0.38,
        0.17,
        std::f32::consts::PI,
        3.0,
        2.0,
    );
    // Subwoofer bass reflex port & neon trim on front face (Z = 4.94)
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -0.57, 4.94],
        [0.42, 0.08, 0.05],
        0.0,
        3.0,
        5.2,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -1.335, 5.50],
        [2.56, 0.05, 1.16],
        0.0,
        3.0,
        5.2,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -0.44, 5.50],
        [2.56, 0.05, 1.16],
        0.0,
        3.0,
        5.2,
    );

    // Acoustic isolation plinth bridging LFE Subwoofer top and Center Horizontal Cabinet
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -0.385, 5.32],
        [1.72, 0.06, 0.62],
        0.0,
        0.0,
        1.0,
    );

    // Channel 2: Center Channel (Horizontal Cabinet stacked atop LFE Subwoofer, front baffle at Z = 4.98 facing -Z)
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -0.02, 5.32],
        [1.8, 0.60, 0.66],
        0.0,
        2.0,
        1.2,
    );
    room_add_cone(
        &mut vertices,
        &mut indices,
        [-0.54, -0.02, 4.98],
        0.21,
        0.08,
        std::f32::consts::PI,
        2.0,
        3.0,
    );
    room_add_cone(
        &mut vertices,
        &mut indices,
        [0.54, -0.02, 4.98],
        0.21,
        0.08,
        std::f32::consts::PI,
        2.0,
        3.0,
    );
    room_add_cone(
        &mut vertices,
        &mut indices,
        [0.0, -0.02, 4.98],
        0.11,
        0.04,
        std::f32::consts::PI,
        2.0,
        4.0,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, -0.33, 5.32],
        [1.86, 0.04, 0.70],
        0.0,
        2.0,
        5.2,
    );
    room_add_box(
        &mut vertices,
        &mut indices,
        [0.0, 0.29, 5.32],
        [1.86, 0.04, 0.70],
        0.0,
        2.0,
        5.2,
    );

    // Channel 4: Surround Left (Stand Monitor) — angled inward (+X) and toward camera (-Z)
    room_add_speaker_tower(
        &mut vertices,
        &mut indices,
        [-6.2, 0.5, 0.5],
        2.05,
        4.0,
        false,
    );
    // Channel 5: Surround Right (Stand Monitor) — angled inward (-X) and toward camera (-Z)
    room_add_speaker_tower(
        &mut vertices,
        &mut indices,
        [6.2, 0.5, 0.5],
        -2.05,
        5.0,
        false,
    );

    // Channel 6: Rear Left (Rear Surround Bipolar Tower) — angled inward (+X, -Z / +Z)
    room_add_speaker_tower(
        &mut vertices,
        &mut indices,
        [-4.2, 0.1, -4.8],
        2.25,
        6.0,
        true,
    );
    // Channel 7: Rear Right (Rear Surround Bipolar Tower) — angled inward (-X, -Z / +Z)
    room_add_speaker_tower(
        &mut vertices,
        &mut indices,
        [4.2, 0.1, -4.8],
        -2.25,
        7.0,
        true,
    );

    // Channels 8..11: Overhead Ceiling Atmos Height Speakers (downward-firing cones + neon trim)
    for &(ch_id, cx, cz) in &[
        (8.0f32, -3.2f32, 3.5f32),
        (9.0f32, 3.2f32, 3.5f32),
        (10.0f32, -3.2f32, -2.5f32),
        (11.0f32, 3.2f32, -2.5f32),
    ] {
        room_add_box(
            &mut vertices,
            &mut indices,
            [cx, 4.32, cz],
            [0.72, 0.46, 0.72],
            0.0,
            ch_id,
            1.2,
        );
        room_add_box(
            &mut vertices,
            &mut indices,
            [cx, 4.09, cz],
            [0.76, 0.04, 0.76],
            0.0,
            ch_id,
            5.2,
        );
        room_add_ceiling_cone(
            &mut vertices,
            &mut indices,
            [cx, 4.07, cz],
            0.25,
            0.09,
            ch_id,
            2.0,
        );
    }

    // 5. ENCLOSING ROOM SHELL (drawn last so foreground objects occlude via Early-Z,
    // subdivided into 8x8 patches so per-vertex 12-speaker lighting captures smooth localized pools)
    let mut room_add_subdivided_quad = |p0: [f32; 3],
                                        p1: [f32; 3],
                                        p2: [f32; 3],
                                        p3: [f32; 3],
                                        normal: [f32; 3],
                                        ch: f32,
                                        mat: f32| {
        const SUB: usize = 8;
        let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        let bilerp = |u: f32, v: f32| -> [f32; 3] {
            let bot = lerp3(p0, p1, u);
            let top = lerp3(p3, p2, u);
            lerp3(bot, top, v)
        };
        for iv in 0..SUB {
            let v0 = (iv as f32) / (SUB as f32);
            let v1 = ((iv + 1) as f32) / (SUB as f32);
            for iu in 0..SUB {
                let u0 = (iu as f32) / (SUB as f32);
                let u1 = ((iu + 1) as f32) / (SUB as f32);
                room_add_quad(
                    &mut vertices,
                    &mut indices,
                    bilerp(u0, v0),
                    bilerp(u1, v0),
                    bilerp(u1, v1),
                    bilerp(u0, v1),
                    normal,
                    ch,
                    mat,
                );
            }
        }
    };

    // Floor (mat = 0.0)
    room_add_subdivided_quad(
        [-11.0, -1.6, 9.0],
        [11.0, -1.6, 9.0],
        [11.0, -1.6, -9.0],
        [-11.0, -1.6, -9.0],
        [0.0, 1.0, 0.0],
        0.0,
        0.0,
    );
    // Back acoustic wall (+Z, mat = 0.5)
    room_add_subdivided_quad(
        [-11.0, -1.6, 9.0],
        [-11.0, 5.0, 9.0],
        [11.0, 5.0, 9.0],
        [11.0, -1.6, 9.0],
        [0.0, 0.0, -1.0],
        0.0,
        0.5,
    );
    // Rear studio wall (-Z, mat = 0.5)
    room_add_subdivided_quad(
        [11.0, -1.6, -9.0],
        [11.0, 5.0, -9.0],
        [-11.0, 5.0, -9.0],
        [-11.0, -1.6, -9.0],
        [0.0, 0.0, 1.0],
        0.0,
        0.5,
    );
    // Left wall (-X, mat = 0.5)
    room_add_subdivided_quad(
        [-11.0, -1.6, -9.0],
        [-11.0, 5.0, -9.0],
        [-11.0, 5.0, 9.0],
        [-11.0, -1.6, 9.0],
        [1.0, 0.0, 0.0],
        0.0,
        0.5,
    );
    // Right wall (+X, mat = 0.5)
    room_add_subdivided_quad(
        [11.0, -1.6, 9.0],
        [11.0, 5.0, 9.0],
        [11.0, 5.0, -9.0],
        [11.0, -1.6, -9.0],
        [-1.0, 0.0, 0.0],
        0.0,
        0.5,
    );
    // Ceiling (+Y, mat = 0.5)
    room_add_subdivided_quad(
        [-11.0, 5.0, -9.0],
        [11.0, 5.0, -9.0],
        [11.0, 5.0, 9.0],
        [-11.0, 5.0, 9.0],
        [0.0, -1.0, 0.0],
        0.0,
        0.5,
    );

    (vertices, indices)
}

#[allow(clippy::too_many_arguments)]
fn glass_add_quad(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    normal: [f32; 3],
    ch: f32,
    mat: f32,
) {
    let start = vertices.len() as u32;
    vertices.push(Vertex {
        position: p0,
        normal,
        tex_coords: [ch, mat],
    });
    vertices.push(Vertex {
        position: p1,
        normal,
        tex_coords: [ch, mat],
    });
    vertices.push(Vertex {
        position: p2,
        normal,
        tex_coords: [ch, mat],
    });
    vertices.push(Vertex {
        position: p3,
        normal,
        tex_coords: [ch, mat],
    });
    indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

pub(in crate::engine) static EMBEDDED_GLASS_FONT: &[u8] =
    include_bytes!("../../../../assets/Orbitron-Black.ttf");

struct GlassGlyphContour {
    points: Vec<[f32; 2]>,
}

struct GlassContourBuilder {
    contours: Vec<GlassGlyphContour>,
    current_contour: Vec<[f32; 2]>,
    start_point: [f32; 2],
    last_point: [f32; 2],
}

impl GlassContourBuilder {
    fn new() -> Self {
        Self {
            contours: Vec::new(),
            current_contour: Vec::new(),
            start_point: [0.0, 0.0],
            last_point: [0.0, 0.0],
        }
    }

    fn finish(mut self) -> Vec<GlassGlyphContour> {
        if !self.current_contour.is_empty() {
            self.contours.push(GlassGlyphContour {
                points: self.current_contour,
            });
        }
        self.contours
    }
}

impl ttf_parser::OutlineBuilder for GlassContourBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        if !self.current_contour.is_empty() {
            self.contours.push(GlassGlyphContour {
                points: std::mem::take(&mut self.current_contour),
            });
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
            let cx = it * it * it * p0[0]
                + 3.0 * it * it * t * p1[0]
                + 3.0 * it * t * t * p2[0]
                + t * t * t * p3[0];
            let cy = it * it * it * p0[1]
                + 3.0 * it * it * t * p1[1]
                + 3.0 * it * t * t * p2[1]
                + t * t * t * p3[1];
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
            self.contours.push(GlassGlyphContour {
                points: std::mem::take(&mut self.current_contour),
            });
        }
    }
}

fn glass_contour_signed_area(pts: &[[f32; 2]]) -> f32 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
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
    if (p[0] - a[0]).abs() < 1e-4 && (p[1] - a[1]).abs() < 1e-4 {
        return false;
    }
    if (p[0] - b[0]).abs() < 1e-4 && (p[1] - b[1]).abs() < 1e-4 {
        return false;
    }
    if (p[0] - c[0]).abs() < 1e-4 && (p[1] - c[1]).abs() < 1e-4 {
        return false;
    }

    let cross1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let cross2 = (c[0] - b[0]) * (p[1] - b[1]) - (c[1] - b[1]) * (p[0] - b[0]);
    let cross3 = (a[0] - c[0]) * (p[1] - c[1]) - (a[1] - c[1]) * (p[0] - c[0]);

    cross1 > 1e-6 && cross2 > 1e-6 && cross3 > 1e-6
}

fn earcut_triangulate_polygon(
    outer: &[[f32; 2]],
    holes: &[Vec<[f32; 2]>],
) -> (Vec<[f32; 2]>, Vec<[usize; 3]>) {
    let mut ring: Vec<[f32; 2]> = outer.to_vec();
    if glass_contour_signed_area(&ring) < 0.0 {
        ring.reverse(); // Ensure CCW
    }

    let mut sorted_holes: Vec<Vec<[f32; 2]>> = holes.to_vec();
    for h in &mut sorted_holes {
        if glass_contour_signed_area(h) > 0.0 {
            h.reverse(); // Ensure CW for holes
        }
    }
    sorted_holes.sort_by(|a, b| {
        let max_a = a.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        let max_b = b.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        max_b
            .partial_cmp(&max_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for h in &sorted_holes {
        if h.len() < 3 {
            continue;
        }
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

fn glass_triangulate_contours(contours: &[Vec<[f32; 2]>]) -> (Vec<[f32; 2]>, Vec<[usize; 3]>) {
    let mut outers = Vec::new();
    let mut holes = Vec::new();

    for c in contours {
        if c.len() < 3 {
            continue;
        }
        let area = glass_contour_signed_area(c);
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
            .filter(|h| h.first().is_some_and(|p| point_in_polygon(*p, outer)))
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

pub fn generate_glass_lyrics_mesh(text: &str) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(16384);
    let mut indices = Vec::with_capacity(65536);

    let trimmed = text.trim();
    let display_str = if trimmed.is_empty() {
        "RUSTTRACKER"
    } else {
        trimmed
    };

    let face = ttf_parser::Face::parse(EMBEDDED_GLASS_FONT, 0).ok();

    if let Some(face) = face {
        let em = face.units_per_em() as f32;
        let base_scale = 1.0 / em;

        // First pass: compute total text width with typographic advances
        let mut total_advance = 0.0f32;
        for ch in display_str.chars() {
            if let Some(glyph_id) = face.glyph_index(ch) {
                let adv = face
                    .glyph_hor_advance(glyph_id)
                    .unwrap_or(face.units_per_em()) as f32
                    * base_scale;
                total_advance += adv;
            } else {
                total_advance += 0.5;
            }
        }

        let target_width = 7.5f32;
        let text_scale = (target_width / total_advance.max(1.0)).clamp(0.15, 1.25);
        let start_x = -total_advance * text_scale * 0.5;
        let baseline_y = 0.16; // Sits just above water level y = 0.0
        let depth = 0.22 * text_scale;
        let bevel = 0.024 * text_scale;
        let hz = depth * 0.5;

        let mut curr_x = start_x;

        for (ch_idx, ch) in display_str.chars().enumerate() {
            if let Some(glyph_id) = face.glyph_index(ch) {
                let adv = face
                    .glyph_hor_advance(glyph_id)
                    .unwrap_or(face.units_per_em()) as f32
                    * base_scale
                    * text_scale;

                let mut builder = GlassContourBuilder::new();
                if let Some(_bbox) = face.outline_glyph(glyph_id, &mut builder) {
                    let contours = builder.finish();

                    let scaled_contours: Vec<Vec<[f32; 2]>> = contours
                        .iter()
                        .filter(|c| c.points.len() >= 3)
                        .map(|c| {
                            c.points
                                .iter()
                                .map(|p| {
                                    [
                                        curr_x + p[0] * base_scale * text_scale,
                                        baseline_y + p[1] * base_scale * text_scale,
                                    ]
                                })
                                .collect()
                        })
                        .collect();

                    if !scaled_contours.is_empty() {
                        let (face_verts_2d, face_tris) =
                            glass_triangulate_contours(&scaled_contours);

                        // 1. Front and Back Faces
                        let start_front = vertices.len() as u32;
                        for p in &face_verts_2d {
                            vertices.push(Vertex {
                                position: [p[0], p[1], hz],
                                normal: [0.0, 0.0, 1.0],
                                tex_coords: [ch_idx as f32, 1.0], // Mat 1.0 = Glass
                            });
                        }
                        for tri in &face_tris {
                            indices.push(start_front + tri[0] as u32);
                            indices.push(start_front + tri[1] as u32);
                            indices.push(start_front + tri[2] as u32);
                        }

                        let start_back = vertices.len() as u32;
                        for p in &face_verts_2d {
                            vertices.push(Vertex {
                                position: [p[0], p[1], -hz],
                                normal: [0.0, 0.0, -1.0],
                                tex_coords: [ch_idx as f32, 1.0],
                            });
                        }
                        for tri in &face_tris {
                            indices.push(start_back + tri[0] as u32);
                            indices.push(start_back + tri[2] as u32);
                            indices.push(start_back + tri[1] as u32);
                        }

                        // 2. Extruded Sidewalls & 45 deg Bevel Chamfers
                        for pts in &scaled_contours {
                            let n_pts = pts.len();
                            let area = glass_contour_signed_area(pts);
                            let is_outer = area < 0.0;

                            for i in 0..n_pts {
                                let p0 = pts[i];
                                let p1 = pts[(i + 1) % n_pts];

                                let dx = p1[0] - p0[0];
                                let dy = p1[1] - p0[1];
                                let len = (dx * dx + dy * dy).sqrt();
                                if len < 1e-6 {
                                    continue;
                                }

                                let mut nx = dy / len;
                                let mut ny = -dx / len;
                                if !is_outer {
                                    nx = -nx;
                                    ny = -ny;
                                }

                                let start_v = vertices.len() as u32;

                                // Side quad
                                let norm_side = [nx, ny, 0.0];
                                vertices.push(Vertex {
                                    position: [p0[0], p0[1], -hz + bevel],
                                    normal: norm_side,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p1[0], p1[1], -hz + bevel],
                                    normal: norm_side,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p1[0], p1[1], hz - bevel],
                                    normal: norm_side,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p0[0], p0[1], hz - bevel],
                                    normal: norm_side,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                indices.extend_from_slice(&[
                                    start_v,
                                    start_v + 1,
                                    start_v + 2,
                                    start_v,
                                    start_v + 2,
                                    start_v + 3,
                                ]);

                                // Top Chamfer quad (+Z)
                                let start_c1 = vertices.len() as u32;
                                let norm_c1 = [
                                    nx * std::f32::consts::FRAC_1_SQRT_2,
                                    ny * std::f32::consts::FRAC_1_SQRT_2,
                                    std::f32::consts::FRAC_1_SQRT_2,
                                ];
                                vertices.push(Vertex {
                                    position: [p0[0], p0[1], hz - bevel],
                                    normal: norm_c1,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p1[0], p1[1], hz - bevel],
                                    normal: norm_c1,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p1[0] - nx * bevel, p1[1] - ny * bevel, hz],
                                    normal: norm_c1,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p0[0] - nx * bevel, p0[1] - ny * bevel, hz],
                                    normal: norm_c1,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                indices.extend_from_slice(&[
                                    start_c1,
                                    start_c1 + 1,
                                    start_c1 + 2,
                                    start_c1,
                                    start_c1 + 2,
                                    start_c1 + 3,
                                ]);

                                // Bot Chamfer quad (-Z)
                                let start_c2 = vertices.len() as u32;
                                let norm_c2 = [
                                    nx * std::f32::consts::FRAC_1_SQRT_2,
                                    ny * std::f32::consts::FRAC_1_SQRT_2,
                                    -std::f32::consts::FRAC_1_SQRT_2,
                                ];
                                vertices.push(Vertex {
                                    position: [p0[0] - nx * bevel, p0[1] - ny * bevel, -hz],
                                    normal: norm_c2,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p1[0] - nx * bevel, p1[1] - ny * bevel, -hz],
                                    normal: norm_c2,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p1[0], p1[1], -hz + bevel],
                                    normal: norm_c2,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                vertices.push(Vertex {
                                    position: [p0[0], p0[1], -hz + bevel],
                                    normal: norm_c2,
                                    tex_coords: [ch_idx as f32, 1.0],
                                });
                                indices.extend_from_slice(&[
                                    start_c2,
                                    start_c2 + 1,
                                    start_c2 + 2,
                                    start_c2,
                                    start_c2 + 2,
                                    start_c2 + 3,
                                ]);
                            }
                        }
                    }
                }
                curr_x += adv;
            } else {
                curr_x += 0.4 * text_scale;
            }
        }
    }

    // 2. Tessellated 3D Water Grid Plane: 128 x 128 vertices spanning [-16.0, 16.0] x [-10.0, 4.5]
    let grid_res_x = 128;
    let grid_res_z = 128;
    let start_water_vert = vertices.len() as u32;

    for iz in 0..grid_res_z {
        let fz = iz as f32 / (grid_res_z - 1) as f32;
        let z = -10.0 + fz * 14.5;
        for ix in 0..grid_res_x {
            let fx = ix as f32 / (grid_res_x - 1) as f32;
            let x = -16.0 + fx * 32.0;
            vertices.push(Vertex {
                position: [x, 0.0, z],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [0.0, 2.0], // Mat 2.0 = Water
            });
        }
    }

    for iz in 0..(grid_res_z - 1) {
        for ix in 0..(grid_res_x - 1) {
            let v0 = start_water_vert + iz * grid_res_x + ix;
            let v1 = v0 + 1;
            let v2 = v0 + grid_res_x;
            let v3 = v2 + 1;
            indices.extend_from_slice(&[v0, v2, v1, v1, v2, v3]);
        }
    }

    // 3. Overhead Softbox Area Light Emitter Quad: at y = 5.2, z = -1.5, tilted downward 35 deg
    let sb_center = [0.0f32, 5.2, -1.5];
    let sb_w = 16.0f32;
    let sb_h = 3.6f32;
    let tilt: f32 = 35.0f32.to_radians();
    let cos_t = tilt.cos();
    let sin_t = tilt.sin();

    let p0 = [
        sb_center[0] - sb_w * 0.5,
        sb_center[1] + sb_h * 0.5 * cos_t,
        sb_center[2] + sb_h * 0.5 * sin_t,
    ];
    let p1 = [
        sb_center[0] + sb_w * 0.5,
        sb_center[1] + sb_h * 0.5 * cos_t,
        sb_center[2] + sb_h * 0.5 * sin_t,
    ];
    let p2 = [
        sb_center[0] + sb_w * 0.5,
        sb_center[1] - sb_h * 0.5 * cos_t,
        sb_center[2] - sb_h * 0.5 * sin_t,
    ];
    let p3 = [
        sb_center[0] - sb_w * 0.5,
        sb_center[1] - sb_h * 0.5 * cos_t,
        sb_center[2] - sb_h * 0.5 * sin_t,
    ];
    let sb_norm = [0.0, -sin_t, cos_t];

    glass_add_quad(
        &mut vertices,
        &mut indices,
        p0,
        p1,
        p2,
        p3,
        sb_norm,
        0.0,
        3.0, // Mat 3.0 = Emissive Softbox
    );

    (vertices, indices)
}

#[allow(clippy::too_many_arguments)]
fn cyber_add_quad(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    normal: [f32; 3],
    mat: f32,
    uv_y: f32,
) {
    let start = vertices.len() as u32;
    vertices.push(Vertex {
        position: p0,
        normal,
        tex_coords: [mat, uv_y],
    });
    vertices.push(Vertex {
        position: p1,
        normal,
        tex_coords: [mat, uv_y],
    });
    vertices.push(Vertex {
        position: p2,
        normal,
        tex_coords: [mat, uv_y],
    });
    vertices.push(Vertex {
        position: p3,
        normal,
        tex_coords: [mat, uv_y],
    });
    indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

fn cyber_add_box(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    size: [f32; 3],
    rot_y: f32,
    mat: f32,
) {
    let hx = size[0] / 2.0;
    let hy = size[1] / 2.0;
    let hz = size[2] / 2.0;
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();

    let rotate_pt = |p: [f32; 3]| -> [f32; 3] {
        let rx = p[0] * cos_r + p[2] * sin_r;
        let rz = -p[0] * sin_r + p[2] * cos_r;
        [rx + center[0], p[1] + center[1], rz + center[2]]
    };
    let rotate_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    cyber_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, -hy, hz]),
        rotate_pt([hx, -hy, hz]),
        rotate_pt([hx, hy, hz]),
        rotate_pt([-hx, hy, hz]),
        rotate_norm([0.0, 0.0, 1.0]),
        mat,
        center[1] + hy,
    );
    cyber_add_quad(
        vertices,
        indices,
        rotate_pt([hx, -hy, -hz]),
        rotate_pt([-hx, -hy, -hz]),
        rotate_pt([-hx, hy, -hz]),
        rotate_pt([hx, hy, -hz]),
        rotate_norm([0.0, 0.0, -1.0]),
        mat,
        center[1] + hy,
    );
    cyber_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, -hy, -hz]),
        rotate_pt([-hx, -hy, hz]),
        rotate_pt([-hx, hy, hz]),
        rotate_pt([-hx, hy, -hz]),
        rotate_norm([-1.0, 0.0, 0.0]),
        mat,
        center[1] + hy,
    );
    cyber_add_quad(
        vertices,
        indices,
        rotate_pt([hx, -hy, hz]),
        rotate_pt([hx, -hy, -hz]),
        rotate_pt([hx, hy, -hz]),
        rotate_pt([hx, hy, hz]),
        rotate_norm([1.0, 0.0, 0.0]),
        mat,
        center[1] + hy,
    );
    cyber_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, hy, hz]),
        rotate_pt([hx, hy, hz]),
        rotate_pt([hx, hy, -hz]),
        rotate_pt([-hx, hy, -hz]),
        rotate_norm([0.0, 1.0, 0.0]),
        mat,
        center[1] + hy,
    );
    cyber_add_quad(
        vertices,
        indices,
        rotate_pt([-hx, -hy, -hz]),
        rotate_pt([hx, -hy, -hz]),
        rotate_pt([hx, -hy, hz]),
        rotate_pt([-hx, -hy, hz]),
        rotate_norm([0.0, -1.0, 0.0]),
        mat,
        center[1] - hy,
    );
}

struct ObjFaceTri {
    verts: [(usize, usize); 3],
    mat: f32,
}

struct ParsedObjTemplate {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tris: Vec<ObjFaceTri>,
}

fn parse_obj_template(obj_src: &str, default_mat: f32) -> ParsedObjTemplate {
    let mut raw_positions: Vec<[f32; 3]> = Vec::new();
    let mut raw_normals: Vec<[f32; 3]> = Vec::new();
    let mut tris = Vec::new();
    let mut current_mat = default_mat;

    for line in obj_src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("v ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3
                && let (Ok(x), Ok(y), Ok(z)) = (
                    parts[0].parse::<f32>(),
                    parts[1].parse::<f32>(),
                    parts[2].parse::<f32>(),
                )
            {
                raw_positions.push([x, y, z]);
            }
        } else if let Some(rest) = trimmed.strip_prefix("vn ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3
                && let (Ok(x), Ok(y), Ok(z)) = (
                    parts[0].parse::<f32>(),
                    parts[1].parse::<f32>(),
                    parts[2].parse::<f32>(),
                )
            {
                raw_normals.push([x, y, z]);
            }
        } else if let Some(rest) = trimmed.strip_prefix("g ") {
            let group_name = rest.trim();
            current_mat = match group_name {
                "floor" => 0.0,
                "neon_frame_0" => 1.0,
                "neon_frame_1" => 2.0,
                "neon_frame_2" => 3.0,
                "neon_frame_3" => 4.0,
                "neon_frame_4" => 5.0,
                "neon_frame_5" => 6.0,
                "neon_frame_6" => 7.0,
                "neon_frame_7" => 8.0,
                "faceplate" => 1.0,
                "meter_frame" => 2.0,
                "dial_scale" => 3.0,
                "needle_left" => 4.0,
                "needle_right" => 5.0,
                "knob" => 6.0,
                "meter_glass" => 7.0,
                "leds" => 8.0,
                "screws" => 9.0,
                "hull" => 1.0,
                "arch_rib" => 2.0,
                "floor_grating" => 3.0,
                "neon_strip" => 4.0,
                "conduit" => 5.0,
                "hazard_trim" => 6.0,
                "paint" => 3.0,
                "glass" => 4.0,
                "taillight" => 6.0,
                "tire" => 7.0,
                "rim" => 7.5,
                "carbon" => 8.0,
                "exhaust" => 9.0,
                "trunk" => 10.0,
                "frond" => 10.5,
                "mast" => 11.0,
                "lamp" => 11.5,
                "tower" => 12.0,
                "spire" => 12.5,
                _ => default_mat,
            };
        } else if let Some(rest) = trimmed.strip_prefix("f ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                let parse_vert = |tok: &str| -> Option<(usize, usize)> {
                    let segs: Vec<&str> = tok.split('/').collect();
                    let v_idx = segs[0].parse::<usize>().ok()?.checked_sub(1)?;
                    let vn_idx = if segs.len() >= 3 && !segs[2].is_empty() {
                        segs[2].parse::<usize>().ok()?.checked_sub(1)?
                    } else {
                        v_idx
                    };
                    Some((v_idx, vn_idx))
                };

                let face_verts: Vec<(usize, usize)> =
                    parts.iter().filter_map(|p| parse_vert(p)).collect();
                if face_verts.len() >= 3 {
                    for i in 1..face_verts.len() - 1 {
                        tris.push(ObjFaceTri {
                            verts: [face_verts[0], face_verts[i], face_verts[i + 1]],
                            mat: current_mat,
                        });
                    }
                }
            }
        }
    }

    ParsedObjTemplate {
        positions: raw_positions,
        normals: raw_normals,
        tris,
    }
}

fn instantiate_obj_template(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    template: &ParsedObjTemplate,
    translation: [f32; 3],
    scale: [f32; 3],
    rot_y: f32,
) {
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();
    let transform_pos = |p: [f32; 3]| -> [f32; 3] {
        let sx = p[0] * scale[0];
        let sy = p[1] * scale[1];
        let sz = p[2] * scale[2];
        let rx = sx * cos_r + sz * sin_r;
        let rz = -sx * sin_r + sz * cos_r;
        [
            rx + translation[0],
            sy + translation[1],
            rz + translation[2],
        ]
    };
    let transform_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    for tri in &template.tris {
        let start_idx = vertices.len() as u32;
        for &(v_i, vn_i) in &tri.verts {
            let pos = if v_i < template.positions.len() {
                template.positions[v_i]
            } else {
                [0.0, 0.0, 0.0]
            };
            let norm = if vn_i < template.normals.len() {
                template.normals[vn_i]
            } else {
                [0.0, 1.0, 0.0]
            };
            let world_p = transform_pos(pos);
            let world_n = transform_norm(norm);
            vertices.push(Vertex {
                position: world_p,
                normal: world_n,
                tex_coords: [tri.mat, translation[2]],
            });
        }
        indices.extend_from_slice(&[start_idx, start_idx + 1, start_idx + 2]);
    }
}

fn load_obj_mesh(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    obj_src: &str,
    translation: [f32; 3],
    scale: [f32; 3],
    rot_y: f32,
    default_mat: f32,
) {
    let template = parse_obj_template(obj_src, default_mat);
    instantiate_obj_template(vertices, indices, &template, translation, scale, rot_y);
}

pub(crate) fn generate_synthwave_racer_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(32768);
    let mut indices = Vec::with_capacity(49152);

    // 1. Continuous Desert / Terrain Ground Planes (mat = 0.5)
    let ground_w = 200.0;
    cyber_add_quad(
        &mut vertices,
        &mut indices,
        [-ground_w, -0.05, -10.0],
        [-9.0, -0.05, -10.0],
        [-9.0, -0.05, 360.0],
        [-ground_w, -0.05, 360.0],
        [0.0, 1.0, 0.0],
        0.5,
        0.0,
    );
    cyber_add_quad(
        &mut vertices,
        &mut indices,
        [9.0, -0.05, -10.0],
        [ground_w, -0.05, -10.0],
        [ground_w, -0.05, 360.0],
        [9.0, -0.05, 360.0],
        [0.0, 1.0, 0.0],
        0.5,
        0.0,
    );

    // 2. 3D Highway Roadbed, Curbs & Concrete K-Rails
    let road_half_w = 9.0;
    let seg_len = 3.5;
    let num_segs = 100;
    for i in 0..num_segs {
        let z0 = -8.0 + (i as f32) * seg_len;
        let z1 = z0 + seg_len;

        // Asphalt (mat = 0.0)
        cyber_add_quad(
            &mut vertices,
            &mut indices,
            [-road_half_w, 0.0, z0],
            [road_half_w, 0.0, z0],
            [road_half_w, 0.0, z1],
            [-road_half_w, 0.0, z1],
            [0.0, 1.0, 0.0],
            0.0,
            z0,
        );

        // Curbs (mat = 1.0)
        cyber_add_quad(
            &mut vertices,
            &mut indices,
            [-road_half_w - 0.8, 0.15, z0],
            [-road_half_w, 0.0, z0],
            [-road_half_w, 0.0, z1],
            [-road_half_w - 0.8, 0.15, z1],
            [0.2, 0.98, 0.0],
            1.0,
            z0,
        );
        cyber_add_quad(
            &mut vertices,
            &mut indices,
            [road_half_w, 0.0, z0],
            [road_half_w + 0.8, 0.15, z0],
            [road_half_w + 0.8, 0.15, z1],
            [road_half_w, 0.0, z1],
            [-0.2, 0.98, 0.0],
            1.0,
            z0,
        );

        // Concrete K-Rail Barriers (mat = 2.0)
        cyber_add_box(
            &mut vertices,
            &mut indices,
            [-road_half_w - 1.2, 0.45, (z0 + z1) / 2.0],
            [0.35, 0.75, seg_len],
            0.0,
            2.0,
        );
        cyber_add_box(
            &mut vertices,
            &mut indices,
            [road_half_w + 1.2, 0.45, (z0 + z1) / 2.0],
            [0.35, 0.75, seg_len],
            0.0,
            2.0,
        );
    }

    // 3. Embedded 3D Low-Poly Supercar Model (.OBJ)
    load_obj_mesh(
        &mut vertices,
        &mut indices,
        include_str!("../../../../src/assets/models/supercar_f40.obj"),
        [0.0, 0.40, 5.2],
        [1.0, 1.0, 1.0],
        0.0,
        3.0,
    );

    // 3b. Solid Supercar Underbody Floor Pan & Interior Cabin Firewall (mat = 8.0 carbon)
    cyber_add_box(
        &mut vertices,
        &mut indices,
        [0.0, 0.35, 5.2],
        [1.70, 0.04, 3.8],
        0.0,
        8.0,
    );
    cyber_add_box(
        &mut vertices,
        &mut indices,
        [0.0, 0.44, 4.8],
        [1.30, 0.16, 1.8],
        0.0,
        8.0,
    );

    // 4. Roadside Streetlamps & Palm Trees (.OBJ Meshes)
    let palm_template = parse_obj_template(
        include_str!("../../../../src/assets/models/palm_tree.obj"),
        10.0,
    );
    let streetlamp_template = parse_obj_template(
        include_str!("../../../../src/assets/models/streetlamp.obj"),
        11.0,
    );
    for i in 0..12 {
        let pz = 18.0 + (i as f32) * 26.0;
        // Left Palm Tree
        instantiate_obj_template(
            &mut vertices,
            &mut indices,
            &palm_template,
            [-road_half_w - 3.8, 0.0, pz],
            [1.0, 1.0, 1.0],
            (i as f32) * 0.8,
        );
        // Right Cobra-Head Streetlamp
        instantiate_obj_template(
            &mut vertices,
            &mut indices,
            &streetlamp_template,
            [road_half_w + 2.4, 0.0, pz + 13.0],
            [1.0, 1.0, 1.0],
            std::f32::consts::PI,
        );
    }

    // 5. Distant Horizon Skyscrapers (.OBJ Meshes)
    let skyscraper_template = parse_obj_template(
        include_str!("../../../../src/assets/models/skyscraper.obj"),
        12.0,
    );
    for i in 0..16 {
        let ang = (i as f32) * 0.38 - 3.0;
        let tx = ang * 46.0;
        let tz = 260.0 + ((i * 7) % 5) as f32 * 14.0;
        let s = 0.75 + ((i * 3) % 4) as f32 * 0.15;
        instantiate_obj_template(
            &mut vertices,
            &mut indices,
            &skyscraper_template,
            [tx, -2.0, tz],
            [s, s, s],
            (i as f32) * 0.5,
        );
    }

    (vertices, indices)
}

pub(crate) fn generate_vumeter_rack_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    load_obj_mesh(
        &mut vertices,
        &mut indices,
        include_str!("../../../../src/assets/models/vumeter_rack.obj"),
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        0.0,
        1.0,
    );
    (vertices, indices)
}
