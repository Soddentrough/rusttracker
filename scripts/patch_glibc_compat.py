#!/usr/bin/env python3
"""
Downgrades GLIBC_2.44 symbol version requirements (specifically C23 sinh/cosh in libm.so.6
introduced on Fedora 45 / glibc 2.44) to GLIBC_2.2.5 in ELF binaries inside an AppDir so the
packaged AppImage runs natively on SteamOS (Steam Deck, glibc 2.43) and older Linux distros.
"""

import os
import struct
import sys

SHT_GNU_VERNEED = 0x6FFFFFFE
SHT_GNU_VERSYM = 0x6FFFFFFF


def read_cstr(buf: bytes, offset: int) -> bytes:
    end = buf.find(b"\x00", offset)
    return buf[offset:end] if end != -1 else b""


def patch_elf_glibc244(path: str) -> bool:
    try:
        with open(path, "rb") as f:
            data = bytearray(f.read())
    except OSError:
        return False

    if len(data) < 64 or data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        return False

    e_shoff, _, _, _, _, e_shentsize, e_shnum, _ = struct.unpack_from(
        "<Q I H H H H H H", data, 40
    )
    if e_shoff == 0 or e_shnum == 0 or e_shentsize < 64:
        return False

    sections = []
    verneed_sec = None
    versym_sec = None

    for i in range(e_shnum):
        sh_pos = e_shoff + i * e_shentsize
        if sh_pos + 64 > len(data):
            return False
        sh = struct.unpack_from("<I I Q Q Q Q I I Q Q", data, sh_pos)
        sections.append(sh)
        sh_type = sh[1]
        if sh_type == SHT_GNU_VERNEED:
            verneed_sec = sh
        elif sh_type == SHT_GNU_VERSYM:
            versym_sec = sh

    if verneed_sec is None or versym_sec is None:
        return False

    vn_offset = verneed_sec[4]
    vn_size = verneed_sec[5]
    dynstr_idx = verneed_sec[6]
    vn_cnt_total = verneed_sec[7]

    if dynstr_idx >= len(sections):
        return False
    dynstr_sec = sections[dynstr_idx]
    dynstr_off = dynstr_sec[4]
    dynstr_end = dynstr_off + dynstr_sec[5]
    dynstr = bytes(data[dynstr_off:dynstr_end])

    ver_remap = {}
    pos = vn_offset
    for _ in range(vn_cnt_total):
        if pos + 16 > vn_offset + vn_size:
            break
        vn_version, vn_cnt, vn_file, vn_aux, vn_next = struct.unpack_from(
            "<H H I I I", data, pos
        )
        aux_pos = pos + vn_aux
        aux_entries = []
        for _ in range(vn_cnt):
            if aux_pos + 16 > vn_offset + vn_size:
                break
            vna_hash, vna_flags, vna_other, vna_name, vna_next = struct.unpack_from(
                "<I H H I I", data, aux_pos
            )
            ver_name = read_cstr(dynstr, vna_name)
            aux_entries.append(
                (aux_pos, vna_hash, vna_flags, vna_other, vna_name, vna_next, ver_name)
            )
            if vna_next == 0:
                break
            aux_pos += vna_next

        fallback = None
        for entry in aux_entries:
            if entry[6] == b"GLIBC_2.2.5":
                fallback = entry
                break

        if fallback is not None:
            _, fb_hash, fb_flags, fb_other, fb_name, _, _ = fallback
            for entry in aux_entries:
                e_pos, _, _, e_other, _, e_next, e_name = entry
                if e_name == b"GLIBC_2.44":
                    ver_remap[e_other & 0x7FFF] = fb_other & 0x7FFF
                    struct.pack_into(
                        "<I H H I I",
                        data,
                        e_pos,
                        fb_hash,
                        fb_flags,
                        fb_other,
                        fb_name,
                        e_next,
                    )

        if vn_next == 0:
            break
        pos += vn_next

    if not ver_remap:
        return False

    vs_offset = versym_sec[4]
    vs_size = versym_sec[5]
    num_syms = vs_size // 2
    for i in range(num_syms):
        p = vs_offset + i * 2
        val = struct.unpack_from("<H", data, p)[0]
        base_ver = val & 0x7FFF
        if base_ver in ver_remap:
            new_val = (val & 0x8000) | ver_remap[base_ver]
            struct.pack_into("<H", data, p, new_val)

    with open(path, "wb") as f:
        f.write(data)
    return True


def main() -> None:
    patched_count = 0
    for target in sys.argv[1:]:
        if os.path.isdir(target):
            for root, _, files in os.walk(target):
                for fn in files:
                    full = os.path.join(root, fn)
                    if not os.path.islink(full) and patch_elf_glibc244(full):
                        print(f"[glibc-compat] Patched GLIBC_2.44 -> GLIBC_2.2.5 in {full}")
                        patched_count += 1
        elif os.path.isfile(target) and not os.path.islink(target):
            if patch_elf_glibc244(target):
                print(f"[glibc-compat] Patched GLIBC_2.44 -> GLIBC_2.2.5 in {target}")
                patched_count += 1
    print(f"[glibc-compat] Total ELF binaries patched: {patched_count}")


if __name__ == "__main__":
    main()
