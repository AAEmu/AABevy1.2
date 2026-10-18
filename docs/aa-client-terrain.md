# Physical support over native terrain holes

The world eca1a50 milestone restores the original five-bit surface IDs, native
height packing/triangle split and real SID31 terrain holes. Its raw height_at
API deliberately keeps decoded heights over a hole. Physical ground_at and
WorldCollision queries omit the cap and can find the lower cave/proxy floor.

The client now uses physical ground for local support and terrain slope tests,
remote-unit snapping, camera/spawn/prewarm height fallback, target rings, party
ping placement and the existing terrain-only FX-ground callback. Raw height
queries still serve map availability, water/debug calculations and invalid-Z
loading fallback where those existing callers need the raw terrain API.

Local support no longer requires a raw terrain height before checking actual
brush/proxy floors. A known hole without physical support has no landing floor,
so existing gravity/fall logic can enter the opening. Missing/outside cells keep
the prior streaming/map fallback. A lower loaded floor beneath a hole is still
valid support. The internal absent floor is negative infinity, never a player
position or outgoing network coordinate. Remote actors keep received Z over
holes rather than snapping to an invisible cap.

The focused client integration check passes against actual original assets:
hole (21204.25,9026.5) retains raw446.40103m but supplies no support; the nearby
solid control (21188.25,9030.5) supplies498.16614m; outside-map fallback stays
unchanged. After loading real20,8 collision, hole (21270.5,8960.5) has raw
474.8673m but correctly supports the lower464.76614m proxy floor. The check
also verifies the camera/spawn helper preserves received Z over the hole.
It finishes in0.54s. Owning world QA separately passes217 tests, native numeric
oracles, ten clean/self-closing GPU scenes, visible cap on/off and zero-resource
unload. See docs/aa-world.md for exact native provenance and captures.

This is headless controller-support integration, not a live avatar traversal
claim. The shared lane in-world connection stall still blocks live movement
validation. Existing collision streaming/fallback policy, non-terrain FX-ground
receivers and broader water/visarea behavior remain separate work.
