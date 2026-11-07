//! The `net` rows: the skill and combat packets the client sends (`aa_net::Command`) and receives (`aa_net::Event`), by the
//! packet name the protocol uses (`aa_net::opcodes`, without the `Packet` suffix), with the skill, timeline and unit they name.
//!
//! aa-net's events are decoded, not raw packets, so the mapping is by variant. Only the skill, buff, target and combat
//! packets are listed; anything else is not traced. `obj` is the unit the packet is about: the caster of a skill packet, the
//! one hit, healed or buffed for the packet of an effect.

use aa_net::opcodes::{cs, sc};
use aa_net::{Command, Event};

/// What a `net` row says besides its direction.
#[derive(Debug, Default, PartialEq)]
pub struct Net {
    pub op: &'static str,
    pub skill_id: Option<u32>,
    pub tl: Option<u16>,
    pub obj: Option<u32>,
    /// Small extra integers (`fields` of the row).
    pub fields: Vec<(&'static str, i64)>,
}

fn client(op: u16) -> &'static str {
    cs::name(op).trim_end_matches("Packet")
}

fn server(op: u16) -> &'static str {
    sc::name(op).trim_end_matches("Packet")
}

fn nonzero(v: u32) -> Option<u32> {
    (v != 0).then_some(v)
}

/// A command the app sends, when it is a skill or combat one.
pub fn of_command(command: &Command) -> Option<Net> {
    let start = |skill_id: u32, obj: u32| Net { op: client(cs::CSStartSkillPacket), skill_id: Some(skill_id), obj: nonzero(obj), ..Net::default() };
    Some(match command {
        Command::UseSkill { skill_id, target_obj_id } => start(*skill_id, *target_obj_id),
        Command::UseSkillAt { skill_id, .. } | Command::UseItemAt { skill_id, .. } | Command::UseMateSkillAt { skill_id, .. } => start(*skill_id, 0),
        Command::UseItem { skill_id, target_obj_id, .. } | Command::UseMateSkill { skill_id, target_obj_id, .. } => start(*skill_id, *target_obj_id),
        Command::UseDoodadSkill { skill_id, doodad_obj_id } => start(*skill_id, *doodad_obj_id),
        Command::StopCasting { tl, plot_tl } => Net {
            op: client(cs::CSStopCastingPacket),
            tl: Some(if *tl != 0 { *tl } else { *plot_tl }),
            fields: vec![("plot", i64::from(*tl == 0 && *plot_tl != 0))],
            ..Net::default()
        },
        Command::Target { obj_id } => Net { op: client(cs::CSChangeTargetPacket), obj: nonzero(*obj_id), ..Net::default() },
        _ => return None,
    })
}

