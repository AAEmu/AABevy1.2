//! Saved looks ("Save / Load Appearance"), and the database lookups for the race page (race
//! advantages) that need the customizer's connection.

use std::path::{Path, PathBuf};

use aa_anim::db::model_id;
use aa_anim::{Gender, Race};

use super::look::{Customizer, Look, MODIFIER_LEN, Scar};

fn pad(m: &[u8]) -> Vec<u8> {
    let mut v = m.to_vec();
    v.resize(MODIFIER_LEN, 0);
    v
}

impl Look {
    /// A plain `key value` text of the look (and the model it belongs to).
    pub fn to_text(&self, race: Race, gender: Gender) -> String {
        let hex: String = self.modifier.iter().map(|b| format!("{b:02x}")).collect();
        let mut s = format!("race {}\ngender {}\n", race.folder(), gender.folder());
        let mut line = |k: &str, v: String| {
            s.push_str(k);
            s.push(' ');
            s.push_str(&v);
            s.push('\n');
        };
        line("hair", self.hair.to_string());
        line("hair_color", self.hair_color.to_string());
        line("skin", self.skin.to_string());
        line("wrinkle", format!("{} {}", self.wrinkle.0, self.wrinkle.1));
        line("eyebrow", self.eyebrow.to_string());
        line("makeup", format!("{} {}", self.makeup.0, self.makeup.1));
        line("deco", format!("{} {}", self.deco.0, self.deco.1));
        line("tattoo", self.tattoo.to_string());
        let sc = self.scar;
        line("scar", format!("{} {} {} {} {}", sc.index, sc.x, sc.y, sc.scale, sc.rotate));
        line("eyebrow_color", format!("{:08x}", self.eyebrow_color));
        line("pupil_color", format!("{:08x}", self.pupil_color));
        line("lip_color", format!("{:08x}", self.lip_color));
        line("deco_color", format!("{:08x}", self.deco_color));
        line("modifier", hex);
        s
    }

    /// The inverse of [`Look::to_text`]: `(race, gender, look)`.
    pub fn from_text(text: &str) -> Option<(Race, Gender, Look)> {
        let mut race = None;
        let mut gender = None;
        let mut look = Look {
            hair: 0,
            hair_color: 1,
            skin: 1,
            wrinkle: (0, 1.0),
            eyebrow: 0,
            makeup: (0, 1.0),
            deco: (0, 1.0),
            tattoo: 0,
            scar: Scar::default(),
            eyebrow_color: 0,
            pupil_color: 0,
            lip_color: 0,
            deco_color: 0,
            modifier: vec![0; MODIFIER_LEN],
            smile: false,
        };
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let Some(key) = it.next() else { continue };
            let v: Vec<&str> = it.collect();
            let u = |i: usize| v.get(i).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
            let f = |i: usize, d: f32| v.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(d);
            let c = |i: usize| v.get(i).and_then(|s| u32::from_str_radix(s, 16).ok()).unwrap_or(0);
            match key {
                "race" => {
                    race = v.first().map(|s| match *s {
                        "elf" => Race::Elf,
                        "ferre" => Race::Ferre,
                        "hariharan" => Race::Hariharan,
                        _ => Race::Nuian,
                    })
                }
                "gender" => gender = v.first().map(|s| if *s == "female" { Gender::Female } else { Gender::Male }),
                "hair" => look.hair = u(0),
                "hair_color" => look.hair_color = u(0).max(1),
                "skin" => look.skin = u(0).max(1),
                "wrinkle" => look.wrinkle = (u(0), f(1, 1.0)),
                "eyebrow" => look.eyebrow = u(0),
                "makeup" => look.makeup = (u(0), f(1, 1.0)),
                "deco" => look.deco = (u(0), f(1, 1.0)),
                "tattoo" => look.tattoo = u(0),
                "scar" => look.scar = Scar { index: u(0), x: f(1, 0.0), y: f(2, 0.0), scale: f(3, 1.0), rotate: f(4, 0.0) },
                "eyebrow_color" => look.eyebrow_color = c(0),
                "pupil_color" => look.pupil_color = c(0),
                "lip_color" => look.lip_color = c(0),
                "deco_color" => look.deco_color = c(0),
                "modifier" => {
                    let hex = v.first().copied().unwrap_or("");
                    let bytes: Vec<u8> =
                        (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect();
                    look.modifier = pad(&bytes);
                }
                _ => {}
            }
        }
        Some((race?, gender?, look))
    }
}

