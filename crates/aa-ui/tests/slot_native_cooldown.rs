//! Native392c7360 selector goldens; the query is deliberately observable.
use aa_ui::{UiRuntime, lua, tree::Id};
use lua::slot::{CooldownSample as Sample, NativeCooldownState as Flags};

fn sample(remain: u32, total: u32) -> Sample {
    Sample {
        remaining_ms: remain,
        total_ms: total,
    }
}
fn setup(native: bool) -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    lua::slot::set_native_cooldown_mode(&ui.lua, native);
    ui.exec(
        "@slot",
        r#"
   AAUI.SlotContent=function() return {type='skill',id=7,
     icon='ui/icon/icon_skill_basic01.dds',cooldown={remain=8000,total=12000}} end
   slot=UIParent:CreateWidget('slot','nativeSlot','UIParent')
   slot:SetExtent(42,42);slot:Show(true);slot:EstablishSlot(ISLOT_ACTION,1)
   events={};queries=0
   slot:SetHandler('OnContentUpdated',function(self,what,value)
     if string.find(what,'cooldown') then table.insert(events,what..':'..tostring(value)) end
   end)
 "#,
    )
    .unwrap();
    let id = lua::value_id(&ui.lua, &ui.lua.load("slot").eval().unwrap()).unwrap();
    (ui, id)
}
fn flags(ui: &UiRuntime, id: Id) -> Flags {
    lua::ui(&ui.lua)
        .tree
        .widget(id)
        .unwrap()
        .slot
        .as_ref()
        .unwrap()
        .native_cooldown
        .unwrap()
}
fn update(ui: &UiRuntime, id: Id, kind: u32, global: Sample, own: Option<Sample>) {
    lua::slot::update_native_slot_cooldown(&ui.lua, id, kind, global, |lua, _| {
        lua.load("queries=queries+1;table.insert(events,'query')")
            .exec()
            .unwrap();
        own
    });
}
fn events(ui: &UiRuntime) -> String {
    ui.lua
        .load("return table.concat(events,'|')")
        .eval()
        .unwrap()
}
fn values(ui: &UiRuntime, id: Id) -> (f32, f32, bool) {
    let u = lua::ui(&ui.lua);
    let cd = u.tree.widget(id).unwrap().cooldown.as_ref().unwrap();
    (cd.sampled_ms.unwrap(), cd.total_ms, cd.global)
}

#[test]
fn global_selection_skips_accessor_and_retains_native_order_into_accessor_timer() {
    let (ui, id) = setup(true);
    assert!(lua::ui(&ui.lua).tree.widget(id).unwrap().cooldown.is_none());
    update(&ui, id, 15, sample(900, 1000), Some(sample(8000, 12000)));
    update(&ui, id, 15, sample(800, 1000), Some(sample(7000, 12000)));
    assert_eq!(events(&ui), "global_cooldown_start:1000");
    assert_eq!(
        flags(&ui, id),
        Flags {
            global: true,
            accessor_active: false
        }
    );
    assert_eq!(values(&ui, id), (800., 1000., true));
    update(&ui, id, 15, sample(0, 1000), Some(sample(7000, 12000)));
    assert_eq!(
        events(&ui),
        "global_cooldown_start:1000|global_cooldown_done:nil|query|skill_cooldown_start:12000"
    );
    // Prior accessor-active state wins the selector even when a fresh raw GCD exists.
    update(&ui, id, 15, sample(900, 1000), Some(sample(900, 1000)));
    assert_eq!(
        flags(&ui, id),
        Flags {
            global: false,
            accessor_active: true
        }
    );
    assert_eq!(values(&ui, id), (900., 1000., false));
    update(&ui, id, 15, sample(0, 0), Some(sample(0, 12000)));
    assert!(events(&ui).ends_with("query|skill_cooldown_done:nil"));
    update(&ui, id, 15, sample(600, 1000), Some(sample(5000, 9000)));
    assert!(events(&ui).ends_with("global_cooldown_start:1000"));
}

#[test]
fn global_done_callback_changes_the_subsequent_query_and_false_query_is_silent() {
    let (ui, id) = setup(true);
    update(&ui, id, 15, sample(900, 1000), None);
    ui.exec(
        "@callback",
        r#"
  remaining=10
  slot:SetHandler('OnContentUpdated',function(self,what,value)
   table.insert(events,what)
   if what=='global_cooldown_done' then remaining=7000;self:EstablishSlot(ISLOT_ACTION,2) end
  end)
 "#,
    )
    .unwrap();
    lua::slot::update_native_slot_cooldown(&ui.lua, id, 15, Sample::default(), |lua, id| {
        assert_eq!(lua.load("remaining").eval::<u32>().unwrap(), 7000);
        assert_eq!(
            lua::ui(lua)
                .tree
                .widget(id)
                .unwrap()
                .slot
                .as_ref()
                .unwrap()
                .source,
            Some(lua::slot::Source::Slot(
                lua.globals().raw_get("ISLOT_ACTION").unwrap(),
                2
            ))
        );
        Some(sample(7000, 12000))
    });
    assert_eq!(
        flags(&ui, id),
        Flags {
            global: false,
            accessor_active: true
        }
    );
    ui.exec("@clear", "events={}").unwrap();
    update(&ui, id, 15, Sample::default(), None);
    assert_eq!(events(&ui), "query");
    assert_eq!(flags(&ui, id), Flags::default());
    assert_eq!(values(&ui, id), (0., 0., false));
}

