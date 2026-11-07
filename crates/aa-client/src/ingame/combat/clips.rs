//! The anim events of a model's clips: when in a clip the blow lands, the arrow leaves, the effects fire.
//!
//! Every character `.cal` names an `$AnimEventDatabase` (`animations/nuian/male/event_database.animevents`), an XML list of
//! `<animation name="...caf"><event name="skill_effect" time="0.358" parameter="launch" bone="item_hand_l"/>...`. The event
//! `parameter` says which group of the skill's effects starts at that moment (`form`, `throw`, `launch`, `hit`) and
//! `combat_sync` marks where the server's skill timeline is in the clip (the swing connects). Only the times are used here:
//! the effect items carry their own bones.
//!
//! An event's `time` is a phase of its clip, 0 at the start and 1 at the end (CryEngine's anim events are normalised): every
//! real time in the 1.2 databases lies in 0..=1 (the 32 above it are uninitialised floats near 1e31), and the original client
//! fires them at `time * duration`. 2026-11-03 skill trace, Triple Slash: `twohand_co_sk_streakattack_3` (2.30 s) plays its
//! `large_4` swing (0.04) at +94 ms and `large_8_axe` (0.12) at +281 ms, `streakattack_1` (1.71 s) its trail (`form` 0.140) at
//! +239 ms and its voice (0.11) at +188 ms. [`ClipEvents::of`] and [`clip_effects`] give seconds for a clip of known length.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The moments of one clip: phases (0..=1) as parsed, seconds from its start once [`ClipEvents::scaled`] by its length.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipEvents {
    pub form: Option<f32>,
    pub throw: Option<f32>,
    pub launch: Option<f32>,
    /// Where the clip meets the server's timeline: the blow lands, the arrow is released.
    pub combat_sync: Option<f32>,
    /// `voice_launch`: where the character calls out (the `FxVoice` items of the skill).
    pub voice: Option<f32>,
    /// `aw_none`: where the weapon leaves the hand (the holster clips).
    pub aw_none: Option<f32>,
}

impl ClipEvents {
    /// The moment the attack is released: `launch`, else `combat_sync`.
    pub fn release(&self) -> Option<f32> {
        self.launch.or(self.combat_sync)
    }

    /// The moment of the voice line: `voice_launch`, else the release.
    pub fn voice_time(&self) -> Option<f32> {
        self.voice.or_else(|| self.release())
    }

    /// The events of clip `name` of the model whose animation table is `cal`, in seconds for a clip `duration` seconds long
    /// (none when the model has no event database or no such clip).
    pub fn of(cal: &aa_anim::Cal, name: &str, duration: f32) -> ClipEvents {
        let (Some(db), Some(entry)) = (cal.anim_events.as_deref(), cal.animation(name)) else { return ClipEvents::default() };
        clip_events(db, &entry.path).scaled(duration)
    }

    /// Phases to seconds: every moment times the clip's length.
    pub fn scaled(self, duration: f32) -> ClipEvents {
        let at = |t: Option<f32>| t.map(|t| t * duration);
        ClipEvents {
            form: at(self.form),
            throw: at(self.throw),
            launch: at(self.launch),
            combat_sync: at(self.combat_sync),
            voice: at(self.voice),
            aw_none: at(self.aw_none),
        }
    }
}

/// An effect the clip itself starts (`effect` / `@effect` anim events: the flowers of `/flowers`, the tears of `/sad`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEffect {
    /// When in the clip: a phase (0..=1) as parsed, seconds in what [`clip_effects`] returns.
    pub time: f32,
    /// `library.group.name` for aa-fx.
    pub effect: String,
    /// The bone it starts on (empty: the unit's origin) and the offset from it in cry axes (m).
    pub bone: String,
    pub offset: [f32; 3],
}

type Table = HashMap<String, ClipEvents>;
type EffectTable = HashMap<String, Arc<[ClipEffect]>>;

/// The effects clip `name` of the model whose animation table is `cal` starts itself, at seconds into a clip `duration` long.
pub fn clip_effects(cal: &aa_anim::Cal, name: &str, duration: f32) -> Arc<[ClipEffect]> {
    let effects = clip_effect_phases(cal, name);
    effects.iter().map(|e| ClipEffect { time: e.time * duration, ..e.clone() }).collect()
}

