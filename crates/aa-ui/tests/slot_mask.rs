//! Executed392c2f50/392c3960 ordinary-slot mask goldens.
use aa_ui::{
    UiRuntime, lua,
    render::{Quad, Tex},
    tree::Id,
};
use lua::slot::CooldownSample;
const MASK: &str = "ui/icon/cooldown_mask.dds";

fn setup(native: bool) -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    lua::slot::set_native_cooldown_mode(&ui.lua, native);
    ui.exec(
        "@slot_mask",
        r#"
      AAUI.SkillContent=function() return {type='skill',id=7,
        icon='ui/icon/icon_skill_basic01.dds',cooldown={remain=1000,total=1000}} end
      slot=UIParent:CreateWidget('slot','maskSlot','UIParent')
      slot:SetExtent(42,42);slot:AddAnchor('TOPLEFT',UIParent,0,0);slot:Show(true)
      slot:EstablishSkill(7);events={}
      slot:SetHandler('OnContentUpdated',function(self,event) table.insert(events,event) end)
    "#,
    )
    .unwrap();
    let id = lua::value_id(&ui.lua, &ui.lua.load("slot").eval().unwrap()).unwrap();
    (ui, id)
}
fn sample(ui: &UiRuntime, id: Id, remaining_ms: u32, total_ms: u32) {
    lua::slot::update_native_slot_cooldown(&ui.lua, id, 15, CooldownSample::default(), |_, _| {
        Some(CooldownSample {
            remaining_ms,
            total_ms,
        })
    });
}
fn quads(ui: &UiRuntime) -> Vec<Quad> {
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| matches!(q.tex, Tex::MaskedImage { .. }))
        .collect()
}
fn fields(ui: &UiRuntime, id: Id) -> (f32, f32) {
    let u = lua::ui(&ui.lua);
    let cd = u.tree.widget(id).unwrap().cooldown.as_ref().unwrap();
    (cd.total_ms, cd.sampled_ms.unwrap())
}

#[test]
fn stock_texture_is_both_inputs_and_half_sector_keeps_native_position_and_uv() {
    let (ui, id) = setup(true);
    sample(&ui, id, 9375, 10000);
    let q = quads(&ui);
    assert_eq!(q.len(), 8);
    assert_eq!(
        q[0].corners.unwrap(),
        [[21., 21.], [41., 1.], [31., 1.], [31., 1.]]
    );
    assert_eq!(
        q[0].uvs.unwrap(),
        [[0.5, 0.5], [1., 0.], [0.75, 0.], [0.75, 0.]]
    );
    assert_eq!(q[0].color, [1.; 4]);
    for q in &q {
        assert_eq!(
            q.tex,
            Tex::MaskedImage {
                image: MASK.into(),
                mask: MASK.into(),
                image_uv: [0., 0., 1., 1.].map(f32::to_bits),
                mask_uv: [0., 0., 1., 1.].map(f32::to_bits)
            }
        );
    }
    // Source mask-image anchors belong to the slot, independently of bound icon UV/layout/color.
    ui.exec(
        "@icon_change",
        r#"
      slot.icon:RemoveAllAnchors();slot.icon:SetExtent(8,12)
      slot.icon:AddAnchor('TOPLEFT',slot,17,19);slot.icon:SetColor(0.2,0.3,0.4,0.5)
      slot.icon:SetCoords(4,8,16,16)
    "#,
    )
    .unwrap();
    let after = quads(&ui);
    assert_eq!(after.len(), q.len());
    for (before, after) in q.iter().zip(after) {
        assert_eq!(after.tex, before.tex);
        assert_eq!(after.corners, before.corners);
        assert_eq!(after.uvs, before.uvs);
        assert_eq!(after.color, before.color);
    }
}

#[test]
fn rectangular_and_fractional_geometry_snaps_after_native_perimeter_interpolation() {
    let (ui, id) = setup(true);
    ui.exec(
        "@rectangle",
        r#"
      slot:SetExtent(42.6,79.4);slot:RemoveAllAnchors()
      slot:AddAnchor('TOPLEFT',UIParent,100.3,101.7)
    "#,
    )
    .unwrap();
    sample(&ui, id, 8000, 10000);
    let q = quads(&ui);
    assert_eq!(q.len(), 7);
    // Actual native input image rectangle101.3,102.7,141.9,180.1; first vertex is not a ray.
    assert_eq!(
        q[0].corners.unwrap(),
        [[122., 141.], [142., 141.], [142., 126.], [142., 126.]]
    );
    let uv = q[0].uvs.unwrap();
    assert!((uv[2][1] - 0.3).abs() < 1e-6);
    sample(&ui, id, 5000, 10000);
    assert_eq!(quads(&ui).len(), 4);
}

#[test]
fn native_zero_and_invalid_progress_clear_only_ui_draw_fields_before_caption() {
    for (remain, total) in [(1000, 1000), (1100, 1000)] {
        let (ui, id) = setup(true);
        sample(&ui, id, remain, total);
        let before = ui
            .lua
            .load("table.concat(events,'|')")
            .eval::<String>()
            .unwrap();
        assert!(quads(&ui).is_empty());
        assert_eq!(fields(&ui, id), (0., 0.));
        assert!(
            lua::ui(&ui.lua)
                .tree
                .widget(id)
                .unwrap()
                .slot
                .as_ref()
                .unwrap()
                .native_cooldown
                .unwrap()
                .accessor_active
        );
        assert_eq!(
            ui.lua
                .load("table.concat(events,'|')")
                .eval::<String>()
                .unwrap(),
            before
        );
        assert!(
            aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
                .iter()
                .all(|q| q.tex != Tex::Atlas)
        );
        // A later actual provider sample restores rendering without a second start callback.
        sample(&ui, id, 800, 1000);
        assert_eq!(quads(&ui).len(), 7);
        assert_eq!(
            ui.lua
                .load("table.concat(events,'|')")
                .eval::<String>()
                .unwrap(),
            before
        );
    }
}

#[test]
fn native_missing_mask_zero_remaining_and_minimum_total_keep_native_reset_boundaries() {
    let (ui, id) = setup(true);
    sample(&ui, id, 100, 199);
    assert!(quads(&ui).is_empty());
    assert_eq!(fields(&ui, id), (199., 100.));
    sample(&ui, id, 0, 1000);
    assert!(quads(&ui).is_empty());
    assert_eq!(fields(&ui, id), (1000., 0.));
    lua::ui(&ui.lua)
        .tree
        .widget_mut(id)
        .unwrap()
        .slot
        .as_mut()
        .unwrap()
        .cooldown_mask = None;
    sample(&ui, id, 1000, 1000);
    assert!(quads(&ui).is_empty());
    assert_eq!(fields(&ui, id), (1000., 1000.));
}

#[test]
fn compatibility_clock_still_advances_after_equal_initial_counts() {
    let (ui, id) = setup(false);
    assert!(quads(&ui).is_empty());
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(id)
            .unwrap()
            .cooldown
            .as_ref()
            .unwrap()
            .sampled_ms
            .is_none()
    );
    lua::tick(&ui.lua, 62.5);
    assert_eq!(
        quads(&ui)[0].corners.unwrap(),
        [[21., 21.], [41., 1.], [31., 1.], [31., 1.]]
    );
    lua::tick(&ui.lua, 1000.);
    assert!(quads(&ui).is_empty());
    assert_eq!(
        ui.lua
            .load("table.concat(events,'|')")
            .eval::<String>()
            .unwrap(),
        "skill_cooldown_done"
    );
}