/// An event the session reports, when it is a skill or combat one.
pub fn of_event(event: &Event) -> Option<Net> {
    Some(match event {
        Event::SkillStarted(s) => Net {
            op: server(sc::SCSkillStartedPacket),
            skill_id: Some(s.skill_id),
            tl: Some(s.tl),
            obj: nonzero(s.caster.obj_id),
            fields: vec![("target", i64::from(s.target.obj_id)), ("cast_ms", i64::from(s.cast_ms)), ("result", i64::from(s.result))],
        },
        Event::SkillFired(s) => Net {
            op: server(sc::SCSkillFiredPacket),
            skill_id: Some(s.skill_id),
            tl: Some(s.tl),
            obj: nonzero(s.caster.obj_id),
            fields: vec![
                ("target", i64::from(s.target.obj_id)),
                ("delay_ms", i64::from(s.delay_ms)),
                ("channeling_ms", i64::from(s.channeling_ms)),
                ("fire_anim_id", i64::from(s.fire_anim_id)),
            ],
        },
        Event::SkillEnded { tl } => Net { op: server(sc::SCSkillEndedPacket), tl: Some(*tl), ..Net::default() },
        Event::SkillStopped { obj_id, skill_id } => Net { op: server(sc::SCSkillStoppedPacket), skill_id: Some(*skill_id), obj: nonzero(*obj_id), ..Net::default() },
        Event::CastingStopped { tl, duration_ms } => Net {
            op: server(sc::SCCastingStoppedPacket),
            tl: Some(*tl),
            fields: vec![("duration_ms", i64::from(*duration_ms))],
            ..Net::default()
        },
        Event::SkillCooldownReset { obj_id, skill_id, tag_id, gcd } => Net {
            op: server(sc::SCSkillCooldownResetPacket),
            skill_id: nonzero(*skill_id),
            obj: nonzero(*obj_id),
            fields: vec![("tag_id", i64::from(*tag_id)), ("gcd", i64::from(*gcd))],
            ..Net::default()
        },
        Event::Cooldowns(_) => Net { op: server(sc::SCCooldownsPacket), ..Net::default() },
        Event::PlotEvent(p) => Net {
            op: server(sc::SCPlotEventPacket),
            skill_id: Some(p.skill_id),
            tl: Some(p.tl),
            obj: nonzero(p.caster_obj_id),
            fields: vec![("event_id", i64::from(p.event_id)), ("target", i64::from(p.target_obj_id)), ("casting_ms", i64::from(p.casting_ms))],
        },
        Event::PlotEnded { tl } => Net { op: server(sc::SCPlotEndedPacket), tl: Some(*tl), ..Net::default() },
        Event::PlotStopped { tl, duration_ms, channeling, .. } => Net {
            op: server(if *channeling { sc::SCPlotChannelingStoppedPacket } else { sc::SCPlotCastingStoppedPacket }),
            tl: Some(*tl),
            fields: vec![("duration_ms", i64::from(*duration_ms))],
            ..Net::default()
        },
        Event::UnitDamaged(d) => Net {
            op: server(sc::SCUnitDamagedPacket),
            skill_id: nonzero(d.action.skill_id),
            tl: Some(d.action.tl),
            obj: nonzero(d.target_obj_id),
            fields: vec![("caster", i64::from(d.caster_obj_id)), ("damage", i64::from(d.damage)), ("hit_type", i64::from(d.hit_type))],
        },
        Event::UnitHealed(h) => Net {
            op: server(sc::SCUnitHealedPacket),
            skill_id: nonzero(h.action.skill_id),
            tl: Some(h.action.tl),
            obj: nonzero(h.target_obj_id),
            fields: vec![("caster", i64::from(h.caster.obj_id)), ("value", i64::from(h.value)), ("hit_type", i64::from(h.hit_type))],
        },
        Event::CombatText { source, target, text_type } => Net {
            op: server(sc::SCCombatTextPacket),
            obj: nonzero(*target),
            fields: vec![("source", i64::from(*source)), ("text_type", i64::from(*text_type))],
            ..Net::default()
        },
        Event::BuffCreated { obj_id, buff } => Net {
            op: server(sc::SCBuffCreatedPacket),
            skill_id: nonzero(buff.skill_id),
            obj: nonzero(*obj_id),
            fields: vec![("buff_id", i64::from(buff.buff_id)), ("index", i64::from(buff.index))],
            ..Net::default()
        },
        Event::BuffRemoved { obj_id, index } => Net { op: server(sc::SCBuffRemovedPacket), obj: nonzero(*obj_id), fields: vec![("index", i64::from(*index))], ..Net::default() },
        Event::BuffUpdated { obj_id, index, .. } => Net { op: server(sc::SCBuffUpdatedPacket), obj: nonzero(*obj_id), fields: vec![("index", i64::from(*index))], ..Net::default() },
        Event::TargetChanged { obj_id, target_obj_id } => Net {
            op: server(sc::SCTargetChangedPacket),
            obj: nonzero(*obj_id),
            fields: vec![("target", i64::from(*target_obj_id))],
            ..Net::default()
        },
        Event::UnitDeath { obj_id } => Net { op: server(sc::SCUnitDeathPacket), obj: nonzero(*obj_id), ..Net::default() },
        Event::CombatChanged { obj_id, engaged } => Net {
            op: server(if *engaged { sc::SCCombatEngagedPacket } else { sc::SCCombatClearedPacket }),
            obj: nonzero(*obj_id),
            ..Net::default()
        },
        Event::CombatFirstHit { victim, hitter, .. } => Net {
            op: server(sc::SCCombatFirstHitPacket),
            obj: nonzero(*victim),
            fields: vec![("hitter", i64::from(*hitter))],
            ..Net::default()
        },
        Event::Teleported { .. } => Net { op: server(sc::SCTeleportUnitPacket), ..Net::default() },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_use_the_protocol_names() {
        let n = of_command(&Command::UseSkill { skill_id: 14376, target_obj_id: 77 }).unwrap();
        assert_eq!((n.op, n.skill_id, n.obj), ("CSStartSkill", Some(14376), Some(77)));
        // a self buff has no target
        assert_eq!(of_command(&Command::UseSkill { skill_id: 1, target_obj_id: 0 }).unwrap().obj, None);
        let n = of_command(&Command::StopCasting { tl: 0, plot_tl: 9 }).unwrap();
        assert_eq!((n.op, n.tl), ("CSStopCasting", Some(9)));
        assert_eq!(of_command(&Command::Target { obj_id: 5 }).unwrap().op, "CSChangeTarget");
        assert!(of_command(&Command::RefreshCharacters).is_none(), "only skill and combat commands are traced");
    }

    #[test]
    fn events_use_the_protocol_names() {
        let n = of_event(&Event::SkillEnded { tl: 12 }).unwrap();
        assert_eq!((n.op, n.tl), ("SCSkillEnded", Some(12)));
        let n = of_event(&Event::CastingStopped { tl: 3, duration_ms: 400 }).unwrap();
        assert_eq!(n.op, "SCCastingStopped");
        assert_eq!(n.fields, vec![("duration_ms", 400)]);
        let n = of_event(&Event::PlotStopped { tl: 4, duration_ms: 0, last_event: 0, channeling: true }).unwrap();
        assert_eq!(n.op, "SCPlotChannelingStopped");
        let n = of_event(&Event::CombatChanged { obj_id: 8, engaged: false }).unwrap();
        assert_eq!((n.op, n.obj), ("SCCombatCleared", Some(8)));
        assert!(of_event(&Event::EnteredWorld).is_none());
    }
}
