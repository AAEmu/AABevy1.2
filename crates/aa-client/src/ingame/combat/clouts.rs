//! The effect areas of doodad clouts: a doodad whose phase has a `DoodadFuncClout` with a projectile shows that projectile's
//! effects where it stands, from `SCDoodadCreated` (or the phase change into it) until the clout's duration is over or the doodad
//! is removed. Sunder Earth's plot step 5319 spawns three doodads 3941 (`대지가르기 효과지역`, phase 9521: clout 865, projectile
//! 248, 7000 ms) and the 1.2 client shows `ground_dot_proj` at each of them 7 ms after the server's `SCDoodadCreated`; the server
//! sends no plot event for that step (skill parity trace t1i, 2026-11-03). Protective Wings' channel doodad 879 is one too: the
//! server spawns it at the caster's feet for the channel.
//!
//! The items play as they are: a clout's projectile is an effect area (`proj_physic_id` 3) whose effect is named like a buff
//! visual (`ground_dot_proj`), so the `_dot` rule of the skill's own effects does not apply here.

use super::*;

/// The effects each doodad's clouts show, by the doodad's object id.
#[derive(Resource, Default)]
pub(super) struct CloutFx(HashMap<u32, Vec<Entity>>);

/// How long a clout without a duration may show at most (it ends with the doodad's removal or phase change first).
const NO_DURATION_SECS: f32 = 600.0;

/// Shows the clouts of the doodads the server spawns or moves into a phase with clouts, and takes them away with the doodad.
pub(super) fn doodad_clouts(mut events: MessageReader<NetEvent>, mut st: Stage, mut shown: ResMut<CloutFx>) {
    for NetEvent(event) in events.read() {
        match event {
            Event::DoodadsSpawned(doodads) => {
                for d in doodads {
                    let at = cry_to_bevy(Vec3::new(d.pos.x, d.pos.y, d.pos.z));
                    show(&mut st, &mut shown, d.obj_id, d.func_group_id, at);
                }
            }
            Event::DoodadPhaseChanged { obj_id, func_group_id, .. } => {
                hide(&mut st.commands, &mut shown, *obj_id);
                let at = st.net_doodads.by_id.get(obj_id).map(|d| cry_to_bevy(Vec3::new(d.pos.x, d.pos.y, d.pos.z)));
                if let Some(at) = at {
                    show(&mut st, &mut shown, *obj_id, *func_group_id, at);
                }
            }
            Event::DoodadsRemoved(ids) => {
                for id in ids {
                    hide(&mut st.commands, &mut shown, *id);
                }
            }
            _ => {}
        }
    }
}

fn show(st: &mut Stage, shown: &mut CloutFx, obj_id: u32, phase: u32, at: Vec3) {
    let clouts = st.book.doodad_clouts(phase);
    if clouts.is_empty() {
        return;
    }
    // a doodad announced again (it came back into view) shows its areas once
    hide(&mut st.commands, shown, obj_id);
    let mut spawned = Vec::new();
    for clout in clouts.iter() {
        let secs = if clout.duration_ms > 0 { clout.duration_ms as f32 / 1000.0 } else { NO_DURATION_SECS };
        // the original spawns the area upright, unrotated, at the doodad's position (`rot` [0,0,0,1] in the trace)
        let anchor = st
            .commands
            .spawn((
                Name::new(format!("clout {} of doodad {obj_id}", clout.projectile.id)),
                Transform::from_translation(at),
                Visibility::default(),
                Fx { follow: None, fixed_rotation: true, ttl: secs },
            ))
            .id();
        for item in clout.projectile.items.iter().filter(|i| i.start <= ev::LAUNCH) {
            if item.effect.is_empty() {
                if let Some(sfx) = &item.sound {
                    st.audio.sfx(&mut st.commands, sfx, Class::Skill, anchor, None, anchor, at);
                }
                continue;
            }
            if muted(&item.effect) {
                continue;
            }
            st.effect_sounds(item, anchor, anchor, at);
            let effect = Effect { scale: item.scale, prime: false, item: true, despawn_when_finished: false, ..Effect::new(item.effect.clone()) };
            st.commands.spawn((Name::new(format!("fx {}", item.effect)), effect, Transform::from_translation(cry_to_bevy(item.offset)), ChildOf(anchor)));
        }
        if verbose() {
            info!("combat: doodad {obj_id} (phase {phase}) shows clout projectile {} for {secs:.1} s", clout.projectile.id);
        }
        spawned.push(anchor);
    }
    shown.0.insert(obj_id, spawned);
}

fn hide(commands: &mut Commands, shown: &mut CloutFx, obj_id: u32) {
    for e in shown.0.remove(&obj_id).unwrap_or_default() {
        commands.entity(e).try_despawn();
    }
}
