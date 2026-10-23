//! Sound moods: the mixer snapshots of `libs/soundmoods/soundmoods.xml`.
//!
//! The game keeps its sound categories (`master/ambience`, `master/footsteps` ...) in the FMOD project and switches them
//! between **moods**: `default`, `combat`, `underwater`, `death`, `indoor`, `low_health` ... (17 in all). Every mood lists
//! all 31 categories with a `Volume` and a `LowPassCutoff` (a listed category without attributes is at full volume and
//! unfiltered), so a mood is a complete snapshot of the mix, and the engine plays the one with the highest `Priority`
//! of those that are active (`default` has 1, `indoor` 2, `underwater` and `low_health` 8, `combat` 10, `death` 25,
//! `loading` and `quest_camera` 50). Each mood also has a `MusicVolume` (0.8 for most).
//!
//! What is done with a mood here ([`Mix`]): a category's `Volume` multiplies the sounds of events in that mixer category (so
//! `ambience_inside` and `underwater_only` are silent in the default mood, as they are in the original, and `ambience` is silent
//! indoors); `MusicVolume` multiplies the music categories; a `LowPassCutoff` becomes a low-pass filter on the volume
//! group the category belongs to (one cutoff per group, taken from the group's main category: `ambience`, `skill`,
//! `player_voice`, `music`); a cutoff of 10 Hz means the category is off and is read as the volume 0 it comes with.
//! Not done: `Pitch` (the weapons are 0.09 octaves down under water), high-pass and the master EQ.

use std::collections::HashMap;

use crate::engine::Category;

/// Path of the mood file in the pak.
pub const MOOD_FILE: &str = "game/libs/soundmoods/soundmoods.xml";

/// The mix of one category in a mood.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CategoryMix {
    pub volume: f32,
    /// Hz, `None` for no filter.
    pub lowpass: Option<f32>,
}

