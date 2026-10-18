# Login art — jade cloud continent

Original artwork generated with the built-in imagegen tool, 18 October 2026.
Style references only: `shots/gallery/01_world_select.png` and `07_loading.png`.
No original game artwork was copied or extracted. No text or logos are baked in.

| File | Purpose / generation prompt |
|---|---|
| bg_far.png | 2560×1440 opaque base: watercolour floating mountain continent on left, jade/indigo sky, gold cloud curls, quiet dark centre-right. |
| bg_mid.png | 2560×1440 RGBA overlay: left cliff, pine trees, tiny pagoda, translucent wash at its foot; rest empty. |
| mist_a.png | 2048×1024 RGBA: pale jade/blue-grey horizontal watercolour cloud band with airy curls. |
| mist_b.png | 2048×1024 RGBA: thinner indigo/teal cloud wisps with restrained gold edges. |
| glow.png | 128×128 RGBA: one round pale-gold firefly core, soft amber fade, no rays. |
| panel.png | 720×560 RGBA: thin antique-gold cloud filigree, jade inlays, empty translucent dark wash. |
| preview.gif / preview_still.png | Art-only composition; 6 s, 10 fps, 480×270. No fields/text are drawn. |
| manifest.json / prompts.json | Runtime numbers / exact verbatim generation prompts. |
| build_preview.py | Reproduce preview: `python crates/aa-client/assets/login/build_preview.py` (Pillow). |

Full prompts used, including composition constraints and transparency requests, are
in [prompts.json](prompts.json). Each image was generated separately, then resized
with Pillow/Lanczos. The base uses a 192-colour palette to stay below 4 MB.
RGBA layers retain full alpha; no key colour was needed. Mist received complementary
cosine blending of half-width-shifted copies in premultiplied alpha, with identical
edge columns, for seamless repeat. The panel interior opacity is capped near 82%.

## Manifest contract

All positions, sizes, drift, and mouse offsets use the **2560×1440 design canvas**,
origin top-left, +X right, +Y down. Scale the entire composition uniformly to cover
the viewport; crop evenly. Convert Bevy Y signs as needed. Z increases toward viewer.
PNG colour is sRGB with straight alpha; standard alpha-over blending.

Layers are centred at `position_px`. Add `parallax * mouse_offset_px` (positive
follows mouse), with mouse clamped to `mouse_clamp_px`. Add `drift_px_s * seconds`.
For `tile_x`, repeat at the image width times its current scale, wrapping drift
modulo that width. Draw enough copies to cover the screen; clamp sampler Y.

Breathing: `w = sin(2*pi*t/period_s + phase_rad)`;
`scale_now = scale * (1 + scale_amplitude*w)`;
`alpha_now = clamp(alpha + alpha_amplitude*w, 0, 1)`.
Overscan on the base and cliff hides their edges during the prescribed motion.

Motes use seeded uniform ranges. `spawn_area_px` is [left, top, right, bottom];
sample a birth position inside it, rise at the chosen speed (subtract Y), and add
sinusoidal X sway using the amplitude/period ranges and random phase. Size is sprite
diameter. Sample lifetime, fade linearly at both ends, multiply sprite by
`tint_rgba`, respawn, and prewarm with random ages. Ordinary alpha blend, Z=40.

Panel anchor is screen centre plus `offset_px`: centre at (1720,700), fixed
720×560, no mouse movement. Insets are [left, top, right, bottom]; inner content
area is 550×415. Put the screen's controls above Z=100; the art contains no controls.

Preview uses manifest values and a small sample mouse excursion. Its final 0.6 s
crossfades to the first frame to make a short seamless review loop; the game should
continue its drift normally. PNGs, alpha ranges, mist seams, and GIF duration were
checked; the composition was visually inspected. This asset-only handoff does not
run Bevy or measure FPS; the screen integration agent owns the DevPlugin capture.
