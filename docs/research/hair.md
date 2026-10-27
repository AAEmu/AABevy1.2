# Hair: shader source, material values, current gap

Research for the hair shader rewrite. Read-only; the facts come from the pak (`<client>`), `reference/`
and `crates/aa-cry/src/shade.rs`. Extracted files: `data/extract/aa-cry/hair/` (git-ignored).

## 1. Verdict on the shader source

The pak contains the CryEngine source of the hair shader, so nothing here is from memory:

* `game/shaders/hwscripts/cryfx/hair.cfx` (28.5 KB, the effect), with the includes it uses:
  `shadelib.cfi` (KajiyaKay, ShiftVectorOpt, GetFresnel, Phong), `fraglib.cfi` (fragment pipeline,
  `frag_custom_*` hooks, alpha clip), `common.cfi`. Extracted to `data/extract/aa-cry/hair/`.
* `game/shaders/hair.ext`: editor extension, property masks (`VERTCOLORS` 0x10, `DISABLE_RAIN_PASS`
  0x20, `PHONG_SPECULAR` 0x10000, `TEMP_HAIR` 0x20000, `WIND_BENDING` 0x40000000).
* Compiled caches `game/shaders/cache/d3d9|d3d11/hair*.cfxb` and `cgpshaders|cgvshaders/hair@*.fxcb`
  (binary, not decoded). Their entry-point names show which variant the client built: `HairVS`,
  `HairOpaquePS`, `HairPS`, `ZPassVS/PS`. There is no `SoftAlphaTestPass` entry (see section 7).

Material keys in all sampled `.mtl` files: `Shader="Hair"`; the GenMask values never include the
`PHONG_SPECULAR` bit (0x10000), so the Kajiya-Kay path is the one in use.

## 2. The Hair shader (hair.cfx, non-soft variant)

Notation: `N` = perturbed normal (`pPass.vNormal`, normal map applied), `B` = `vBinormal` (the mesh
V axis, `ObjToTangentSpace[1]`), `V` = eye direction, `L` = light direction, `H = normalize(V+L)`.
`a_s` = alpha of the Specular map sampled at `baseTC * DetailTilling`; `a_d` = alpha of the Diffuse map.

**Strand direction.** The Kajiya-Kay tangent is the binormal `B` shifted along the normal (it is
not normalised):

    s       = (2u - 1) * BumpScale,  with u = 2*a_s - 1       (the source applies the remap twice: literal)
    T1      = B + N * (s + PrimarySpecShift)                  (primary lobe)
    T2      = B + N * (s + SecondarySpecShift)                (secondary lobe)
    KK(T,H,p) = pow( sqrt( max(1 - (T.H)^2, 0.01) ), p )      = (1 - (T.H)^2)^(p/2)

The shift noise is the **alpha of the Specular map**, not its luma. `BumpScale` is the shift
amplitude here, not a normal-map scale. The normal map is used unscaled.
Back faces: the vertex shader flips the tangent sign by `dot(eye, N) * 0.8 + 0.2`.

**Lobes and widths.** Two lobes, shifted by the two shift params, each with its own exponent and colour.
There is no separate "width" parameter: the width of each lobe is the exponent (`PrimarySpecPow`,
`SecondarySpecPow`). No anisotropy strength parameter exists.

