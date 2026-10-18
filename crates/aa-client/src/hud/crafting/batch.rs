//! One full batch request; UI progress follows observed local CraftEffect casts.
use super::super::api::{HudBridge, HudData, Shared};
use super::bench::Bench;
use aa_net::state::{Doodads, Player};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

#[derive(Default, Debug)]
pub(super) struct Batch {
    pub left: i32,
    recipe: u32,
    bench: u32,
    first: bool,
    cast: Option<(u16, u32)>,
    fired: bool,
    reset: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Request {
    Start { owner: u32, bench: Bench, recipe: u32, count: i32 },
    Stop,
    Close,
}

/// The two local cast slots remain independent; cancelling is a request, not completion.
#[derive(Default)]
pub(super) struct CastSlots {
    plain: Option<(u16, u32)>,
    plot: Option<(u16, u32)>,
}

impl CastSlots {
    fn observe(&mut self, event: &Event, own: u32) {
        match event {
            Event::SkillStarted(s) if s.caster.obj_id == own && s.result == 0 => self.plain = Some((s.tl, s.skill_id)),
            Event::PlotEvent(p) if p.caster_obj_id == own => self.plot = Some((p.tl, p.skill_id)),
            Event::SkillEnded { tl } | Event::CastingStopped { tl, .. } => {
                if self.plain.is_some_and(|c| c.0 == *tl) {
                    self.plain = None;
                }
            }
            Event::PlotEnded { tl } | Event::PlotStopped { tl, .. } => {
                if self.plot.is_some_and(|c| c.0 == *tl) {
                    self.plot = None;
                }
            }
            Event::SkillStopped { obj_id, skill_id } if *obj_id == own => {
                if self.plain.is_some_and(|c| c.1 == *skill_id) {
                    self.plain = None;
                }
                if self.plot.is_some_and(|c| c.1 == *skill_id) {
                    self.plot = None;
                }
            }
            Event::UnitDeath { obj_id } if *obj_id == own => *self = default(),
            _ => {}
        }
    }

