#!/usr/bin/env python3
import os

def filter_lines(path, bad_words):
    if not os.path.exists(path):
        return
    with open(path, 'r', encoding='utf-8', errors='ignore') as f:
        lines = f.readlines()
    new_lines = []
    for line in lines:
        if any(w in line for w in bad_words):
            continue
        new_lines.append(line)
    with open(path, 'w', encoding='utf-8') as f:
        f.writelines(new_lines)

def insert_after(path, trigger, insertion):
    if not os.path.exists(path):
        return
    with open(path, 'r', encoding='utf-8', errors='ignore') as f:
        content = f.read()
    if trigger in content and insertion not in content:
        content = content.replace(trigger, trigger + '\n' + insertion, 1)
        with open(path, 'w', encoding='utf-8') as f:
            f.write(content)

if __name__ == '__main__':
    # 1. Patch frame.h: remove unsupported development side data enums
    filter_lines('/mingw64/include/libavutil/frame.h', [
        'AV_FRAME_DATA_DYNAMIC_HDR_SMPTE_2094_APP5',
        'AV_FRAME_DATA_IAMF_',
        'AV_FRAME_DATA_RAW_COLOR_PARAMS',
    ])

    # 2. Patch packet.h: remove unsupported development packet side data enums
    filter_lines('/mingw64/include/libavcodec/packet.h', [
        'AV_PKT_DATA_DYNAMIC_HDR_SMPTE_2094_APP5',
        'AV_PKT_DATA_HEVC_CONF',
    ])

    # 3. Patch codec.h & avcodec.h: restore legacy AVCodec fields
    avcodec_fields = '''    const AVRational *supported_framerates;
    const enum AVPixelFormat *pix_fmts;
    const int *supported_samplerates;
    const enum AVSampleFormat *sample_fmts;
    const struct AVChannelLayout *ch_layouts;'''

    insert_after('/mingw64/include/libavcodec/codec.h', 'uint8_t max_lowres;', avcodec_fields)
    insert_after('/mingw64/include/libavcodec/avcodec.h', 'uint8_t max_lowres;', avcodec_fields)

    # 4. Patch codec_id.h: ensure legacy codec IDs exist exactly once
    filter_lines('/mingw64/include/libavcodec/codec_id.h', [
        'AV_CODEC_ID_V410',
        'AV_CODEC_ID_V308',
        'AV_CODEC_ID_V408',
    ])
    insert_after('/mingw64/include/libavcodec/codec_id.h', 'AV_CODEC_ID_V210,', '''    AV_CODEC_ID_V410,
    AV_CODEC_ID_V308,
    AV_CODEC_ID_V408,''')
