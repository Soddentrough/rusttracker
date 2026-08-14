#!/usr/bin/env python3
import os

def patch_file(path, replacements):
    if not os.path.exists(path):
        return
    with open(path, 'r', encoding='utf-8', errors='ignore') as f:
        content = f.read()
    for old, new in replacements:
        content = content.replace(old, new)
    with open(path, 'w', encoding='utf-8') as f:
        f.write(content)

if __name__ == '__main__':
    # Patch frame.h
    patch_file('/mingw64/include/libavutil/frame.h', [
        ('AV_FRAME_DATA_DYNAMIC_HDR_SMPTE_2094_APP5,', ''),
        ('AV_FRAME_DATA_IAMF_MIX_GAIN_PARAM,', ''),
        ('AV_FRAME_DATA_IAMF_DEMIXING_INFO_PARAM,', ''),
        ('AV_FRAME_DATA_IAMF_RECON_GAIN_INFO_PARAM,', ''),
        ('AV_FRAME_DATA_RAW_COLOR_PARAMS,', ''),
    ])

    # Patch packet.h
    patch_file('/mingw64/include/libavcodec/packet.h', [
        ('AV_PKT_DATA_DYNAMIC_HDR_SMPTE_2094_APP5,', ''),
        ('AV_PKT_DATA_HEVC_CONF,', ''),
    ])

    # Patch codec.h and avcodec.h
    avcodec_fields = '''uint8_t max_lowres;
    const AVRational *supported_framerates;
    const enum AVPixelFormat *pix_fmts;
    const int *supported_samplerates;
    const enum AVSampleFormat *sample_fmts;
    const struct AVChannelLayout *ch_layouts;'''

    patch_file('/mingw64/include/libavcodec/codec.h', [
        ('uint8_t max_lowres;', avcodec_fields)
    ])
    patch_file('/mingw64/include/libavcodec/avcodec.h', [
        ('uint8_t max_lowres;', avcodec_fields)
    ])

    # Patch codec_id.h
    codec_id_insert = '''AV_CODEC_ID_V210,
    AV_CODEC_ID_V410,
    AV_CODEC_ID_V308,
    AV_CODEC_ID_V408,'''

    patch_file('/mingw64/include/libavcodec/codec_id.h', [
        ('AV_CODEC_ID_V410,', ''),
        ('AV_CODEC_ID_V308,', ''),
        ('AV_CODEC_ID_V408,', ''),
        ('AV_CODEC_ID_V210,', codec_id_insert)
    ])
