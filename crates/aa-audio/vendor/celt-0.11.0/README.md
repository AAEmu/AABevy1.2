# Private legacy CELT 0.11.0 decoder

Upstream: Xiph CELT, tag **v0.11**, version 0.11.0 in `configure.ac`, commit
`0b405d1170122c859faab435405666506d52fa2e`.

Archive: https://gitlab.xiph.org/xiph/celt/-/archive/v0.11/celt-v0.11.tar.gz

SHA-256: `4fa104630f9bab52a5009cfd2c7a3b7431c8f5876a2704643caef1c09705e0a0`
(209281 bytes, fetched 2026-10-23). Use the commit/archive hash, not a later
Mumble branch or modern Opus CELT implementation. The original game's sole CELT
bank and FMOD 4.44.14 decoder use bitstream `0x80000010` and 44100/512 mono.

`COPYING` and the retained per-file notices govern these sources (BSD-style
permissions, with the additional notice in `float_cast.h`). `FILES.json` records
the upstream path and SHA-256 before and after local changes for each retained
upstream file. `aa-celt.patch` records every change to upstream files.
`aa_bridge.c` is project-owned glue under the aa-audio package license.

The retained subset is 16 C units and 24 headers plus COPYING. Omitted: Ogg
headers/container code, CLI/tools, tests, encoder programs, fixed-point and
platform DSP implementations, and unused standard-mode table sources.
`AA_CELT_DECODER_ONLY` excludes the encoder state, API and analysis routines in
`celt.c`. Some entropy-encoder and shared quantization helpers remain because
upstream's band/allocation routines share encode/decode branches; removing them
would require a much broader codec fork. The Rust bridge exposes only creation,
one-frame decode and destruction. `AA_CELT_STATIC` disables DLL export attributes.

The build uses floating-point custom modes, no Opus build or experimental
postfilter, and `USE_ALLOCA` for bounded per-call scratch instead of the old
mutable global scratch stack. No codec object or mode is shared across streams.
The wrapper rejects short/null packets before upstream's packet-loss synthesis.
The fixed-mode input/output bounds are independently checked in Rust and C.

Compatibility review: vgmstream selects CELT 0.11.0/custom modes for FSB5,
44100/512, persistent per-stream state, 32-bit framed lengths and the fixed sync.
Its [ecintrin.h portability replacement](https://github.com/vgmstream/vgmstream/blob/master/ext_libs/extra/celt-0110/ecintrin.h)
uses the same `_BitScanReverse` MSVC branch as this pinned release; its fallback
change is unnecessary on the checked MSVC target. Its dual-version symbol
renaming is needed for coexistence with 0.6.1; this private build includes only
0.11.0. No vgmstream decoder source, modern Opus decoder, FMOD binary, game PCM
or binary table is vendored.

The original instructions use a precomputed custom mode and x87 arithmetic.
This source constructs the same custom geometry and uses the host compiler's
floating-point arithmetic. Quantified actual-asset error against the original
instruction golden is documented in `docs/aa-audio.md`; do not claim bit-exact
output without that comparison.