/// [`clip_effects`] as parsed: the times are phases of the clip.
fn clip_effect_phases(cal: &aa_anim::Cal, name: &str) -> Arc<[ClipEffect]> {
    let (Some(db), Some(entry)) = (cal.anim_events.as_deref(), cal.animation(name)) else { return Arc::default() };
    static TABLES: OnceLock<Mutex<HashMap<String, Arc<EffectTable>>>> = OnceLock::new();
    let key = db.to_ascii_lowercase().replace('\\', "/");
    let tables = TABLES.get_or_init(Default::default);
    let cached = tables.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
    let table = cached.unwrap_or_else(|| {
        let parsed = Arc::new(aa_cry::read(&key).ok().map(|b| parse_effects(&String::from_utf8_lossy(&b))).unwrap_or_default());
        tables.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(parsed).clone()
    });
    table.get(&file_name(&entry.path)).cloned().unwrap_or_default()
}

/// The `effect` events of every clip of an `.animevents` file, by file name.
fn parse_effects(xml: &str) -> EffectTable {
    let mut table = EffectTable::new();
    let Ok(doc) = roxmltree::Document::parse(xml) else { return table };
    for animation in doc.descendants().filter(|n| n.has_tag_name("animation")) {
        let Some(name) = animation.attribute("name") else { continue };
        let mut effects: Vec<ClipEffect> = Vec::new();
        for event in animation.children().filter(|n| n.has_tag_name("event")) {
            if !matches!(event.attribute("name").map(str::trim), Some("effect" | "@effect")) {
                continue;
            }
            let (Some(time), Some(effect)) = (event.attribute("time").and_then(|t| t.trim().parse::<f32>().ok()), event.attribute("parameter").map(str::trim)) else { continue };
            let mut offset = [0.0; 3];
            for (slot, v) in offset.iter_mut().zip(event.attribute("offset").unwrap_or("").split(',')) {
                *slot = v.trim().parse().unwrap_or(0.0);
            }
            effects.push(ClipEffect { time, effect: effect.to_string(), bone: event.attribute("bone").unwrap_or("").trim().to_string(), offset });
        }
        if !effects.is_empty() {
            effects.sort_by(|a, b| a.time.total_cmp(&b.time));
            table.insert(file_name(name), effects.into());
        }
    }
    table
}

/// The events of the clip at `clip_path` (a pak path such as `animations/nuian/male/bow/bow_co_sk_launch.caf`), from the event
/// database `database` (the `.cal`'s `$AnimEventDatabase`). The clips are matched by file name: the databases of the monsters
/// name the folders of the base model they were copied from (`animations/animals/rat/...` for `rat_black`).
pub fn clip_events(database: &str, clip_path: &str) -> ClipEvents {
    static TABLES: OnceLock<Mutex<HashMap<String, Arc<Table>>>> = OnceLock::new();
    let key = database.to_ascii_lowercase().replace('\\', "/");
    let tables = TABLES.get_or_init(Default::default);
    let cached = tables.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
    let table = cached.unwrap_or_else(|| {
        let parsed = Arc::new(aa_cry::read(&key).ok().map(|b| parse(&String::from_utf8_lossy(&b))).unwrap_or_default());
        tables.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(parsed).clone()
    });
    table.get(&file_name(clip_path)).copied().unwrap_or_default()
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase()
}

