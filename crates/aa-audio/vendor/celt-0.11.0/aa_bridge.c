/* Private aa-audio bridge. Upstream CELT license is in COPYING and each source. */
#include "celt.h"
#include <stdlib.h>

typedef struct {
    CELTMode *mode;
    CELTDecoder *decoder;
} AaCelt;

void aa_celt011_destroy(AaCelt *handle) {
    if (!handle) return;
    if (handle->decoder) celt_decoder_destroy(handle->decoder);
    if (handle->mode) celt_mode_destroy(handle->mode);
    free(handle);
}

AaCelt *aa_celt011_create(void) {
    int error = CELT_OK;
    celt_int32 version = 0;
    AaCelt *handle = (AaCelt *)calloc(1, sizeof(AaCelt));
    if (!handle) return NULL;
    handle->mode = celt_mode_create(44100, 512, &error);
    if (!handle->mode || error != CELT_OK) goto failure;
    error = celt_mode_info(handle->mode, CELT_GET_BITSTREAM_VERSION, &version);
    if (error != CELT_OK || (celt_uint32)version != 0x80000010U) goto failure;
    handle->decoder = celt_decoder_create_custom(handle->mode, 1, &error);
    if (!handle->decoder || error != CELT_OK) goto failure;
    return handle;
failure:
    aa_celt011_destroy(handle);
    return NULL;
}

int aa_celt011_decode(AaCelt *handle, const unsigned char *data, int length,
                     celt_int16 *pcm, size_t pcm_samples) {
    /* Mirror Rust's bounds at the FFI boundary; never enter packet-loss synthesis. */
    if (!handle || !handle->decoder || !data || length < 2 || length > 512 ||
        !pcm || pcm_samples != 512) return CELT_BAD_ARG;
    return celt_decode(handle->decoder, data, length, pcm, 512);
}
