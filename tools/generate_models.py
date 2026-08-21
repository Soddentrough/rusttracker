#!/usr/bin/env python3
"""
3D Low-Poly Mesh Generator for RustTracker (.OBJ Exporter)
Generates clean, topologically sound Wavefront .OBJ models:
- supercar_f40.obj: Low-poly 1980s sports car with curved wheel arches, raked canopy, rear louvers, and F40 wing.
- streetlamp.obj: Curved cobra-head highway lamppost.
- palm_tree.obj: Segmented curved trunk with radiating fan fronds.
- skyscraper.obj: Multi-tier setback art-deco skyscraper with rooftop spire.
"""

import math
import os

def normalize(v):
    l = math.sqrt(v[0]*v[0] + v[1]*v[1] + v[2]*v[2])
    if l < 1e-6:
        return [0.0, 1.0, 0.0]
    return [v[0]/l, v[1]/l, v[2]/l]

def cross(a, b):
    return [
        a[1]*b[2] - a[2]*b[1],
        a[2]*b[0] - a[0]*b[2],
        a[0]*b[1] - a[1]*b[0]
    ]

def sub(a, b):
    return [a[0]-b[0], a[1]-b[1], a[2]-b[2]]

class ObjMesh:
    def __init__(self, name):
        self.name = name
        self.vertices = []
        self.normals = []
        self.groups = {} # group_name -> list of faces (each face is list of vertex indices 0-based)
        self.current_group = "default"

    def set_group(self, name):
        self.current_group = name
        if name not in self.groups:
            self.groups[name] = []

    def add_vertex(self, x, y, z):
        self.vertices.append([float(x), float(y), float(z)])
        return len(self.vertices) - 1

    def add_quad(self, v0, v1, v2, v3):
        if self.current_group not in self.groups:
            self.groups[self.current_group] = []
        self.groups[self.current_group].append([v0, v1, v2])
        self.groups[self.current_group].append([v0, v2, v3])

    def add_tri(self, v0, v1, v2):
        if self.current_group not in self.groups:
            self.groups[self.current_group] = []
        self.groups[self.current_group].append([v0, v1, v2])

    def add_box(self, center, size):
        cx, cy, cz = center
        sx, sy, sz = size[0]*0.5, size[1]*0.5, size[2]*0.5
        
        # 8 vertices
        v0 = self.add_vertex(cx - sx, cy - sy, cz - sz)
        v1 = self.add_vertex(cx + sx, cy - sy, cz - sz)
        v2 = self.add_vertex(cx + sx, cy + sy, cz - sz)
        v3 = self.add_vertex(cx - sx, cy + sy, cz - sz)
        v4 = self.add_vertex(cx - sx, cy - sy, cz + sz)
        v5 = self.add_vertex(cx + sx, cy - sy, cz + sz)
        v6 = self.add_vertex(cx + sx, cy + sy, cz + sz)
        v7 = self.add_vertex(cx - sx, cy + sy, cz + sz)

        # 6 faces (counter-clockwise)
        self.add_quad(v4, v5, v6, v7) # Front (+z)
        self.add_quad(v1, v0, v3, v2) # Back (-z)
        self.add_quad(v0, v4, v7, v3) # Left (-x)
        self.add_quad(v5, v1, v2, v6) # Right (+x)
        self.add_quad(v7, v6, v2, v3) # Top (+y)
        self.add_quad(v0, v1, v5, v4) # Bottom (-y)

    def compute_normals(self):
        # Calculate vertex normals with area-weighted face normal accumulation
        vert_normals = [[0.0, 0.0, 0.0] for _ in range(len(self.vertices))]
        
        for g_name, faces in self.groups.items():
            for face in faces:
                p0 = self.vertices[face[0]]
                p1 = self.vertices[face[1]]
                p2 = self.vertices[face[2]]
                fn = cross(sub(p1, p0), sub(p2, p0))
                for v_idx in face:
                    vert_normals[v_idx][0] += fn[0]
                    vert_normals[v_idx][1] += fn[1]
                    vert_normals[v_idx][2] += fn[2]
        
        self.normals = [normalize(vn) for vn in vert_normals]

    def export_obj(self, filepath):
        self.compute_normals()
        with open(filepath, "w") as f:
            f.write(f"# Wavefront OBJ: {self.name}\n")
            f.write(f"# Exported by RustTracker Toolchain\n\n")
            
            for v in self.vertices:
                f.write(f"v {v[0]:.6f} {v[1]:.6f} {v[2]:.6f}\n")
            f.write("\n")
            
            for n in self.normals:
                f.write(f"vn {n[0]:.6f} {n[1]:.6f} {n[2]:.6f}\n")
            f.write("\n")
            
            for g_name, faces in self.groups.items():
                f.write(f"g {g_name}\n")
                for face in faces:
                    f.write(f"f {face[0]+1}//{face[0]+1} {face[1]+1}//{face[1]+1} {face[2]+1}//{face[2]+1}\n")
                f.write("\n")
        print(f"Generated {filepath}: {len(self.vertices)} vertices, {sum(len(f) for f in self.groups.values())} triangles across {len(self.groups)} groups.")