impl Default for CategoryMix {
    fn default() -> Self {
        CategoryMix { volume: 1.0, lowpass: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mood {
    pub name: String,
    pub priority: i32,
    pub music_volume: f32,
    /// By the last part of the category path (`footsteps`).
    pub categories: HashMap<String, CategoryMix>,
}

#[derive(Clone, Debug, Default)]
pub struct MoodTable {
    moods: Vec<Mood>,
}

impl MoodTable {
    pub fn parse(xml: &str) -> MoodTable {
        let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else { return MoodTable::default() };
        let moods = doc
            .root_element()
            .children()
            .filter(|n| n.has_tag_name("Mood"))
            .map(|m| Mood {
                name: m.attribute("Name").unwrap_or_default().to_string(),
                priority: m.attribute("Priority").and_then(|v| v.parse().ok()).unwrap_or(0),
                music_volume: m.attribute("MusicVolume").and_then(|v| v.parse().ok()).unwrap_or(1.0),
                categories: m
                    .children()
                    .filter(|c| c.has_tag_name("Category"))
                    .map(|c| {
                        let volume = c.attribute("Volume").and_then(|v| v.parse().ok()).unwrap_or(1.0);
                        // 10 Hz is how the file says "off": the volume is 0 then
                        let lowpass = c.attribute("LowPassCutoff").and_then(|v| v.parse::<f32>().ok()).filter(|&hz| hz > 20.0);
                        (c.attribute("Name").unwrap_or_default().to_string(), CategoryMix { volume, lowpass })
                    })
                    .collect(),
            })
            .collect();
        MoodTable { moods }
    }

    pub fn load(pak: &aa_pak::Pak) -> MoodTable {
        pak.read(MOOD_FILE).map(|b| MoodTable::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
    }

    pub fn get(&self, name: &str) -> Option<&Mood> {
        self.moods.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    pub fn len(&self) -> usize {
        self.moods.len()
    }

    pub fn is_empty(&self) -> bool {
        self.moods.is_empty()
    }

    /// The mood that wins among `active` (the highest priority; `default` when none is known).
    pub fn winner<'a>(&'a self, active: &[String]) -> Option<&'a Mood> {
        active.iter().filter_map(|n| self.get(n)).max_by_key(|m| m.priority).or_else(|| self.get("default"))
    }
}

/// The category whose cutoff stands for a volume group.
fn main_category(group: Category) -> Option<&'static str> {
    match group {
        Category::Music => Some("music"),
        Category::Sfx => Some("skill"),
        Category::Ambience => Some("ambience"),
        Category::Voice => Some("player_voice"),
        Category::Ui | Category::UserMusic => None,
    }
}

/// What the active moods do to the mix, ready for the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct Mix {
    pub name: String,
    pub music_volume: f32,
    volumes: HashMap<String, f32>,
    /// Low-pass cutoff of each group (Hz), `None` for open.
    lowpass: HashMap<Category, f32>,
}

impl Default for Mix {
    fn default() -> Self {
        Mix { name: String::new(), music_volume: 1.0, volumes: HashMap::new(), lowpass: HashMap::new() }
    }
}

impl Mix {
    pub fn of(mood: &Mood) -> Mix {
        let volumes = mood.categories.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.volume)).collect();
        let mut lowpass = HashMap::new();
        for group in [Category::Music, Category::Sfx, Category::Ambience, Category::Voice] {
            if let Some(hz) = main_category(group).and_then(|c| mood.categories.get(c)).and_then(|c| c.lowpass) {
                lowpass.insert(group, hz);
            }
        }
        Mix { name: mood.name.clone(), music_volume: mood.music_volume, volumes, lowpass }
    }

    /// The gain of events in the mixer category `leaf` (`footsteps`, `music_battle` ...): its volume, and the mood's
    /// music volume for the music categories.
    pub fn gain(&self, leaf: &str) -> f32 {
        let volume = self.volumes.get(leaf).copied().unwrap_or(1.0);
        if leaf.starts_with("music") { volume * self.music_volume } else { volume }
    }

    /// Cutoff in Hz of the group's low-pass filter, `None` when open.
    pub fn lowpass(&self, group: Category) -> Option<f32> {
        self.lowpass.get(&group).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<SoundMoods>
 <Mood Name="underwater" Priority="8" MusicVolume="0.8">
  <Category Name="ambience" Volume="0.1" LowPassCutoff="300"/>
  <Category Name="footsteps" LowPassCutoff="300"/>
  <Category Name="skill" LowPassCutoff="2000"/>
  <Category Name="music"/>
 </Mood>
 <Mood Name="default" Priority="1" MusicVolume="0.8">
  <Category Name="ambience_inside" Volume="0" LowPassCutoff="10"/>
  <Category Name="ambience"/>
  <Category Name="music"/>
 </Mood>
 <Mood Name="combat" Priority="10" MusicVolume="0.8">
  <Category Name="music" Volume="0" LowPassCutoff="10"/>
  <Category Name="music_battle"/>
  <Category Name="ambience_inside"/>
 </Mood>
</SoundMoods>"#;

    #[test]
    fn the_highest_priority_mood_is_the_mix() {
        let t = MoodTable::parse(XML);
        assert_eq!(t.len(), 3);
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(t.winner(&names(&["underwater", "default"])).unwrap().name, "underwater");
        assert_eq!(t.winner(&names(&["underwater", "combat", "default"])).unwrap().name, "combat");
        assert_eq!(t.winner(&names(&[])).unwrap().name, "default");
        assert_eq!(t.winner(&names(&["nonsense"])).unwrap().name, "default");
    }

    #[test]
    fn a_mix_has_volumes_cutoffs_and_music_volume() {
        let t = MoodTable::parse(XML);
        let dflt = Mix::of(t.get("default").unwrap());
        assert_eq!(dflt.gain("ambience_inside"), 0.0, "cave ambience is off outdoors");
        assert_eq!(dflt.gain("ambience"), 1.0);
        assert_eq!(dflt.gain("footsteps"), 1.0, "a category the mood does not list is untouched");
        assert!((dflt.gain("music") - 0.8).abs() < 1e-6, "music volume");
        assert_eq!(dflt.lowpass(Category::Ambience), None);
        let uw = Mix::of(t.get("underwater").unwrap());
        assert!((uw.gain("ambience") - 0.1).abs() < 1e-6);
        assert_eq!(uw.lowpass(Category::Ambience), Some(300.0));
        assert_eq!(uw.lowpass(Category::Sfx), Some(2000.0));
        assert_eq!(uw.lowpass(Category::Music), None);
        assert_eq!(uw.lowpass(Category::UserMusic), None);
        // 10 Hz is "off", which the volume says
        let combat = Mix::of(t.get("combat").unwrap());
        assert_eq!(combat.gain("music"), 0.0);
        assert_eq!(combat.lowpass(Category::Music), None);
        assert!((combat.gain("music_battle") - 0.8).abs() < 1e-6);
    }
}