/// Where the looks are saved: `Documents/AABevy/looks` (`AA_LOOKS_DIR` overrides).
pub fn looks_dir() -> PathBuf {
    if let Ok(d) = std::env::var("AA_LOOKS_DIR") {
        return d.into();
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    Path::new(&home).join("Documents").join("AABevy").join("looks")
}

/// The saved looks, newest first: `(name, path)`.
pub fn saved_looks() -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(looks_dir()) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "look"))
        .filter_map(|e| {
            let t = e.metadata().ok()?.modified().ok()?;
            Some((t, e.path().file_stem()?.to_string_lossy().into_owned(), e.path()))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, n, p)| (n, p)).collect()
}

fn file_name(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric() || " _-".contains(*c)).collect()
}

impl Customizer {
    /// Saves the look under `name`; false when a file of that name exists and `overwrite` is not set.
    pub fn save_look(&self, name: &str, overwrite: bool) -> std::io::Result<bool> {
        let dir = looks_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.look", file_name(name)));
        if path.exists() && !overwrite {
            return Ok(false);
        }
        std::fs::write(path, self.look.to_text(self.cat.race, self.cat.gender))?;
        Ok(true)
    }

    pub fn look_exists(name: &str) -> bool {
        looks_dir().join(format!("{}.look", file_name(name))).exists()
    }

    /// Loads a saved look of the current model; false when it is another race / gender.
    pub fn load_look(&mut self, path: &Path) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else { return false };
        let Some((race, gender, look)) = Look::from_text(&text) else { return false };
        if (race, gender) != (self.cat.race, self.cat.gender) {
            return false;
        }
        self.look = look;
        self.revision += 1;
        true
    }

    /// Buffs of the race (`character_buffs`): the advantages shown on the race page.
    pub fn race_buffs(&self, race: Race, gender: Gender) -> Vec<u32> {
        let model = model_id(race, gender);
        let Ok(mut st) = self.conn().prepare(
            "SELECT cb.buff_id FROM character_buffs cb JOIN characters c ON c.id = cb.character_id WHERE c.model_id = ?1 ORDER BY cb.id",
        ) else {
            return Vec::new();
        };
        st.query_map([model], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// `(name, icon path, description)` of a buff in `locale` (`en_us`, ...).
    pub fn buff_info(&self, id: u32, locale: &str) -> Option<(String, String, String)> {
        let col = match locale {
            "ko" | "en_us" | "zh_cn" | "ja" | "ru" | "de" | "fr" => locale,
            _ => "en_us",
        };
        let sql = format!(
            "SELECT COALESCE(ln.{col}, b.name), COALESCE(ic.filename, ''), COALESCE(ld.{col}, b.desc, '') FROM buffs b \
             LEFT JOIN icons ic ON ic.id = b.icon_id \
             LEFT JOIN localized_texts ln ON ln.tbl_name = 'buffs' AND ln.tbl_column_name = 'name' AND ln.idx = b.id \
             LEFT JOIN localized_texts ld ON ld.tbl_name = 'buffs' AND ld.tbl_column_name = 'desc' AND ld.idx = b.id \
             WHERE b.id = ?1"
        );
        self.conn()
            .query_row(&sql, [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .ok()
            .map(|(n, i, d)| (n, if i.is_empty() { i } else { format!("ui/icon/{i}") }, d))
    }
}