def generate_supercar():
    mesh = ObjMesh("Supercar F40 / Countach")
    
    # -------------------------------------------------------------
    # 1. BODY SHELL (Corsa Red Gloss Paint)
    # -------------------------------------------------------------
    mesh.set_group("paint")

    # Front Wedge Hood & Nose Cone
    v_nose_c = mesh.add_vertex(0.0, -0.04, 2.15)
    v_nose_l = mesh.add_vertex(-0.85, -0.04, 2.05)
    v_nose_r = mesh.add_vertex(0.85, -0.04, 2.05)
    
    v_hood_c = mesh.add_vertex(0.0, 0.16, 0.75)
    v_hood_l = mesh.add_vertex(-0.90, 0.15, 0.75)
    v_hood_r = mesh.add_vertex(0.90, 0.15, 0.75)

    mesh.add_tri(v_nose_c, v_hood_c, v_hood_l)
    mesh.add_tri(v_nose_c, v_hood_l, v_nose_l)
    mesh.add_tri(v_nose_c, v_hood_r, v_hood_c)
    mesh.add_tri(v_nose_c, v_nose_r, v_hood_r)

    # Front Fenders / Wheel Arch Brows
    v_fender_fl_l = mesh.add_vertex(-1.02, 0.08, 1.85)
    v_fender_fl_t = mesh.add_vertex(-1.05, 0.22, 1.15)
    v_fender_fl_r = mesh.add_vertex(-1.02, 0.10, 0.75)
    
    mesh.add_tri(v_nose_l, v_hood_l, v_fender_fl_l)
    mesh.add_tri(v_fender_fl_l, v_hood_l, v_fender_fl_t)
    mesh.add_tri(v_hood_l, v_fender_fl_r, v_fender_fl_t)

    v_fender_fr_r = mesh.add_vertex(1.02, 0.08, 1.85)
    v_fender_fr_t = mesh.add_vertex(1.05, 0.22, 1.15)
    v_fender_fr_l = mesh.add_vertex(1.02, 0.10, 0.75)

    mesh.add_tri(v_nose_r, v_fender_fr_r, v_hood_r)
    mesh.add_tri(v_fender_fr_r, v_fender_fr_t, v_hood_r)
    mesh.add_tri(v_hood_r, v_fender_fr_t, v_fender_fr_l)

    # Roof & Greenhouse Shell
    v_roof_fl = mesh.add_vertex(-0.68, 0.48, 0.05)
    v_roof_fr = mesh.add_vertex(0.68, 0.48, 0.05)
    v_roof_rl = mesh.add_vertex(-0.70, 0.48, -0.48)
    v_roof_rr = mesh.add_vertex(0.70, 0.48, -0.48)
    mesh.add_quad(v_roof_fl, v_roof_fr, v_roof_rr, v_roof_rl)

    # Rear Flared Haunches
    v_haunch_l_f = mesh.add_vertex(-1.06, 0.22, -0.40)
    v_haunch_l_t = mesh.add_vertex(-1.08, 0.24, -1.05)
    v_haunch_l_r = mesh.add_vertex(-1.04, 0.18, -1.82)

    v_haunch_r_f = mesh.add_vertex(1.06, 0.22, -0.40)
    v_haunch_r_t = mesh.add_vertex(1.08, 0.24, -1.05)
    v_haunch_r_r = mesh.add_vertex(1.04, 0.18, -1.82)

    v_deck_c = mesh.add_vertex(0.0, 0.18, -1.35)
    v_deck_l = mesh.add_vertex(-0.80, 0.18, -1.35)
    v_deck_r = mesh.add_vertex(0.80, 0.18, -1.35)

    mesh.add_quad(v_fender_fl_r, v_haunch_l_f, v_deck_l, v_hood_l)
    mesh.add_quad(v_hood_r, v_deck_r, v_haunch_r_f, v_fender_fr_l)
    mesh.add_tri(v_deck_l, v_haunch_l_f, v_haunch_l_t)
    mesh.add_tri(v_deck_r, v_haunch_r_t, v_haunch_r_f)

    # Rear Tail Quarter Panels
    v_tail_tl = mesh.add_vertex(-0.98, 0.16, -1.85)
    v_tail_tr = mesh.add_vertex(0.98, 0.16, -1.85)
    v_tail_bl = mesh.add_vertex(-0.95, -0.04, -1.85)
    v_tail_br = mesh.add_vertex(0.95, -0.04, -1.85)

    mesh.add_tri(v_haunch_l_t, v_haunch_l_r, v_tail_tl)
    mesh.add_tri(v_haunch_r_t, v_tail_tr, v_haunch_r_r)

    # F40 High-Mount Integrated Rear Wing
    mesh.add_box([-1.00, 0.32, -1.78], [0.06, 0.38, 0.24]) # Left pylon
    mesh.add_box([1.00, 0.32, -1.78], [0.06, 0.38, 0.24])  # Right pylon
    mesh.add_box([0.0, 0.50, -1.80], [2.25, 0.04, 0.32])   # Wing blade
    mesh.add_box([-1.14, 0.50, -1.80], [0.03, 0.18, 0.36]) # Left endplate
    mesh.add_box([1.14, 0.50, -1.80], [0.03, 0.18, 0.36])  # Right endplate

    # -------------------------------------------------------------
    # 2. GLASS CANOPY & WINDOWS
    # -------------------------------------------------------------
    mesh.set_group("glass")
    mesh.add_quad(v_hood_l, v_hood_r, v_roof_fr, v_roof_fl)
    mesh.add_tri(v_hood_l, v_roof_fl, v_roof_rl)
    mesh.add_tri(v_hood_l, v_roof_rl, v_deck_l)
    mesh.add_tri(v_hood_r, v_roof_rr, v_roof_fr)
    mesh.add_tri(v_hood_r, v_deck_r, v_roof_rr)

    # -------------------------------------------------------------
    # 3. CARBON FIBER (Louvers, Splitter, Rear Diffuser)
    # -------------------------------------------------------------
    mesh.set_group("carbon")
    mesh.add_quad(v_roof_rl, v_roof_rr, v_deck_r, v_deck_l)
    mesh.add_box([0.0, -0.09, 2.18], [2.05, 0.04, 0.28])
    mesh.add_box([-1.05, -0.06, 0.0], [0.08, 0.06, 3.70])
    mesh.add_box([1.05, -0.06, 0.0], [0.08, 0.06, 3.70])
    mesh.add_quad(v_tail_tl, v_tail_tr, v_tail_br, v_tail_bl)
    mesh.add_box([0.0, -0.07, -1.84], [1.94, 0.06, 0.20])

    # -------------------------------------------------------------
    # 4. GLOWING RED TAILLIGHTS (Quad Circular Clusters)
    # -------------------------------------------------------------
    mesh.set_group("taillight")
    mesh.add_box([-0.72, 0.09, -1.87], [0.20, 0.12, 0.03])
    mesh.add_box([-0.42, 0.09, -1.87], [0.20, 0.12, 0.03])
    mesh.add_box([0.42, 0.09, -1.87], [0.20, 0.12, 0.03])
    mesh.add_box([0.72, 0.09, -1.87], [0.20, 0.12, 0.03])

    # -------------------------------------------------------------
    # 5. DUAL NITROUS EXHAUST TIPS
    # -------------------------------------------------------------
    mesh.set_group("exhaust")
    mesh.add_box([-0.26, -0.03, -1.90], [0.12, 0.08, 0.14])
    mesh.add_box([0.26, -0.03, -1.90], [0.12, 0.08, 0.14])

    # -------------------------------------------------------------
    # 6. WHEELS & 5-SPOKE RIMS
    # -------------------------------------------------------------
    def add_wheel(cx, cy, cz, radius, width):
        mesh.set_group("tire")
        segs = 16
        step = 2.0 * math.pi / segs
        hw = width * 0.5
        
        # Treads
        for i in range(segs):
            a0 = i * step
            a1 = (i + 1) * step
            y0 = cy + math.sin(a0) * radius
            z0 = cz + math.cos(a0) * radius
            y1 = cy + math.sin(a1) * radius
            z1 = cz + math.cos(a1) * radius

            v0 = mesh.add_vertex(cx - hw, y0, z0)
            v1 = mesh.add_vertex(cx + hw, y0, z0)
            v2 = mesh.add_vertex(cx + hw, y1, z1)
            v3 = mesh.add_vertex(cx - hw, y1, z1)
            mesh.add_quad(v0, v1, v2, v3)

        # Outer Rim Star
        mesh.set_group("rim")
        outer_x = cx + hw if cx > 0 else cx - hw
        v_center = mesh.add_vertex(outer_x, cy, cz)
        for i in range(segs):
            a0 = i * step
            a1 = (i + 1) * step
            p0 = mesh.add_vertex(outer_x, cy + math.sin(a0) * radius * 0.85, cz + math.cos(a0) * radius * 0.85)
            p1 = mesh.add_vertex(outer_x, cy + math.sin(a1) * radius * 0.85, cz + math.cos(a1) * radius * 0.85)
            if cx > 0:
                mesh.add_tri(v_center, p0, p1)
            else:
                mesh.add_tri(v_center, p1, p0)

    add_wheel(-1.00, -0.06, 1.15, 0.30, 0.22)
    add_wheel(1.00, -0.06, 1.15, 0.30, 0.22)
    add_wheel(-1.04, -0.04, -1.05, 0.34, 0.26)
    add_wheel(1.04, -0.04, -1.05, 0.34, 0.26)

    return mesh


