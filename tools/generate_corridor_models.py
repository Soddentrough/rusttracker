#!/usr/bin/env python3
"""
Generate 3D Wavefront .OBJ model for Visualizer 5 (Neon Corridor):
Modular Sci-Fi Industrial Tunnel Segment with Conduit Ribs & Neon Light Bars.
"""

import os
import math

def main():
    output_dir = "/home/naoki/Development/rusttracker/src/assets/models"
    os.makedirs(output_dir, exist_ok=True)
    obj_path = os.path.join(output_dir, "corridor_segment.obj")

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

    def add_cylinder(cx, cy, cz, radius, length, segments=12, axis='z'):
        nonlocal verts, normals, groups
        v_start = len(verts) + 1
        half_l = length / 2.0
        for i in range(segments):
            a0 = (i / segments) * 2.0 * math.pi
            a1 = ((i + 1) / segments) * 2.0 * math.pi
            c0, s0 = math.cos(a0), math.sin(a0)
            c1, s1 = math.cos(a1), math.sin(a1)
            if axis == 'z':
                p0 = [cx + radius*c0, cy + radius*s0, cz - half_l]
                p1 = [cx + radius*c1, cy + radius*s1, cz - half_l]
                p2 = [cx + radius*c1, cy + radius*s1, cz + half_l]
                p3 = [cx + radius*c0, cy + radius*s0, cz + half_l]
                n = [(c0+c1)*0.5, (s0+s1)*0.5, 0]
                add_quad(p0, p1, p2, p3, n)

    # Segment length: 4.0 (z from -2.0 to 2.0)
    z_len = 4.0
    hw = 3.5 # Half-width
    hh = 3.0 # Half-height

    # 1. Structural Hull & Wall Panels
    groups.append(("hull", []))
    # Left Wall
    add_box([-hw - 0.2, -0.2, -z_len/2], [-hw, hh*1.2, z_len/2])
    # Right Wall
    add_box([hw, -0.2, -z_len/2], [hw + 0.2, hh*1.2, z_len/2])
    # Ceiling
    add_box([-hw - 0.2, hh*1.2, -z_len/2], [hw + 0.2, hh*1.2 + 0.2, z_len/2])
    # Lower Subfloor Foundation
    add_box([-hw - 0.2, -0.6, -z_len/2], [hw + 0.2, -0.2, z_len/2])

    # 2. Heavy Bulkhead Arches & Portal Ribs
    groups.append(("arch_rib", []))
    # Front Arch Rib at z = 1.6
    add_box([-hw - 0.05, 0.0, 1.5], [-hw + 0.4, hh*1.15, 1.9])
    add_box([hw - 0.4, 0.0, 1.5], [hw + 0.05, hh*1.15, 1.9])
    add_box([-hw, hh*1.0, 1.5], [hw, hh*1.15, 1.9])
    # Angled Corner Gussets
    add_quad([-hw + 0.4, hh*0.7, 1.9], [-hw + 1.2, hh*1.0, 1.9],
             [-hw + 1.2, hh*1.0, 1.5], [-hw + 0.4, hh*0.7, 1.5])
    add_quad([hw - 1.2, hh*1.0, 1.9], [hw - 0.4, hh*0.7, 1.9],
             [hw - 0.4, hh*0.7, 1.5], [hw - 1.2, hh*1.0, 1.5])

    # 3. Metallic Floor Grating (Walking Surface)
    groups.append(("floor_grating", []))
    add_quad([-hw*0.75, 0.0, -z_len/2], [hw*0.75, 0.0, -z_len/2],
             [hw*0.75, 0.0, z_len/2], [-hw*0.75, 0.0, z_len/2], [0, 1, 0])

    # 4. Continuous Neon Light Rails (Ceiling & Floor Kickers)
    groups.append(("neon_strip", []))
    # Top-Left Neon Strip
    add_box([-hw*0.90, hh*1.08, -z_len/2], [-hw*0.82, hh*1.14, z_len/2])
    # Top-Right Neon Strip
    add_box([hw*0.82, hh*1.08, -z_len/2], [hw*0.90, hh*1.14, z_len/2])
    # Bottom-Left Kicker Strip
    add_box([-hw*0.78, 0.02, -z_len/2], [-hw*0.72, 0.08, z_len/2])
    # Bottom-Right Kicker Strip
    add_box([hw*0.72, 0.02, -z_len/2], [hw*0.78, 0.08, z_len/2])

    # 5. Overhead Industrial Conduits & Pipes
    groups.append(("conduit", []))
    add_cylinder(-1.2, hh*1.05, 0.0, 0.12, z_len, 10, 'z')
    add_cylinder(1.2, hh*1.05, 0.0, 0.12, z_len, 10, 'z')
    add_cylinder(0.0, hh*1.10, 0.0, 0.18, z_len, 12, 'z')

    # 6. Industrial Hazard Warning Trim
    groups.append(("hazard_trim", []))
    add_box([-hw*0.78, 0.0, -z_len/2], [-hw*0.75, 0.02, z_len/2])
    add_box([hw*0.75, 0.0, -z_len/2], [hw*0.78, 0.02, z_len/2])

    with open(obj_path, 'w') as f:
        f.write("# Sci-Fi Modular Neon Corridor OBJ Segment\n")
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