**Per light** (`frag_custom_per_light`; also the two `BackLightDir/Color` lights when `%_RT_OBJECT_BACKLIGHT`):

    NdL'    = saturate( NdL * DiffuseWrap + (1 - DiffuseWrap) )          DiffuseWrap 0.5..1
    rim     = (1 - N.V)^3 * RimMultiplier * saturate(1 - saturate(L.V))  (fresnel = (1-N.V)^2, then ^1.5)
    diffuse = (NdL' + rim) * lightDiffuse
    spec    = ( KK(T1,H,PrimarySpecPow)*PrimarySpecColor
              + KK(T2,H,SecondarySpecPow)*SecondarySpecColor ) * lightSpec * NdL'
              (secondary lobe only at QUALITY_HIGH)
    + directional lights only, quality != LOW:
      0.2 * (1 - sat(occl * sat(NdL*3))) * KK(T2, normalize(Vx,Vy,1), PrimarySpecPow*0.3) * PrimarySpecColor * lightSpec
    each term times fOcclShadow * fFallOff * cFilter

**Ambient** (`frag_custom_end`):

    amb_term = cAmbientAcc * saturate(N.V*0.5 + 0.5)                 (wrapped ambient)
    amb_spec = ( KK(T1,V,PrimarySpecPow)*PrimarySpecColor + KK(T2,V,SecondarySpecPow)*SecondarySpecColor )
               * AmbientSpecMult * (amb_term + diffuseAcc)           (V used as H)

**Final colour:**

    cHair   = diffuseTex.rgb * HairColor.rgb * 2                     (set in frag_custom_begin)
    out.rgb = (amb_term + diffuseAcc) * cHair * MatDifColor          (MatDifColor = Diffuse, 0.0667)
            + (specAcc + amb_spec) * specMap.rgb * MatSpecColor       (MatSpecColor = Specular attr)
    out.a   = saturate(2 * a_d)

The env-map term is skipped: `frag_final_composition` is bypassed (`bCustomComposition`), so the
Environment slot is never sampled on the hair path. `Shininess` and `MatSpecColor.w` only feed the
Phong branch, which is not used. `Emissive` is not in the hair formulas here (the shade.rs header says
the shaders add it to the ambient accumulation; if so it reaches `amb_term`).

**Alpha handling (non-soft variant, the one with `ForceZpass; Hair;` in the script):**

* Pass `opaque_pass`: CullMode None, depth write on. No clip in the shader. The output alpha is `2*a_d`.
  The cut-out comes from the engine's alpha-test state (not in the pak). `bDisableAlphaTestD3D10` is set,
  so `fraglib`'s own clip is off.
* Pass `front_pass` (CullMode Front = back faces first), then `back_pass` (CullMode Back = front faces):
  ZWrite off, ZFunc Less, IgnoreMaterialState. Each does `clip(a_d - 0.05)` **before** the doubling,
  then `alpha = saturate(2*a_d)`. The blend state is not in the `.cfx`; it has to come from the engine's
  transparent-pass default (unverified).
* ZPass (depth/normal prepass): encodes `a_d` against `PBAlphaTest` (the mtl `AlphaTest`) with
  `EncodeSceneDepthWithAlpha`; normal written as `N*0.5+0.5`, alpha `MatSpecColor.w/255`.

**Texture slots used:** Diffuse (`diffuseMapSampler`: rgb colour, a = strand coverage);
Specular (`glossMapSampler`: rgb = spec mask, a = shift noise); Normalmap (`bumpMapSampler`);
Environment: sampled by the samplers block but not on the hair path. No separate shift or noise texture.

## 3. Hair .mtl parameters

Pak census (paths containing `/hair/`, `.mtl`): **12,769 files** (ferre 2456, elf 3235, hariharan 3538,
nuian 3480, plus 60 people/dwarf/npc/warborn files). 234 distinct style folders. The 267 hair DDS files
under `/hair/` include the shared `hair_texture/` maps.

Sample: 225 files, stratified 24 per race/gender group (`data/extract/aa-cry/hair/mtl/`, filenames
`<race>_<gender>_...`). 210 Hair submaterials parsed. Ranges are over the sample, first channel for
colours. Re-run the census before quoting a range as final.

Not all hair uses the Hair shader. 15 of the 225 sampled files are `Shader="Illum"`, not `Hair`:
the ferre hair styles `fe_f_hair01..05` and `fe_m_hair01..03` (some colours), plus the astra, dwarf,
warborn and people_girl hair. They take the Illum path (`bake_specular`, metallic/roughness), with
values such as Specular 0.38, Shininess 15, AlphaTest 0.525, `SpecularLevel` 1.2. The census of how many
of the 12,769 files are Illum was not run.

Material level, all sampled submaterials:

| Attribute | Values seen | Used by hair.cfx? |
|---|---|---|
| Diffuse | 0.0667 (201); 0.50, 0.43, 0.26 (dwarf, people) | yes, MatDifColor |
| Specular | 0.31–0.39 (elf, nuian); 0.25–0.31 (ferre f); 0.25–0.37 (hariharan f); 0.08–0.31 (ferre m); 0.11–0.31 (hariharan m); 0.08–4.7 (people, dwarf) | yes, MatSpecColor |
| Emissive | 0.353 (173), 0 (29), others | see section 2 |
| Shininess | absent (201); 10 (7); 1 and 1.23 (people) | no (Phong only) |
| AlphaTest | 0.30 (most); 0.19–0.30 (hariharan m); 0.10–0.60 (people); 0.01–0.02 (dwarf) | ZPass threshold |
| Opacity | 0.99 (208); 1 (2) | not in hair path |
| SpecularLevel | 1 (209); 7 (1) | no |
| MtlFlags (submaterial) | 542848 = 0x84880 (134); 526464 = 0x80880 (67); 2176 = 0x880 (9) | bit meanings not in the pak |
| GenMask | 0x800 (131); 0x20000 TEMP_HAIR (66); 0 (5); others | see section 7 |

Public params, all sampled Hair submaterials. Shader default in brackets (from the `.cfx` UI defaults).

| Param | Range (all) | Notes, per race/gender |
|---|---|---|
| HairColor | 0.02–1.00 (76 distinct) | elf/nuian/hariharan f: 0.02–1; ferre m: 0.07–0.98; hariharan m: 0.05–1 [1,1,1] |
| PrimarySpecColor | 0.03–0.94 | elf/nuian 0.03–0.84; ferre f 0.03–0.82; ferre m 0.07–0.65; people dwarf 0.5–0.94 [1,1,1] |
| SecondarySpecColor | 0.02–0.97 | elf/nuian 0.02–0.53; ferre f 0.04–0.25; ferre m 0.02–0.26 [1,1,1] |
| PrimarySpecPow | 58–395 | elf/ferre/nuian 60–180; hariharan m up to 332 [200] |
| PrimarySpecShift | -1.00–1.00 (7 distinct) | elf/nuian -1..0.29; ferre m fixed -1 [0.08] |
| SecondarySpecPow | 29–269 | elf/nuian 53–180; hariharan m 29–180 [16] |
| SecondarySpecShift | -0.78–0.12 | elf/nuian -0.55..-0.10; hariharan m -0.55..0.10 [0.54] |
| DiffuseWrap | 0.50–0.97 | elf/ferre/nuian/hariharan 0.80–0.97; people 0.8 [0.75] |
| RimMultiplier | 0.09–2.55 | elf/nuian/hariharan 0.09–2.55; ferre m 0.30–2.55 [0.2] |
| BumpScale (shift amplitude) | 0–0.98 | elf/nuian/hariharan 0.02–0.19; ferre m 0.19 fixed; people up to 0.98 [0.5] |
| AmbientSpecMult | 0.2–1.0 (3 values) | nuian f 0.2–0.37 [1.0] |
| DetailTilling | 0 or 1 (2 distinct values) | [1] |

Params present but not read by hair.cfx: `AnimFrequency`, `AnimAmplitudeWav0/2`, `AnimPhase` (wind,
needs `WIND_BENDING` 0x40000000, not in any sample GenMask), `VirtuaCLDiffuse` (0–1.98),
`VirtualCLSpecular` (0–2), `ViewOpacity` and `FillMultiplier` (people only). The wrapper material
carries `AmbientMultiplier`, `FresnelBias`, `FresnelScale`, `ReflectAmount`: not used on the hair path.
There is no width, anisotropy-strength or rim-power parameter.

Textures (210 submaterials): Diffuse 210, Specular 210, Normalmap 180, Environment 195. Names are
`<id>_df.dds` (diffuse), `<id>_sp.dds` (specular, alpha = shift noise) and `<id>_ddn.dds` (normal, often in
`hair_texture/`). Some styles reuse another style's `_sp`/`_ddn` (hair016 uses hair001's).
The 30 submaterials without a Normalmap slot presumably get the engine's flat default (unverified).