#[test]
fn excluded_accessor_kinds_bypass_the_global_selector_and_raw_u32s_narrow_once() {
    for kind in [16, 17, 19] {
        let (ui, id) = setup(true);
        update(&ui, id, kind, sample(900, 1000), Some(sample(6000, 9000)));
        assert_eq!(events(&ui), "query|skill_cooldown_start:9000");
        assert_eq!(values(&ui, id), (6000., 9000., false));
    }
    let (ui, id) = setup(true);
    update(&ui, id, 15, sample(u32::MAX, u32::MAX - 1), None);
    assert_eq!(values(&ui, id), (4294967296., 4294967296., true));
    assert_eq!(ui.lua.load("queries").eval::<u32>().unwrap(), 0);
}

#[test]
fn reset_flags_preserves_snapshot_and_native_samples_do_not_run_legacy_countdown() {
    let (ui, id) = setup(true);
    update(&ui, id, 15, sample(900, 1000), None);
    lua::tick(&ui.lua, 10000.);
    assert_eq!(values(&ui, id), (900., 1000., true));
    assert_eq!(events(&ui), "global_cooldown_start:1000");
    ui.exec("@reset", "slot:ResetState()").unwrap();
    assert_eq!(flags(&ui, id), Flags::default());
    assert_eq!(values(&ui, id), (900., 1000., false));
    assert_eq!(events(&ui), "global_cooldown_start:1000");
    update(&ui, id, 15, sample(800, 1000), None);
    assert_eq!(
        events(&ui),
        "global_cooldown_start:1000|global_cooldown_start:1000"
    );
}

#[test]
fn explicit_mode_keeps_compatibility_countdown_and_other_widget_classes_independent() {
    let (ui, id) = setup(false);
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(id)
            .unwrap()
            .slot
            .as_ref()
            .unwrap()
            .native_cooldown
            .is_none()
    );
    lua::tick(&ui.lua, 9000.);
    assert!(lua::ui(&ui.lua).tree.widget(id).unwrap().cooldown.is_none());
    assert_eq!(events(&ui), "skill_cooldown_done:nil");
    update(&ui, id, 15, sample(900, 1000), None);
    assert_eq!(events(&ui), "skill_cooldown_done:nil");
    lua::slot::set_native_cooldown_mode(&ui.lua, true);
    assert_eq!(flags(&ui, id), Flags::default());
    update(&ui, id, 15, sample(900, 1000), None);
    lua::slot::set_native_cooldown_mode(&ui.lua, true);
    assert!(flags(&ui, id).global); // repeated capability installation does not restart.
    lua::slot::set_native_cooldown_mode(&ui.lua, false);
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(id)
            .unwrap()
            .slot
            .as_ref()
            .unwrap()
            .native_cooldown
            .is_none()
    );
    ui.exec("@rebind", "slot:EstablishSlot(ISLOT_ACTION,1);events={}")
        .unwrap();
    lua::tick(&ui.lua, 9000.);
    assert_eq!(events(&ui), "skill_cooldown_done:nil");
}

#[path = "../examples/native_cooldown_caption.rs"]
#[allow(dead_code)]
mod original;
#[test]
fn original_action_slot_reads_both_events_and_global_caption_is_suppressed() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    lua::slot::set_native_cooldown_mode(&ui.lua, true);
    original::install(&mut ui, true);
    ui.exec(
        "@original",
        r#"
  queries=0;events={}
  originalSlot=captionSlots[1]
  uses=0
  AAUI.UseSlot=function(kind,index,button)
    assert(kind==ISLOT_ACTION and index==1 and button=='LeftButton');uses=uses+1
  end
 "#,
    )
    .unwrap();
    let id = lua::value_id(&ui.lua, &ui.lua.load("originalSlot").eval().unwrap()).unwrap();
    update(&ui, id, 15, sample(900, 1000), Some(sample(8000, 12000)));
    assert!(
        ui.lua
            .load("originalSlot.cooldown and originalSlot.cooldownDuration==1000")
            .eval::<bool>()
            .unwrap()
    );
    let captions = |ui: &UiRuntime| {
        aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
            .into_iter()
            .filter(|q| q.corners.is_none() && q.tex == aa_ui::render::Tex::Atlas)
            .count()
    };
    let global_count = captions(&ui);
    update(&ui, id, 15, Sample::default(), Some(sample(8000, 12000)));
    assert!(
        ui.lua
            .load("originalSlot.cooldown and originalSlot.cooldownDuration==12000")
            .eval::<bool>()
            .unwrap()
    );
    assert!(captions(&ui) > global_count);
    update(&ui, id, 15, Sample::default(), Some(sample(0, 12000)));
    assert!(
        ui.lua
            .load("not originalSlot.cooldown and originalSlot.cooldownDuration==0")
            .eval::<bool>()
            .unwrap()
    );
    lua::tick(&ui.lua, 0.);
    let rect = lua::ui(&ui.lua).tree.rect(id);
    let point = (rect.x + rect.w / 2., rect.y + rect.h / 2.);
    assert_eq!(aa_ui::input::pick(&ui.lua, point.0, point.1), Some(id));
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&ui.lua, &mut mouse, Some(point), true, false, false, 0.);
    aa_ui::input::update(&ui.lua, &mut mouse, Some(point), false, true, false, 0.);
    assert_eq!(ui.lua.load("uses").eval::<u32>().unwrap(), 1);
    assert!(ui.errors.is_empty());
}
