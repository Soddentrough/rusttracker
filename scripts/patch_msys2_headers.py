#!/usr/bin/env python3
import sys
import os

def find_include_dir():
    if len(sys.argv) > 1 and os.path.isdir(sys.argv[1]):
        return sys.argv[1]
    candidates = [
        os.environ.get('MINGW_PREFIX', '') + '/include',
        os.environ.get('MSYSTEM_PREFIX', '') + '/include',
        '/mingw64/include',
        'C:/msys64/mingw64/include',
        'D:/msys64/mingw64/include',
    ]
    temp_dir = 'D:/a/_temp'
    if os.path.exists(temp_dir):
        for root, dirs, files in os.walk(temp_dir):
            if root.endswith('mingw64/include') or root.endswith('mingw64\\include'):
                candidates.append(root)
    for c in candidates:
        if c and os.path.isdir(c) and os.path.exists(os.path.join(c, 'libavutil', 'frame.h')):
            return c
    return None

def filter_lines(path, bad_words):
    if not os.path.exists(path):
        print(f"File not found: {path}")
        return
    with open(path, 'r', encoding='utf-8', errors='ignore') as f:
        lines = f.readlines()
    new_lines = [line for line in lines if not any(w in line for w in bad_words)]
    with open(path, 'w', encoding='utf-8') as f:
        f.writelines(new_lines)
    print(f"Patched {os.path.basename(path)}: {len(lines)} -> {len(new_lines)} lines")

def insert_after(path, trigger, insertion):
    if not os.path.exists(path):
        print(f"File not found: {path}")
        return
    with open(path, 'r', encoding='utf-8', errors='ignore') as f:
        content = f.read()
    if trigger in content and insertion not in content:
        content = content.replace(trigger, trigger + '\n' + insertion, 1)
        with open(path, 'w', encoding='utf-8') as f:
            f.write(content)
        print(f"Inserted fields into {os.path.basename(path)}")
    else:
        print(f"Skipped {os.path.basename(path)} (trigger not found or already inserted)")

if __name__ == '__main__':
    inc_dir = find_include_dir()
    print(f"Resolved include directory: {inc_dir}")
    if not inc_dir:
        print("ERROR: Could not locate FFmpeg include directory!")
        sys.exit(1)

    # 1. Patch frame.h: remove unsupported development side data enums
    filter_lines(os.path.join(inc_dir, 'libavutil', 'frame.h'), [
        'AV_FRAME_DATA_DYNAMIC_HDR_SMPTE_2094_APP5',
        'AV_FRAME_DATA_IAMF_',
        'AV_FRAME_DATA_RAW_COLOR_PARAMS',
    ])

    # 2. Patch packet.h: remove unsupported development packet side data enums
    filter_lines(os.path.join(inc_dir, 'libavcodec', 'packet.h'), [
        'AV_PKT_DATA_DYNAMIC_HDR_SMPTE_2094_APP5',
        'AV_PKT_DATA_HEVC_CONF',
    ])

    # 3. Patch codec.h & avcodec.h: restore legacy AVCodec fields
    avcodec_fields = '''    const AVRational *supported_framerates;
    const enum AVPixelFormat *pix_fmts;
    const int *supported_samplerates;
    const enum AVSampleFormat *sample_fmts;
    const struct AVChannelLayout *ch_layouts;'''

    insert_after(os.path.join(inc_dir, 'libavcodec', 'codec.h'), 'uint8_t max_lowres;', avcodec_fields)
    insert_after(os.path.join(inc_dir, 'libavcodec', 'avcodec.h'), 'uint8_t max_lowres;', avcodec_fields)

    # 4. Patch codec_id.h: ensure legacy codec IDs exist and remove unreleased new codec IDs
    filter_lines(os.path.join(inc_dir, 'libavcodec', 'codec_id.h'), [
        'AV_CODEC_ID_V410',
        'AV_CODEC_ID_V308',
        'AV_CODEC_ID_V408',
        'AV_CODEC_ID_WEBP_ANIM',
        'AV_CODEC_ID_APPLE_APAC',
    ])
    insert_after(os.path.join(inc_dir, 'libavcodec', 'codec_id.h'), 'AV_CODEC_ID_V210,', '''    AV_CODEC_ID_V410,
    AV_CODEC_ID_V308,
    AV_CODEC_ID_V408,''')
