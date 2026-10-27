# aa-bake: a compressed game pak

`aa-bake` turns the 24.5 GB `game_pak` into a smaller archive that `aa-pak` opens through the same
API (`AA_PAK=<baked>/game.aapk`). It is the first step towards streaming the game to a browser.
This work lives on the `bake-pak` branch.

```text
cargo build --release -p aa-bake            # then copy target/release/aa-bake.exe aside
aa-bake bake <baked>/game.aapk 19       # lossless, ~13 min on 12 threads
aa-bake verify <baked>/game.aapk        # every file equals the original
aa-bake bake <baked>/game-1k.aapk 19 --max-texture 1024   # lossy tier
aa-bake stats | dedup | dds <mb> | dict <ext> <mb> | ddsinfo   # the measurements below
```

## Format

`[file data][index, zstd][footer]`, see `crates/aa-pak/src/baked.rs`. Every file is compressed on
its own, so one file is still one positioned read. Files that zstd cannot shrink by 3% (FSB sound
banks) are stored as is, and `Pak::read_at` reads ranges of them directly (`aa-audio` streams
banks that way). Identical files share one blob.

## Lossless result: 24.50 GB -> 9.17 GB

| Step | Saves |
|---|---|
| Drop files the remake never reads: `.bai` AI paths (1.09 GB), `.avi` machinima (0.91), CryEngine shader cache (0.75), `bin32/` (0.18), the encrypted client db (0.13) | 3.06 GB |
| Store identical files once (32,981 files, mostly terrain `.ctc`) | 1.55 GB raw |
| zstd level 19 per file | the rest |
| DDS: regroup BC blocks into endpoint and index runs before zstd | ~6% of DDS |

| Type | Raw GB | Baked GB | Ratio |
|---|---|---|---|
| dds | 8.83 | 4.36 | 49% |
| chr | 4.74 | 1.92 | 41% |
| ctc | 2.39 | 0.63 | 26% |
| cgf | 1.81 | 0.69 | 38% |
| dat | 1.50 | 0.34 | 23% |
| fsb | 0.76 | 0.72 | 95% (stored) |
| caf | 0.64 | 0.31 | 48% |

Checked: `verify` (all 180,672 files byte-identical), the login screen screenshot is pixel-identical,
and `aa-world --example fly` at the Nuian spawn matches (only animated water differs); same FPS.
Decoding runs at ~260 MB/s per thread (C zstd).

Tried and not worth it: zstd dictionaries per type (1-2%), zstd level 3 instead of 19 (12.8 GB).

## Where the rest is

Textures are already BC1/BC3/BC5 (DXT1/DXT5/ATI2), so lossless gains are spent. Texture bytes by
size: 2K 2.5 GB, 1K 3.5 GB, 512 2.2 GB raw. Further steps, biggest first:

1. `--max-texture 1024` (done, lossy, non-UI only): 2,110 textures lose their 2K level, 1.43 GB
   raw, but only **9.17 -> 8.71 GB** baked, because the 2K textures were the most compressible
   ones. The Nuian spawn looks the same at 1080p. Not worth the quality loss on its own.
2. Rate-distortion re-encoding of BC blocks (reuse earlier blocks' index bits when the error is
   small) so zstd finds more matches: typically 20-40% off textures at near-invisible cost.
3. Meshes (`.chr` 1.9 GB, `.cgf` 0.7 GB baked): a pre-parsed mesh format with quantised vertices
   and meshoptimizer encoding; also loads faster than parsing CryEngine chunks.
4. Animations (`.caf`): key reduction and quantisation.