    fn stop_commands(&self) -> impl Iterator<Item = Command> {
        self.plain
            .map(|c| Command::StopCasting { tl: c.0, plot_tl: 0 })
            .into_iter()
            .chain(self.plot.map(|c| Command::StopCasting { tl: 0, plot_tl: c.0 }))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Fire {
    Started(i32),
    Ended(Option<i32>),
    Failed(String),
}

fn failed_links(data: &HudData, notice: &aa_net::CraftFailureNotice) -> Vec<String> {
    let Some(game) = data.game.get() else { return Vec::new() };
    notice
        .entries
        .iter()
        .filter(|id| game.items.contains_key(id))
        .map(|&id| {
            let record = aa_ui::item_link::ItemRecord {
                item_type: id,
                flags: 1,
                world_id: 0xff,
                ..default()
            };
            aa_ui::item_link::format(&aa_ui::item_link::pack(&record))
        })
        .collect()
}

impl Batch {
    fn start(&mut self, recipe: u32, bench: u32, count: i32) {
        *self = Self {
            left: count,
            recipe,
            bench,
            first: true,
            ..default()
        };
    }

    fn next_frame(&mut self) {
        if self.reset {
            *self = default();
        }
    }

    fn observe(&mut self, event: &Event, own: u32, craft_skill: impl Fn(u32) -> bool) -> Option<Fire> {
        if self.left <= 0 {
            return None;
        }
        match event {
            Event::SkillStarted(s) if s.caster.obj_id == own && craft_skill(s.skill_id) => {
                if s.result != 0 {
                    *self = default();
                    return Some(Fire::Ended(None));
                }
                self.cast = Some((s.tl, s.skill_id));
                self.fired = false;
                if self.first {
                    self.first = false;
                    return Some(Fire::Started(self.left));
                }
            }
            Event::SkillFired(s) if s.caster.obj_id == own && craft_skill(s.skill_id) => self.fired = true,
            Event::SkillEnded { tl } if self.cast.is_some_and(|c| c.0 == *tl) => {
                if !self.fired {
                    self.cast = None;
                    self.reset = true;
                    return Some(Fire::Ended(None));
                }
                self.left -= 1;
                self.cast = None;
                self.fired = false;
                self.reset = self.left <= 0;
                return Some(Fire::Ended((self.left > 0).then_some(self.left)));
            }
            Event::SkillStopped { obj_id, skill_id } if *obj_id == own && craft_skill(*skill_id) => {
                self.cast = None;
                self.fired = false;
                self.reset = true;
                return Some(Fire::Ended(None));
            }
            Event::CraftRejected {
                craft_id,
                doodad_obj_id,
                count,
                ..
            } if self.recipe == *craft_id && self.bench == *doodad_obj_id && self.left == *count && self.first => {
                *self = default();
            }
            _ => {}
        }
        None
    }
}

fn queue(data: &mut HudData, recipe: u32, object: u32, count: i32) {
    if data.character_id == 0 || count <= 0 || data.crafting.batch.left > 0 {
        return;
    }
    let Some(bench) = data.crafting.current.active.filter(|b| b.object == object) else {
        return;
    };
    if data.crafting.current.recipes.contains(&recipe) {
        data.crafting.requests.push(Request::Start {
            owner: data.character_id,
            bench,
            recipe,
            count,
        });
    }
}

fn command(data: &HudData, doodads: &Doodads, owner: u32, request: Request) -> Option<Command> {
    let Request::Start {
        owner: quoted_owner,
        bench,
        recipe,
        count,
    } = request
    else {
        return None;
    };
    if owner == 0 || owner != quoted_owner || data.character_id != owner || data.crafting.batch.left > 0 || count <= 0 {
        return None;
    }
    if data.crafting.current.active != Some(bench) || !data.crafting.current.recipes.contains(&recipe) {
        return None;
    }
    let actual = doodads.by_id.get(&bench.object)?;
    if actual.template_id != bench.template {
        return None;
    }
    let game = data.game.get()?;
    let definition = game.crafting.recipes.get(&recipe)?;
    if !game.crafting.craft_skills.contains(&definition.skill) {
        return None;
    }
    Some(Command::ExecuteCraft {
        craft_id: recipe,
        doodad_obj_id: bench.object,
        count,
    })
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let craft: Table = lua.globals().get("X2Craft")?;
    let state = bridge.clone();
    craft.set(
        "IsWorkingCraft",
        lua.create_function(move |_, _: Value| Ok(state.borrow().crafting.batch.left > 0))?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetLeftBatchCount",
        lua.create_function(move |_, _: Value| Ok(state.borrow().crafting.batch.left))?,
    )?;
    let state = bridge.clone();
    craft.set(
        "ExecuteBatchCraftByType",
        lua.create_function(move |_, (_self, recipe, object, count): (Value, u32, u32, i32)| {
            queue(&mut state.borrow_mut(), recipe, object, count);
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "StopBatchCrafting",
        lua.create_function(move |_, _: Value| {
            state.borrow_mut().crafting.requests.push(Request::Stop);
            Ok(())
        })?,
    )?;
    let interaction: Table = lua.globals().get("X2Interaction")?;
    let prior: mlua::Function = interaction.get("CancelInteraction")?;
    let state = bridge.clone();
    interaction.set(
        "CancelInteraction",
        lua.create_function(move |_, args: mlua::MultiValue| {
            if state.borrow().crafting.current.active.is_some() {
                state.borrow_mut().crafting.requests.push(Request::Close);
                Ok(())
            } else {
                prior.call::<()>(args)
            }
        })?,
    )?;
    Ok(())
}

pub(super) fn update(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    doodads: Res<Doodads>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let owner = player.spawn.as_ref().map_or(0, |s| s.character().id);
    let mut fires = Vec::new();
    let mut messages = Vec::new();
    let mut ended_interaction = false;
    {
        let mut data = bridge.0.borrow_mut();
        if owner == 0 {
            data.crafting = default();
            events.clear();
            return;
        }
        data.crafting.batch.next_frame();
        let shared = data.game.clone();
        for NetEvent(event) in events.read() {
            data.crafting.casts.observe(event, own);
            if let Some(fire) = data
                .crafting
                .batch
                .observe(event, own, |skill| shared.get().is_some_and(|g| g.crafting.craft_skills.contains(&skill)))
            {
                fires.push(fire);
            }
            if let Event::CraftRejected { message, .. } = event {
                messages.push(message.clone());
            }
            if let Event::CraftFailureNotice(notice) = event {
                fires.extend(failed_links(&data, notice).into_iter().map(Fire::Failed));
            }
        }
        if data
            .crafting
            .current
            .active
            .is_some_and(|b| !doodads.by_id.get(&b.object).is_some_and(|d| d.template_id == b.template))
        {
            ended_interaction = super::bench::close(&mut data);
        }
        for request in std::mem::take(&mut data.crafting.requests) {
            match request {
                Request::Start { recipe, bench, count, .. } => {
                    if let Some(command) = command(&data, &doodads, owner, request) {
                        data.crafting.batch.start(recipe, bench.object, count);
                        net.write(NetCommand(command));
                    }
                }
                Request::Stop => {
                    for command in data.crafting.casts.stop_commands() {
                        net.write(NetCommand(command));
                    }
                }
                Request::Close => ended_interaction |= super::bench::close(&mut data),
            }
        }
    }
    for fire in fires {
        match fire {
            Fire::Started(left) => rt.fire_event("CRAFT_STARTED", left),
            Fire::Ended(Some(left)) => rt.fire_event("CRAFT_ENDED", left),
            Fire::Ended(None) => rt.fire_event("CRAFT_ENDED", ()),
            Fire::Failed(link) => rt.fire_event("CRAFT_FAILED", link),
        }
    }
    if ended_interaction {
        rt.fire_event("INTERACTION_END", ());
    }
    for message in messages {
        rt.fire_event("SYSMSG", message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::buff::BuffCaster;
    use aa_net::skill::{SkillFired, SkillStarted, SkillTarget};

    #[test]
    fn failure_notice_item_links_preserve_duplicates_and_do_not_change_batch() {
        let mut data = HudData::default();
        let mut game = super::super::super::data::GameData::default();
        game.items.insert(402, default());
        assert!(data.game.0.set(game).is_ok());
        data.crafting.batch.start(5557, 300, 3);
        let links = failed_links(
            &data,
            &aa_net::CraftFailureNotice {
                raw_id: 5557,
                raw_count: 3,
                entries: vec![402, 0, 402],
            },
        );
        assert_eq!(links.len(), 2);
        assert_eq!(links[0], links[1]);
        let payload = links[0].strip_prefix("|i402,0,").unwrap().strip_suffix(';').unwrap();
        let record = aa_ui::item_link::unpack(&aa_ui::item_link::decode_payload(payload).unwrap());
        assert_eq!(
            (record.id, record.item_type, record.grade, record.flags, record.count, record.world_id),
            (0, 402, 0, 1, 1, 255)
        );
        assert_eq!(data.crafting.batch.left, 3, "a failure item notice is not a cast-ended event");
    }

    fn started(tl: u16, result: u8) -> Event {
        Event::SkillStarted(SkillStarted {
            skill_id: 21787,
            tl,
            caster: BuffCaster { kind: 0, obj_id: 100 },
            target: SkillTarget::doodad(300).unwrap(),
            cast_ms: 1000,
            base_cast_ms: 1000,
            synergy: 0,
            result,
            result_value: 0,
            raw_result: aa_net::SkillResult { raw_flags: 1, code: result, ..Default::default() },
            raw_caster: aa_net::SkillCasterData::Unit(100),
            raw_target: aa_net::SkillTargetData::Doodad(300),
        })
    }
    fn fired(tl: u16) -> Event {
        Event::SkillFired(SkillFired {
            skill_id: 21787,
            tl,
            caster: BuffCaster { kind: 0, obj_id: 100 },
            target: SkillTarget::doodad(300).unwrap(),
            delay_ms: 0,
            channeling_ms: 0,
            fire_anim_id: 0,
            raw_final_flag: 0,
            result: Default::default(),
            raw_caster: aa_net::SkillCasterData::Unit(100),
            raw_target: aa_net::SkillTargetData::Doodad(300),
        })
    }
    #[test]
    fn native_observed_batch_latches_failure_stop_and_unfired_end() {
        let mut b = Batch::default();
        b.start(5557, 300, 3);
        assert_eq!(b.observe(&started(1, 0), 999, |_| true), None, "other casters do not advance this batch");
        assert_eq!(b.observe(&started(1, 0), 100, |_| false), None, "only CraftEffect skills qualify");
        assert_eq!(b.observe(&started(1, 0), 100, |_| true), Some(Fire::Started(3)));
        assert_eq!(b.observe(&fired(1), 100, |_| true), None);
        assert_eq!(b.left, 3, "Fired alone is not Ended or an inventory receipt");
        assert_eq!(b.observe(&Event::SkillEnded { tl: 1 }, 100, |_| true), Some(Fire::Ended(Some(2))));
        b.next_frame();
        assert_eq!(b.left, 2);
        assert_eq!(b.observe(&started(2, 0), 100, |_| true), None, "STARTED occurs only once for a submitted batch");
        assert_eq!(b.observe(&Event::SkillEnded { tl: 2 }, 100, |_| true), Some(Fire::Ended(None)));
        assert_eq!(b.left, 2, "unfired end clears the counter on the next manager update");
        b.next_frame();
        assert_eq!(b.left, 0);
        b.start(5557, 300, 3);
        assert_eq!(b.observe(&started(3, 1), 100, |_| true), Some(Fire::Ended(None)));
        assert_eq!(b.left, 0, "failed Started resets immediately");
        b.start(5557, 300, 3);
        b.observe(&started(4, 0), 100, |_| true);
        assert_eq!(
            b.observe(&Event::SkillStopped { obj_id: 100, skill_id: 21787 }, 100, |_| true),
            Some(Fire::Ended(None))
        );
        assert_eq!(b.left, 3);
        b.next_frame();
        assert_eq!(b.left, 0);
    }

    #[test]
    fn stop_requests_preserve_plain_and_plot_slots_until_received_end() {
        let mut slots = CastSlots::default();
        slots.observe(&started(9, 0), 100);
        slots.observe(
            &Event::PlotEvent(aa_net::skill::PlotEvent {
                tl: 12,
                event_id: 1,
                skill_id: 99,
                caster_obj_id: 100,
                target_obj_id: 300,
                caster_pos: None,
                target_pos: None,
                casting_ms: 1000,
            }),
            100,
        );
        let commands = slots.stop_commands().collect::<Vec<_>>();
        assert!(matches!(
            commands.as_slice(),
            [Command::StopCasting { tl: 9, plot_tl: 0 }, Command::StopCasting { tl: 0, plot_tl: 12 }]
        ));
        assert_eq!(slots.stop_commands().count(), 2, "sending cancel does not manufacture an end");
        slots.observe(&Event::CastingStopped { tl: 9, duration_ms: 0 }, 100);
        assert!(matches!(
            slots.stop_commands().collect::<Vec<_>>().as_slice(),
            [Command::StopCasting { tl: 0, plot_tl: 12 }]
        ));
        slots.observe(&Event::PlotEnded { tl: 12 }, 100);
        assert_eq!(slots.stop_commands().count(), 0);
    }

    #[test]
    #[ignore = "isolated original workbench and physical buttons"]
    fn original_workbench_buttons_send_one_batch_and_follow_received_casts() -> Result<(), Box<dyn std::error::Error>> {
        let (rt, bridge, path) = super::super::fixture::runtime(1920.0, 1080.0)?;
        let recipe = super::super::fixture::workbench(&rt, &bridge)?;
        assert_eq!(recipe, 5557);
        let inventory = bridge.borrow().crafting.inventory.clone().unwrap();
        let before = inventory.clone();
        let mut doodads = Doodads::default();
        doodads.by_id.insert(300, super::super::fixture::doodad());
        let quote = Request::Start {
            owner: 47,
            bench: Bench { object: 300, template: 568 },
            recipe,
            count: 3,
        };
        assert!(
            command(&bridge.borrow(), &doodads, 46, quote).is_none(),
            "a changed character invalidates the button request"
        );
        let mut replaced = doodads.clone();
        replaced.by_id.get_mut(&300).unwrap().template_id = 2241;
        assert!(
            command(&bridge.borrow(), &replaced, 47, quote).is_none(),
            "an object ID reused for a different bench is stale"
        );
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_non_send(rt)
            .insert_non_send(HudBridge(bridge.clone()))
            .insert_resource(Player {
                spawn: Some(super::super::super::expansion::fixture::spawn()),
            })
            .insert_resource(doodads)
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(Update, update);
        let drain = |app: &mut App| app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
        let click = |app: &App, widget: &str| super::super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), widget);
        click(&app, "CRAFTING_FRAME.detailPage.buttonFrame.maxCraftBtn")?;
        click(&app, "CRAFTING_FRAME.detailPage.buttonFrame.craftBtn")?;
        app.update();
        assert!(matches!(
            drain(&mut app).as_slice(),
            [Command::ExecuteCraft {
                craft_id: 5557,
                doodad_obj_id: 300,
                count: 3
            }]
        ));
        app.update();
        assert!(drain(&mut app).is_empty());
        app.world_mut().write_message(NetEvent(started(10, 0)));
        app.update();
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(X2Craft:IsWorkingCraft() and X2Craft:GetLeftBatchCount()==3); assert(CRAFTING_FRAME.detailPage.buttonFrame.cancelBtn:IsEnabled())")
            .exec()?;
        app.world_mut().write_message(NetEvent(fired(10)));
        app.world_mut().write_message(NetEvent(Event::SkillEnded { tl: 10 }));
        app.update();
        assert_eq!(bridge.borrow().crafting.batch.left, 2);
        assert!(drain(&mut app).is_empty(), "the server drives the next cast; the client never resubmits");
        app.world_mut().write_message(NetEvent(started(11, 0)));
        app.update();
        click(&app, "CRAFTING_FRAME.detailPage.buttonFrame.cancelBtn")?;
        app.update();
        assert!(matches!(drain(&mut app).as_slice(), [Command::StopCasting { tl: 11, plot_tl: 0 }]));
        assert_eq!(bridge.borrow().crafting.batch.left, 2, "cancel request is not an ACK");
        app.world_mut().write_message(NetEvent(Event::SkillStopped { obj_id: 100, skill_id: 21787 }));
        app.update();
        app.update();
        assert_eq!(bridge.borrow().crafting.batch.left, 0);
        assert_eq!(
            bridge.borrow().crafting.inventory.as_ref().unwrap().bag,
            before.bag,
            "progress must not synthesize item changes"
        );
        app.world().non_send::<UiRuntime>().lua.load("X2Interaction:CancelInteraction()").exec()?;
        app.update();
        assert!(bridge.borrow().crafting.current.active.is_none());
        assert_eq!(bridge.borrow().crafting.current.recipes.len(), 71, "closing retains native CURRENT maps");
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(not CRAFTING_FRAME:IsVisible()); assert(#X2Craft:GetCraftTypesFromCurrentDoodad(1,103)>0)")
            .exec()?;
        drop(app);
        drop(bridge);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
