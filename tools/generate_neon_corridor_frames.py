#!/usr/bin/env python3
"""
Generate 3D Wavefront .OBJ model for Visualizer 5 (Neon Corridor / Neon Room):
True 3D concentric audio-reactive neon portal frames with dark polished reflective floor.
"""

import os
import math

def main():
    output_dir = "/home/naoki/Development/rusttracker/src/assets/models"
    os.makedirs(output_dir, exist_ok=True)
    obj_path = os.path.join(output_dir, "neon_corridor_frames.obj")

    verts = []
    normals = []
    groups = []

    def add_quad(p0, p1, p2, p3, norm=None):
        nonlocal verts, normals, groups
        v_start = len(verts) + 1
        if norm is None:
            u = [p1[i] - p0[i] for i in range(3)]
            v = [p3[i] - p0[i] for i in range(3)]
            nx = u[1]*v[2] - u[2]*v[1]
            ny = u[2]*v[0] - u[0]*v[2]
            nz = u[0]*v[1] - u[1]*v[0]
            l = math.sqrt(nx*nx + ny*ny + nz*nz) or 1.0
            norm = [nx/l, ny/l, nz/l]
        
        for p in [p0, p1, p2, p3]:
            verts.append(p)
            normals.append(norm)
        
        groups[-1][1].append((v_start, v_start+1, v_start+2))
        groups[-1][1].append((v_start, v_start+2, v_start+3))

    def add_box(min_p, max_p):
        x0, y0, z0 = min_p
        x1, y1, z1 = max_p
        add_quad([x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1], [0, 0, 1])
        add_quad([x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [0, 0, -1])
        add_quad([x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0], [0, 1, 0])
        add_quad([x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1], [0, -1, 0])
        add_quad([x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [1, 0, 0])
        add_quad([x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [-1, 0, 0])

    def add_neon_frame(z_center, width=5.4, height=3.8, tube_r=0.07):
        hw = width / 2.0
        # Left post
        add_box([-hw - tube_r, 0.0, z_center - tube_r], [-hw + tube_r, height, z_center + tube_r])
        # Right post
        add_box([hw - tube_r, 0.0, z_center - tube_r], [hw + tube_r, height, z_center + tube_r])
        # Top lintel
        add_box([-hw - tube_r, height - tube_r*2, z_center - tube_r], [hw + tube_r, height, z_center + tube_r])
        # Bottom threshold
        add_box([-hw - tube_r, 0.0, z_center - tube_r], [hw + tube_r, tube_r*2, z_center + tube_r])

    # 1. Dark Reflective Polished Floor Plane (mat = 0.0)
    groups.append(("floor", []))
    add_quad([-25.0, 0.0, -10.0], [25.0, 0.0, -10.0], [25.0, 0.0, 80.0], [-25.0, 0.0, 80.0], [0, 1, 0])

    # 2. 8 Audio-Channel Reactive Neon Portal Frames
    z_positions = [2.0, 5.5, 9.0, 12.5, 16.0, 19.5, 23.0, 26.5]
    for i, z in enumerate(z_positions):
        groups.append((f"neon_frame_{i}", []))
        add_neon_frame(z, width=5.6, height=4.0, tube_r=0.07)

    with open(obj_path, 'w') as f:
        f.write("# True 3D Concentric Neon Audio Channel Portal Frames OBJ Model\n")
        for v in verts:
            f.write(f"v {v[0]:.4f} {v[1]:.4f} {v[2]:.4f}\n")
        for n in normals:
            f.write(f"vn {n[0]:.4f} {n[1]:.4f} {n[2]:.4f}\n")
        for g_name, tris in groups:
            f.write(f"g {g_name}\n")
            for t in tris:
                f.write(f"f {t[0]}//{t[0]} {t[1]}//{t[1]} {t[2]}//{t[2]}\n")

    print(f"Generated {obj_path} ({len(verts)} vertices, {sum(len(t[1]) for t in groups)} triangles)")

if __name__ == "__main__":
    main()