def generate_streetlamp():
    mesh = ObjMesh("Cobra-Head Highway Streetlamp")
    
    # Base and Vertical Mast
    mesh.set_group("mast")
    mesh.add_box([0.0, 3.5, 0.0], [0.35, 7.0, 0.35])
    
    # Curved Upper Armature
    v0 = mesh.add_vertex(-0.15, 7.0, -0.15)
    v1 = mesh.add_vertex(0.15, 7.0, -0.15)
    v2 = mesh.add_vertex(0.15, 7.0, 0.15)
    v3 = mesh.add_vertex(-0.15, 7.0, 0.15)

    v4 = mesh.add_vertex(-1.8, 8.2, -0.12)
    v5 = mesh.add_vertex(-1.5, 8.2, -0.12)
    v6 = mesh.add_vertex(-1.5, 8.2, 0.12)
    v7 = mesh.add_vertex(-1.8, 8.2, 0.12)

    mesh.add_quad(v0, v1, v5, v4)
    mesh.add_quad(v1, v2, v6, v5)
    mesh.add_quad(v2, v3, v7, v6)
    mesh.add_quad(v3, v0, v4, v7)

    # Downward Angled Cobra Cowl / Glowing Lamp Fixture
    mesh.set_group("lamp")
    mesh.add_box([-2.2, 8.1, 0.0], [1.10, 0.18, 0.55])

    return mesh