## 4. Our code now (crates/aa-cry/src/shade.rs, `"hair"` arm and helpers)

* Base: `StandardMaterial`, two-sided. `AlphaMode::Mask(0.5)` on `base_color.a = 2.0`, so a diffuse
  alpha of 0.25 is the cut. Emissive is zeroed.
* Colour: `base = (HairColor^1.5) * 2 * Diffuse`. The `^1.5` is a fit to the captures; the shader has no gamma.
* `bake_hair_colour`: fills the cut texels with the colour of the visible strands (alpha below 64/255).
* `bake_hair_specular` (Shading::Full only): replaces the Specular texture with a strand map built from
  the Specular map luma times a cover mask, and sets `reflectance = 2*s^0.4` (clamped 0.5..1.8) and
  `specular_tint` from `PrimarySpecColor`. Roughness is `shininess_to_roughness(PrimarySpecPow)`
  clamped to 0.3–0.5.

Divergences from the shader:

1. **No Kajiya-Kay, no second lobe.** Highlight is Bevy's isotropic GGX, masked by a baked strand map.
   Shift noise (Specular alpha) and `BumpScale` are not used. `SecondarySpec*` is dropped.
2. **Exponent mapping.** `PrimarySpecPow` 58–395 is squeezed into a roughness clamp of 0.3–0.5, so
   the sharp exponent variation is lost.
