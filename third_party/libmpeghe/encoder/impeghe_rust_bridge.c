/* Rust FFI Bridge for libmpeghe (MPEG-H 3D Audio LC Profile Encoder) */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "impeghe_type_def.h"
#include "impeghe_api.h"
#include "impeghe_error_standards.h"
#include "impeghe_apicmd_standards.h"
#include "impeghe_block_switch_const.h"
#include "impeghe_drc_common.h"
#include "impeghe_drc_uni_drc.h"
#include "impeghe_drc_api.h"
#include "impeghe_dmx_cicp2geometry.h"
#include "impeghe_dmx_matrix_common.h"
#include "impeghe_memory_standards.h"
#include "impeghe_error_codes.h"

static pVOID rust_mpegh_malloc_global(UWORD32 size, UWORD32 alignment) {
    return calloc(1, (size_t)size + (size_t)alignment + 64);
}

typedef struct rust_mpegh_enc_ctx {
    ia_mpeghe_config_struct cfg;
    pVOID api_obj;
    int32_t *pb_inp_buf;
    uint8_t *pb_out_buf;
    uint8_t raw_asc[512];
    int32_t raw_asc_len;
} rust_mpegh_enc_ctx;

rust_mpegh_enc_ctx *rust_mpegh_enc_create(
    int32_t num_channels,
    int32_t sample_rate,
    int32_t cicp_index,
    int32_t bitrate
) {
    rust_mpegh_enc_ctx *ctx = (rust_mpegh_enc_ctx *)calloc(1, sizeof(rust_mpegh_enc_ctx));
    if (!ctx) {
        return NULL;
    }

    ia_input_config *in_cfg = &ctx->cfg.input_config;
    ia_output_config *out_cfg = &ctx->cfg.output_config;

    in_cfg->aud_ch_pcm_cfg.n_channels = num_channels;
    in_cfg->aud_ch_pcm_cfg.sample_rate = sample_rate;
    in_cfg->aud_ch_pcm_cfg.pcm_sz = 32;
    in_cfg->cicp_index = cicp_index;
    in_cfg->codec_mode = 1; /* USAC_ONLY_FD */
    in_cfg->bitrate = bitrate;
    in_cfg->fdp_enable = 0;
    in_cfg->noise_filling = 0;
    in_cfg->enhanced_noise_filling = 1;
    in_cfg->igf_after_tns_synth = 1;
    in_cfg->tns_enable = 1;
    in_cfg->cplx_pred = 0;
    in_cfg->fill_elem = 1;
    in_cfg->prof_level = 2; /* PROFILE_LC_LVL3 (supports up to 12ch / 7.1.4) */
    in_cfg->out_fmt = 2;    /* Raw mpegh3daConfig / mpegh3daFrame */
    in_cfg->mhas_pkt = 0;
    in_cfg->mct_mode = -1;
    in_cfg->use_vec_est = (UWORD32)-1;

    out_cfg->malloc_xaac = &rust_mpegh_malloc_global;

    IA_ERRORCODE err = impeghe_create(in_cfg, out_cfg);
    if (err != IA_NO_ERROR) {
        impeghe_delete(out_cfg);
        free(ctx);
        return NULL;
    }

    ctx->api_obj = out_cfg->pv_ia_process_api_obj;
    ctx->pb_inp_buf = (int32_t *)out_cfg->mem_info_table[IA_MEMTYPE_INPUT].mem_ptr;
    ctx->pb_out_buf = (uint8_t *)out_cfg->mem_info_table[IA_MEMTYPE_OUTPUT].mem_ptr;

    if (!ctx->api_obj || !ctx->pb_inp_buf || !ctx->pb_out_buf) {
        impeghe_delete(out_cfg);
        free(ctx);
        return NULL;
    }

    err = impeghe_init(ctx->api_obj, in_cfg, out_cfg);
    if (err != IA_NO_ERROR || out_cfg->i_dec_len <= 0 || out_cfg->i_dec_len > (WORD32)sizeof(ctx->raw_asc)) {
        impeghe_delete(out_cfg);
        free(ctx);
        return NULL;
    }

    ctx->raw_asc_len = (int32_t)out_cfg->i_dec_len;
    memcpy(ctx->raw_asc, ctx->pb_out_buf, (size_t)ctx->raw_asc_len);
    return ctx;
}

int32_t rust_mpegh_enc_get_asc(
    const rust_mpegh_enc_ctx *ctx,
    const uint8_t **out_asc_ptr
) {
    if (!ctx || !out_asc_ptr) {
        return 0;
    }
    *out_asc_ptr = ctx->raw_asc;
    return ctx->raw_asc_len;
}

int32_t rust_mpegh_enc_get_frame_length(const rust_mpegh_enc_ctx *ctx) {
    if (!ctx || ctx->cfg.output_config.in_frame_length <= 0) {
        return 1024;
    }
    return (int32_t)ctx->cfg.output_config.in_frame_length;
}

int32_t *rust_mpegh_enc_get_input_pcm_buf(rust_mpegh_enc_ctx *ctx) {
    if (!ctx) {
        return NULL;
    }
    return ctx->pb_inp_buf;
}

int32_t rust_mpegh_enc_encode_frame(
    rust_mpegh_enc_ctx *ctx,
    const uint8_t **out_frame_ptr,
    int32_t *out_is_rap
) {
    if (!ctx || !out_frame_ptr || !out_is_rap) {
        return -1;
    }
    IA_ERRORCODE err = impeghe_execute(ctx->api_obj, &ctx->cfg.input_config, &ctx->cfg.output_config);
    if (err != IA_NO_ERROR) {
        return -1;
    }
    int32_t out_bytes = (int32_t)ctx->cfg.output_config.i_out_bytes;
    if (out_bytes <= 0) {
        return 0;
    }
    *out_frame_ptr = ctx->pb_out_buf;
    /* In mpegh3daFrame, the MSB of byte 0 is usacIndependencyFlag (1 = RAP / I-frame) */
    *out_is_rap = ((ctx->pb_out_buf[0] & 0x80u) != 0u) ? 1 : 0;
    return out_bytes;
}

void rust_mpegh_enc_destroy(rust_mpegh_enc_ctx *ctx) {
    if (!ctx) {
        return;
    }
    impeghe_delete(&ctx->cfg.output_config);
    free(ctx);
}
