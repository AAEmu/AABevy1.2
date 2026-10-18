# Received headgear and authored hair variants

Character select, listed-character prewarming, local-player spawn and prepared
remote humanoids now resolve their received appearance through the same client
headgear adapter. Character creation without equipment keeps its existing
resolver. The already verified aa-anim selector/resolver supplies the authored
mesh or explicit hidden-hair result; its512 original-byte selector goldens and
source details are recorded in aa-anim.md.

The adapter uses the actual model-specific item visual admitted by the existing
costume assembler. The image-item look is tried first and the original item is
the existing fallback when the replacement has no complete pak assets. Costume
assembly and hair selection share this lookup, so a fallback hood cannot retain
the unavailable replacement helmet's hair flags. Only descriptors represented
in the resolved Costume::Items set qualify.

Original39315820 gives the rendered cosplay descriptor (wire27) priority over
head (wire0), including a cosplay descriptor that selects normal hair. The raw
item_assets.detail is passed intact to aa-anim. Missing equipment follows the
ordinary resolver without an extra hair reload. Style/color materials and all
other resolved parts are retained. A read-only catalog check found no original
armor/model group with differing detail values; an ambiguous replacement catalog
is refused instead of choosing an arbitrary descriptor.

The native512 selector goldens do not establish every appearance producer.
This adapter follows the existing rendered costume; it adds no new helmet
visibility policy, beard geometry or rebuild after later equipment/appearance
changes. Those remain separate integration gaps. Full scene/material/occlusion
parity and clothing clipping are not inferred from selecting the correct hair.

The headgear aa-dev example feeds explicit received-style equipment through
the same describe/resolve functions and actual client player spawn: original
Nuian female hairstyle24024, metal22874 (detail28744), hood18230 (25768), and
the hood with received image override22874. All four retain their full body rigs
and the same original chest armor2691. No connection, equipment transaction or
character mutation is performed.

Three focused checks pass in0.86s against the actual database/pak: normal,
Type3 and hidden hair with other parts/materials preserved; received image
override and invalid-image fallback matching rendered costume; and genuine
cosplay14359/detail57344 retaining normal hair over the hidden-head descriptor,
nonrendered equipment and explicit bald appearance. The expected asset path
honors the original direct mesh versus .cdf formats. The combined full client
suite at this milestone passes298 tests /38 optional probes ignored in55.55s with three test
threads, including these resolver calls. Its inspected main was
target/debug/aa-client-20261018-native-fades.exe, SHA256
C236C8B021D8674370142878D8DA9B0E957A1E54B0768B89E2AC4E90E9E53510.

The final headgear example builds and its1920x1080 capture
shots/aa-client/headgear/received.png is inspected at288.1-416.2 FPS without
WARN/ERROR/panic. PID23312 self-closes. Logged resolved hair paths confirm normal
nu_f_hair001.chr, the metal's authored nu_f_hair001_hm_type3.chr, absent hood hair,
and that same Type3 asset for the received image override. This is a diagnostic
selection/render check; source hair/armor clipping and full native-scene visual
equivalence remain unclaimed.

The later combined main target/debug/aa-client-20261018-enchant-frames.exe and
idle target/debug/aa-client-play.exe alias now share SHA256
347DDF0D98E758E49CCBF6E082048BA2894635637CBF7430D14EBD2F83A00327.
Its full suite passes307 tests /38 optional probes ignored in86.92s, including
the same headgear checks. Eight inspected original HUD/world-list captures
validate that combined build (aa-client-enchant.md); they add no new headgear
scene/reference claim.