3. **Diffuse.** Lambert with no wrap (`DiffuseWrap` 0.8–0.97 in the data) and no rim term.
   The `(1-N.V)^3` back-rim term is missing.
4. **Specular scale.** The shader multiplies by `MatSpecColor` (`Specular`, 0.31–0.39) and the Specular map
   RGB. The code takes `PrimarySpecColor` luma and overwrites `reflectance`, so the `Specular` attribute
   never reaches hair.
5. **Alpha.** The shader keeps a soft fringe: blended back-to-front passes with `2*a` after a 0.05 clip.
   The code hard-cuts at `a = 0.25` and has no blended pass, so strand ends are thinner and harder.
6. **Strand direction.** The shader's highlight follows the binormal shifted by the normal; the code follows
   a baked texture, so streaks do not move with the view.
7. **Two-sided lighting.** The shader flips the tangent sign with the eye dot (`*0.8+0.2`); the code does not.

## 5. Reference captures with visible hair

All under `reference/`. Only the ones below were opened and checked for hair; `step1_*` and `step3_*`
(full body, hair small) and `ingame/spawn` were not reviewed for hair.

| File | Race / gender | Hairstyle, colour (as shown) |
|---|---|---|
| `character_create/step2_nuian_male/real.png` | nuian male | short swept-up style; pale ash-blond |
| `character_create/step2_nuian_female/real.png` | nuian female | pulled-back style; light blond-beige |
| `character_create/step2_elf_female/real.png` | elf female | pulled-back bun; light blond |
| `character_create/step2_elf_male/real.png` | elf male | swept-back style; ash gold |
| `character_create/step2_ferre_female/real.png` | ferre female | dreads; copper red |
| `character_create/step2_ferre_male/real.png` | ferre male | dreads; beige gold |
| `character_create/step2_hariharan_female/real.png` | hariharan female | sleek style with top knot; near-black dark brown |
| `character_create/step2_hariharan_male/real.png` | hariharan male | short spiky style; near-black dark brown |
| `character_select/1_character/real.png` | nuian male "Refone", Lv1 | short golden blond, sheen at the top |
| `ingame/character_info/real.png` | nuian male "Refone" (from behind) | short golden blond; small in the frame |

The step2 images are the best hair references: 1920x1080, face close up, the highlight streaks visible.
Hair colour swatches in the picker are not mapped to `HairColor` values in the pak. Match by eye.

## 6. Open questions

* Which variant the client picks at runtime (`%SOFT_ALPHATEST` or not) is not in any `.ext` we have.
  `hair.ext` and `runtime.ext` do not define it, and the cache holds no `SoftAlphaTestPass` entry, so the
  non-soft variant is the likely one. Inference, not proof.
* `MtlFlags` and `GenMask` bits are not all named in the pak. GenMask 0x800 maps to `%_RT_PARTICLE` in
  `runtime.ext`, which cannot be right for hair, so the GenMask mapping is unresolved.
* Engine blend state for the front/back passes (`IgnoreMaterialState`) and the opaque-pass alpha test are
  set by the engine, not in the `.cfx`.
* The double remap of the shift noise (`(2u-1)*BumpScale`, `u = 2a-1`) is literal source. With
  `BumpScale` 0.02–0.19 the shift is small, but check it against the captures.

## 7. Notes for the implementer

* Port `hair.cfx` as a custom material: the two Kajiya-Kay lobes with shift noise from the Specular alpha,
  the wrapped diffuse and rim terms, the ambient spec, and the Specular map RGB times `Specular` as spec mask.
* Keep the two alpha passes: an opaque pass with the mtl alpha test, then a back-to-front blended pass
  (`clip(a-0.05)`, alpha `2a`). Do not use alpha-to-coverage. The prepass issue in shade.rs still applies,
  so the prepass should see the opaque pass only.
* `HairColor`, `PrimarySpecColor` and `SecondarySpecColor` are in the 0–1 picker range. Keep the `^1.5`
  fit until the picker's gamma is known.