def generate_palm_tree():
    mesh = ObjMesh("Roadside Palm Tree")
    
    # Curved Segmented Trunk
    mesh.set_group("trunk")
    segs = 6
    h_step = 1.0
    for i in range(segs):
        y0 = i * h_step
        y1 = (i + 1) * h_step
        x0 = math.sin(i * 0.25) * 0.35
        x1 = math.sin((i + 1) * 0.25) * 0.35
        mesh.add_box([(x0 + x1)*0.5, (y0 + y1)*0.5, 0.0], [0.42 - i*0.04, h_step, 0.42 - i*0.04])

    # Radiating Arced Palm Fronds (8 leaves)
    mesh.set_group("frond")
    crown_y = segs * h_step
    crown_x = math.sin(segs * 0.25) * 0.35
    num_fronds = 8
    frond_len = 3.6
    for i in range(num_fronds):
        ang = i * (2.0 * math.pi / num_fronds)
        dx = math.cos(ang) * frond_len
        dz = math.sin(ang) * frond_len
        
        v0 = mesh.add_vertex(crown_x, crown_y, 0.0)
        v1 = mesh.add_vertex(crown_x + dx*0.4, crown_y + 0.45, dz*0.4)
        v2 = mesh.add_vertex(crown_x + dx, crown_y - 0.50, dz)
        v3 = mesh.add_vertex(crown_x + dx*0.5, crown_y - 0.20, dz*0.5)

        mesh.add_tri(v0, v1, v3)
        mesh.add_tri(v1, v2, v3)

    return mesh


def generate_skyscraper():
    mesh = ObjMesh("Art-Deco Cyberpunk Skyscraper")
    
    # Stepped Terraced Towers
    mesh.set_group("tower")
    mesh.add_box([0.0, 22.0, 0.0], [18.0, 44.0, 18.0])   # Base tier
    mesh.add_box([0.0, 52.0, 0.0], [14.0, 16.0, 14.0])   # Mid tier
    mesh.add_box([0.0, 66.0, 0.0], [10.0, 12.0, 10.0])   # Upper tier
    mesh.add_box([0.0, 75.0, 0.0], [6.0, 6.0, 6.0])      # Crown tier

    # Rooftop Aviation Beacon Spire
    mesh.set_group("spire")
    mesh.add_box([0.0, 84.0, 0.0], [0.6, 12.0, 0.6])

    return mesh


if __name__ == "__main__":
    out_dir = os.path.join(os.path.dirname(__file__), "..", "src", "assets", "models")
    os.makedirs(out_dir, exist_ok=True)
    
    generate_supercar().export_obj(os.path.join(out_dir, "supercar_f40.obj"))
    generate_streetlamp().export_obj(os.path.join(out_dir, "streetlamp.obj"))
    generate_palm_tree().export_obj(os.path.join(out_dir, "palm_tree.obj"))
    generate_skyscraper().export_obj(os.path.join(out_dir, "skyscraper.obj"))
    print("All 3D .OBJ models successfully generated!")
