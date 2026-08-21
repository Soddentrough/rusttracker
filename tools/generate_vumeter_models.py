#!/usr/bin/env python3
"""
Generate 3D Wavefront .OBJ model for Visualizer 18 (Retro VU Meters):
Vintage 19" 3U Rack-Mount Analog Dual VU Meter Studio Console with Cutout Cavities.
"""

import os
import math

def main():
    output_dir = "/home/naoki/Development/rusttracker/src/assets/models"
    os.makedirs(output_dir, exist_ok=True)
    obj_path = os.path.join(output_dir, "vumeter_rack.obj")

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
        add_quad([x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y0, z1], [-1, 0, 0])

    def add_cylinder(cx, cy, cz, radius, height, segments=16, axis='z'):
        nonlocal verts, normals, groups
        v_start = len(verts) + 1
        half_h = height / 2.0
        
        for i in range(segments):
            a0 = (i / segments) * 2.0 * math.pi
            a1 = ((i + 1) / segments) * 2.0 * math.pi
            c0, s0 = math.cos(a0), math.sin(a0)
            c1, s1 = math.cos(a1), math.sin(a1)
            
            if axis == 'z':
                p0 = [cx + radius*c0, cy + radius*s0, cz - half_h]
                p1 = [cx + radius*c1, cy + radius*s1, cz - half_h]
                p2 = [cx + radius*c1, cy + radius*s1, cz + half_h]
                p3 = [cx + radius*c0, cy + radius*s0, cz + half_h]
                n = [(c0+c1)*0.5, (s0+s1)*0.5, 0]
                add_quad(p0, p1, p2, p3, n)
                add_quad([cx, cy, cz + half_h], [cx + radius*c0, cy + radius*s0, cz + half_h],
                         [cx + radius*c1, cy + radius*s1, cz + half_h], [cx, cy, cz + half_h], [0, 0, 1])

    # 1. Main Rack Chassis & Brushed Aluminum Faceplate (Surrounding Meter Cutouts)
    groups.append(("faceplate", []))
    # Top bar above meters
    add_box([-5.5, 1.45, -1.5], [5.5, 2.0, 0.0])
    # Bottom bar below meters
    add_box([-5.5, -2.0, -1.5], [5.5, -1.35, 0.0])
    # Center pillar between meters
    add_box([-0.55, -1.35, -1.5], [0.55, 1.45, 0.0])
    # Left outer flank
    add_box([-5.5, -1.35, -1.5], [-4.85, 1.45, 0.0])
    # Right outer flank
    add_box([4.85, -1.35, -1.5], [5.5, 1.45, 0.0])
    # Left and Right rack mounting ears
    add_box([-6.0, -2.0, -0.05], [-5.5, 2.0, 0.05])
    add_box([5.5, -2.0, -0.05], [6.0, 2.0, 0.05])

    # 2. Black Recessed Meter Cavity Frames & Inward Sidewalls
    groups.append(("meter_frame", []))
    depth = 0.35
    for mx in [-2.70, 2.70]:
        w = 2.15
        # Top cavity bevel wall
        add_quad([mx - w, 1.45, 0.0], [mx + w, 1.45, 0.0], [mx + w, 1.45, -depth], [mx - w, 1.45, -depth])
        # Bottom cavity bevel wall
        add_quad([mx - w, -1.35, -depth], [mx + w, -1.35, -depth], [mx + w, -1.35, 0.0], [mx - w, -1.35, 0.0])
        # Left cavity bevel wall
        add_quad([mx - w, -1.35, 0.0], [mx - w, 1.45, 0.0], [mx - w, 1.45, -depth], [mx - w, -1.35, -depth])
        # Right cavity bevel wall
        add_quad([mx + w, -1.35, -depth], [mx + w, 1.45, -depth], [mx + w, 1.45, 0.0], [mx + w, -1.35, 0.0])

    # 3. Vintage Cream Parchment Meter Dial Scales (Backplane)
    groups.append(("dial_scale", []))
    for mx in [-2.70, 2.70]:
        w = 2.15
        add_quad([mx - w, -1.35, -depth + 0.02],
                 [mx + w, -1.35, -depth + 0.02],
                 [mx + w, 1.45, -depth + 0.02],
                 [mx - w, 1.45, -depth + 0.02], [0, 0, 1])

    # 4. Needle 1 (Left Channel Needle)
    groups.append(("needle_left", []))
    lx = -2.70
    ly = -1.15 # Needle pivot base
    add_quad([lx - 0.02, ly, -depth + 0.08], [lx + 0.02, ly, -depth + 0.08],
             [lx + 0.006, ly + 2.35, -depth + 0.08], [lx - 0.006, ly + 2.35, -depth + 0.08], [0, 0, 1])
    add_cylinder(lx, ly, -depth + 0.10, 0.14, 0.05, 12, 'z')

    # 5. Needle 2 (Right Channel Needle)
    groups.append(("needle_right", []))
    rx = 2.70
    ry = -1.15
    add_quad([rx - 0.02, ry, -depth + 0.08], [rx + 0.02, ry, -depth + 0.08],
             [rx + 0.006, ry + 2.35, -depth + 0.08], [rx - 0.006, ry + 2.35, -depth + 0.08], [0, 0, 1])
    add_cylinder(rx, ry, -depth + 0.10, 0.14, 0.05, 12, 'z')

    # 6. Machined Aluminum Knobs and Power Switch
    groups.append(("knob", []))
    add_cylinder(0.0, -0.65, 0.15, 0.50, 0.28, 20, 'z')
    add_cylinder(-5.15, -1.55, 0.08, 0.10, 0.22, 10, 'z')

    # 7. LED Indicators (Power, Left Peak, Right Peak)
    groups.append(("leds", []))
    add_cylinder(-5.15, -0.85, 0.04, 0.08, 0.06, 12, 'z')
    add_cylinder(-0.95, 1.25, 0.02, 0.07, 0.04, 10, 'z')
    add_cylinder(4.45, 1.25, 0.02, 0.07, 0.04, 10, 'z')

    # 8. Mounting Screws (4 Rack Screws)
    groups.append(("screws", []))
    for sx in [-5.75, 5.75]:
        for sy in [-1.6, 1.6]:
            add_cylinder(sx, sy, 0.06, 0.14, 0.04, 10, 'z')

    with open(obj_path, 'w') as f:
        f.write("# Vintage Hi-Fi 3U Analog VU Meter Rack OBJ Model\n")
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