/// Parses an `.animevents` file into the events of every clip, by file name (the first of each kind wins).
fn parse(xml: &str) -> Table {
    let mut table = Table::new();
    let Ok(doc) = roxmltree::Document::parse(xml) else { return table };
    for animation in doc.descendants().filter(|n| n.has_tag_name("animation")) {
        let Some(name) = animation.attribute("name") else { continue };
        let mut events = ClipEvents::default();
        for event in animation.children().filter(|n| n.has_tag_name("event")) {
            let Some(time) = event.attribute("time").and_then(|t| t.trim().parse::<f32>().ok()) else { continue };
            // `!skill_effect` and `skill_effect` carry the same data (the bang is the authors' marker, not a switch)
            let slot = match (event.attribute("name").map(str::trim), event.attribute("parameter").map(str::trim)) {
                (Some("skill_effect" | "!skill_effect"), Some("form")) => &mut events.form,
                (Some("skill_effect" | "!skill_effect"), Some("throw")) => &mut events.throw,
                (Some("skill_effect" | "!skill_effect"), Some("launch")) => &mut events.launch,
                (Some("skill_effect" | "!skill_effect"), Some("voice_launch")) => &mut events.voice,
                (Some("combat_sync"), _) => &mut events.combat_sync,
                (Some("aw_none"), _) => &mut events.aw_none,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(time);
            }
        }
        table.insert(file_name(name), events);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_launch_throw_form_and_sync() {
        let xml = r#"<anim_event_list>
 <animation name="animations/nuian/male/bow/bow_co_sk_launch.caf">
  <event name="combat_sync" time="0.35793" parameter="" bone="" offset="0,0,0" dir="0,0,0"/>
  <event name="!skill_effect" time="0.35793" parameter="launch" bone="item_hand_l" offset="0,0,0" dir="0,0,0"/>
  <event name="!skill_effect" time="0.092239998" parameter="throw" bone="item_hand_r" offset="0,0,0" dir="0,0,0"/>
  <event name="skill_effect" time="0.2" parameter="voice_launch" bone="Bip01 Head" offset="0,0,0" dir="0,0,0"/>
  <event name="skill_effect" time="0.5" parameter="launch" bone="" offset="0,0,0" dir="0,0,0"/>
 </animation>
 <animation name="animations/animals/rat/fist/fist_co_attack_r_01.caf">
  <event name="combat_sync" time="0.48708001" parameter="" bone="" offset="0,0,0" dir="0,0,0"/>
 </animation>
</anim_event_list>"#;
        let t = parse(xml);
        let bow = t["bow_co_sk_launch.caf"];
        assert_eq!((bow.launch, bow.combat_sync, bow.form), (Some(0.35793), Some(0.35793), None));
        assert!((bow.throw.unwrap() - 0.0922).abs() < 1e-3);
        assert_eq!(bow.release(), Some(0.35793), "the first launch event wins");
        assert_eq!((bow.voice, bow.voice_time()), (Some(0.2), Some(0.2)));
        let rat = t["fist_co_attack_r_01.caf"];
        assert_eq!((rat.launch, rat.release()), (None, Some(0.487_08)));
    }

    #[test]
    fn phases_become_seconds_with_the_clip_length() {
        // twohand_co_sk_streakattack_1: 1.71 s long, the trail forms at 0.140 (+239 ms in the original), the voice at 0.11 (+188 ms)
        let events = ClipEvents { form: Some(0.14021), voice: Some(0.11), combat_sync: Some(0.17712), ..ClipEvents::default() };
        let s = events.scaled(1.709);
        assert!((s.form.unwrap() - 0.2396).abs() < 1e-3, "{s:?}");
        assert!((s.voice.unwrap() - 0.188).abs() < 1e-3, "{s:?}");
        assert!((s.combat_sync.unwrap() - 0.3027).abs() < 1e-3, "{s:?}");
        assert_eq!((s.throw, s.launch, s.aw_none), (None, None, None));
        assert_eq!(ClipEvents::default().scaled(2.0), ClipEvents::default());
    }

    #[test]
    fn effects_of_a_clip() {
        let xml = r#"<anim_event_list>
 <animation name="animations/nuian/male/fist/fist_em_celebrate.caf">
  <event name="effect" time="0.2915" parameter="X2_EFFECTS.e_effect.flower" bone="item_hand_l" offset="0,0.5,-0.25" dir="0,0,0"/>
  <event name="combat_sync" time="0.1" parameter="" bone="" offset="0,0,0" dir="0,0,0"/>
 </animation>
 <animation name="animations/nuian/male/fist/fist_ac_yatadance.caf">
  <event name="@effect" time="0.8" parameter="a.b.c" bone="" offset="0,0,0" dir="0,0,0"/>
  <event name="@effect" time="0.1" parameter="a.b.d" bone="item_hand_l" offset="0,0,0" dir="0,0,0"/>
 </animation>
</anim_event_list>"#;
        let t = parse_effects(xml);
        let flower = &t["fist_em_celebrate.caf"];
        assert_eq!(flower.len(), 1);
        assert_eq!((flower[0].effect.as_str(), flower[0].bone.as_str(), flower[0].offset), ("X2_EFFECTS.e_effect.flower", "item_hand_l", [0.0, 0.5, -0.25]));
        // in time order
        assert_eq!(t["fist_ac_yatadance.caf"].iter().map(|e| e.effect.as_str()).collect::<Vec<_>>(), ["a.b.d", "a.b.c"]);
    }

    #[test]
    fn real_databases() {
        if !aa_cry::pak_available() {
            return;
        }
        // phases of the clips
        let bow = clip_events("animations/nuian/male/event_database.animevents", "animations/nuian/male/bow/bow_co_sk_launch.caf");
        assert!((bow.release().unwrap() - 0.358).abs() < 0.01, "{bow:?}");
        // the rat's database names the folder `rat`, its clips live in `rat_black`
        let rat = clip_events("animations/animals/rat_black/event_database.animevents", "animations/animals/rat_black/fist/fist_co_attack_r_01.caf");
        assert!((rat.combat_sync.unwrap() - 0.487).abs() < 0.01, "{rat:?}");
        // the sword leaves the hand 43 % into the holster clip
        let holster = clip_events("animations/nuian/male/event_database.animevents", "animations/nuian/male/onehand/onehand_ac_holster_side_l.caf");
        assert!((holster.aw_none.unwrap() - 0.428).abs() < 0.01, "{holster:?}");
        assert_eq!(clip_events("animations/nope.animevents", "x.caf"), ClipEvents::default());
    }
}
