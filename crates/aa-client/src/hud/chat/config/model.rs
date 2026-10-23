//! Eight global native tab IDs, ordered per-window rows and separately saved window extras.
use serde_json::{Value as Json, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::OnceLock,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tab {
    pub id: i32,
    pub window: i32,
    pub name: String,
    pub font: i32,
    pub background: [f32; 4],
    pub filters: [bool; 200],
    pub colors: [[f32; 4]; 200],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Extra {
    pub selected: i32,
    pub method: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Model {
    pub tabs: BTreeMap<i32, Tab>,
    /// Current append order; native disk records are separately saved in ascending raw-ID order.
    pub windows: BTreeMap<i32, Vec<i32>>,
    pub extra: BTreeMap<i32, Extra>,
}

fn source() -> &'static Json {
    static VALUE: OnceLock<Json> = OnceLock::new();
    VALUE.get_or_init(|| serde_json::from_str(include_str!("native_defaults.json")).expect("checked native chat defaults"))
}

pub(super) fn valid_name(name: &str) -> bool {
    let name = name.split('\0').next().unwrap_or_default();
    (1..=10).contains(&name.chars().count())
}

impl Tab {
    pub fn new(id: i32, window: i32, name: String, font: i32) -> Self {
        let colors: Vec<[f32; 4]> = serde_json::from_value(source()["palette"].clone()).unwrap();
        Self {
            id,
            window,
            name,
            font,
            background: [0., 0., 0., 1.],
            filters: [false; 200],
            colors: colors.try_into().unwrap(),
        }
    }

    pub fn defaults(&self, font: i32) -> Self {
        let mut out = Self::new(self.id, self.window, self.name.clone(), font);
        let kind = match self.id {
            0 => "general",
            1 => "party",
            2 => "trade",
            _ => return out,
        };
        for id in source()["filters"][kind].as_array().unwrap() {
            out.filters[id.as_u64().unwrap() as usize] = true;
        }
        out
    }

    pub fn valid(&self) -> bool {
        (0..8).contains(&self.id) && (0..8).contains(&self.window) && valid_name(&self.name)
    }

    pub fn widget_info(&self) -> aa_ui::lua::chat::ChatTabInfo {
        aa_ui::lua::chat::ChatTabInfo {
            id: i64::from(self.id),
            name: self.name.clone(),
            filters: Some(
                self.filters
                    .iter()
                    .enumerate()
                    .filter_map(|(id, &on)| on.then_some(id as i64))
                    .collect(),
            ),
            font_size: Some(self.font as f32),
            background: Some(self.background),
            filter_colors: self.colors.iter().enumerate().map(|(id, &color)| (id as i64, color)).collect(),
        }
    }
}

impl Model {
    /// A detached tab keeps its recovery record, but has no current input-method owner.
    pub fn current_window(&self, raw: i32) -> Option<i32> {
        let window = self.tabs.get(&raw)?.window;
        self.windows.get(&window)?.contains(&raw).then_some(window)
    }

    pub fn defaults(names: [String; 3], font: i32) -> Self {
        let mut out = Self::default();
        for (id, name) in names.into_iter().enumerate() {
            let tab = Tab::new(id as i32, 0, name, font).defaults(font);
            out.tabs.insert(tab.id, tab);
        }
        out.windows.insert(0, vec![0, 1, 2]);
        out.extra.insert(0, Extra::default());
        out
    }

    pub fn allocate(&mut self, window: i32, name: &str, font: i32) -> Option<i32> {
        if !(0..8).contains(&window) || !valid_name(name) {
            return None;
        }
        let id = (0..8).find(|id| !self.tabs.contains_key(id))?;
        self.tabs
            .insert(id, Tab::new(id, window, name.split('\0').next().unwrap().into(), font));
        self.windows.entry(window).or_default().push(id);
        Some(id)
    }

    pub fn rename(&mut self, id: i32, name: &str) -> bool {
        if !valid_name(name) {
            return false;
        }
        let Some(tab) = self.tabs.get_mut(&id) else { return false };
        tab.name = name.split('\0').next().unwrap().into();
        true
    }

    pub fn replace(&mut self, tab: Tab) -> bool {
        if !tab.valid() || self.tabs.get(&tab.id).is_none_or(|old| old.window != tab.window) {
            return false;
        }
        self.tabs.insert(tab.id, tab);
        true
    }

    /// Widget removal is deliberately separate and occurs before this native model guard.
    pub fn remove(&mut self, id: i32) -> bool {
        if id == 0 {
            return false;
        }
        let Some(tab) = self.tabs.remove(&id) else { return false };
        if let Some(tabs) = self.windows.get_mut(&tab.window) {
            tabs.retain(|&other| other != id);
        }
        true
    }

    pub fn clean_empty(&mut self) -> bool {
        let ids = self
            .windows
            .iter()
            .filter_map(|(&id, tabs)| tabs.is_empty().then_some(id))
            .collect::<Vec<_>>();
        for id in &ids {
            self.windows.remove(id);
            self.extra.remove(id);
        }
        !ids.is_empty()
    }

    pub fn tabs_json(&self) -> Json {
        Json::Array(
            self.tabs
                .values()
                .map(|tab| {
                    json!({
                        "id":tab.id,"window":tab.window,"name":tab.name,"font":tab.font,"background":tab.background.map(f32::to_bits),
                        "filters":tab.filters.iter().enumerate().filter_map(|(id,&on)|on.then_some(id)).collect::<Vec<_>>(),
                        "colors":tab.colors.iter().flatten().map(|v| format!("{:08x}", v.to_bits())).collect::<String>(),
                    })
                })
                .collect(),
        )
    }

    pub fn extra_json(&self) -> Json {
        Json::Array(
            self.extra
                .iter()
                .map(|(&id, extra)| json!({"id":id,"selected":extra.selected,"method":extra.method}))
                .collect(),
        )
    }

    /// Tab records admit atomically; extras load independently. All200 palette/filter fields survive.
    pub fn load(tabs: &Json, extra: Option<&Json>) -> Option<Self> {
        let rows = tabs.as_array()?;
        if rows.is_empty() || rows.len() > 8 {
            return None;
        }
        let mut out = Self::default();
        for row in rows {
            let integer = |key| i32::try_from(row.get(key)?.as_i64()?).ok();
            let id = integer("id")?;
            let window = integer("window")?;
            let mut tab = Tab::new(id, window, row.get("name")?.as_str()?.to_string(), integer("font")?);
            if !tab.valid() || tab.name.contains('\0') || out.tabs.contains_key(&id) {
                return None;
            }
            let background: [u32; 4] = serde_json::from_value(row.get("background")?.clone()).ok()?;
            tab.background = background.map(f32::from_bits);
            let colors = row.get("colors")?.as_str()?;
            if colors.len() != 200 * 4 * 8 {
                return None;
            }
            let bits = colors
                .as_bytes()
                .chunks_exact(8)
                .map(|part| u32::from_str_radix(std::str::from_utf8(part).ok()?, 16).ok())
                .collect::<Option<Vec<_>>>()?;
            tab.colors = std::array::from_fn(|row| std::array::from_fn(|channel| f32::from_bits(bits[row * 4 + channel])));
            let mut filters = HashSet::new();
            for filter in row.get("filters")?.as_array()? {
                let id = usize::try_from(filter.as_u64()?).ok()?;
                if id >= 200 || !filters.insert(id) {
                    return None;
                }
                tab.filters[id] = true;
            }
            out.windows.entry(window).or_default().push(id);
            out.tabs.insert(id, tab);
        }
        out.load_extra(extra);
        Some(out)
    }

    /// Native loads extras even when tab loading failed and defaults were constructed.
    pub fn load_extra(&mut self, extra: Option<&Json>) {
        let parsed = (|| {
            let rows = extra?;
            let rows = rows.as_array()?;
            if rows.len() > 8 {
                return None;
            }
            let mut out = BTreeMap::new();
            for row in rows {
                let id = i32::try_from(row.get("id")?.as_i64()?).ok()?;
                if !(0..8).contains(&id) || out.contains_key(&id) {
                    return None;
                }
                out.insert(
                    id,
                    Extra {
                        selected: i32::try_from(row.get("selected")?.as_i64()?).ok()?,
                        method: i32::try_from(row.get("method")?.as_i64()?).ok()?,
                    },
                );
            }
            Some(out)
        })();
        self.extra = parsed.unwrap_or_else(|| BTreeMap::from([(0, Extra::default())]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Model {
        Model::defaults(["All".into(), "Party".into(), "Trade".into()], 15)
    }
    #[test]
    fn native_defaults_global_allocation_and_raw_id_reuse_preserve_append_order() {
        let mut m = model();
        assert_eq!(m.tabs[&0].filters.iter().filter(|&&on| on).count(), 41);
        assert!(m.tabs[&0].filters[60]);
        assert_eq!(m.tabs[&1].filters.iter().filter(|&&on| on).count(), 10);
        assert_eq!(m.tabs[&2].filters.iter().filter(|&&on| on).count(), 6);
        assert!(!m.remove(0));
        assert!(m.remove(1));
        assert_eq!(m.allocate(0, "Harbor", 15), Some(1));
        assert_eq!(m.windows[&0], [0, 2, 1]);
        assert!(!m.tabs[&1].filters.iter().any(|&on| on));
        for id in 3..8 {
            assert_eq!(m.allocate(1, "Crew", 15), Some(id));
        }
        assert!(m.allocate(0, "Full", 15).is_none());
        assert!(!m.rename(1, ""));
        assert!(m.rename(1, "X"));
        assert_eq!(m.tabs[&1].defaults(15).name, "X");
        assert_eq!(m.tabs[&1].defaults(15).filters.iter().filter(|&&on| on).count(), 10);
        let loaded = Model::load(&m.tabs_json(), Some(&m.extra_json())).unwrap();
        assert_eq!(
            loaded.windows[&0],
            [0, 1, 2],
            "native ascending raw-ID save changes append order after reload"
        );
        assert_eq!(loaded.tabs, m.tabs);
    }
    #[test]
    fn saved_200_entry_metadata_and_signed_extra_survive_atomic_reload() {
        let mut m = model();
        m.tabs.get_mut(&1).unwrap().filters[199] = true;
        m.tabs.get_mut(&1).unwrap().colors[199] = [0.125, 0.25, 0.5, -0.0];
        m.extra.insert(0, Extra { selected: -1, method: -6 });
        assert_eq!(Model::load(&m.tabs_json(), Some(&m.extra_json())), Some(m.clone()));
        let mut bad = m.tabs_json();
        bad[1]["id"] = json!(0);
        assert!(Model::load(&bad, None).is_none());
        let mut bad = m.tabs_json();
        bad[1]["filters"] = json!([200]);
        assert!(Model::load(&bad, None).is_none());
    }

    #[test]
    fn extras_load_independently_without_invalidating_valid_tab_metadata() {
        let mut m = model();
        m.rename(1, "Crew");
        for extra in [
            json!(null),
            json!({}),
            json!([{"id":0,"selected":1}]),
            json!([{"id":0,"selected":1,"method":4},{"id":0,"selected":2,"method":7}]),
        ] {
            let loaded = Model::load(&m.tabs_json(), Some(&extra)).unwrap();
            assert_eq!(loaded.tabs, m.tabs);
            assert_eq!(loaded.extra, BTreeMap::from([(0, Extra::default())]));
        }
        let mut defaults = model();
        defaults.load_extra(Some(&json!([{"id":0,"selected":2,"method":4}])));
        assert_eq!(defaults.extra[&0], Extra { selected: 2, method: 4 });
        defaults.load_extra(Some(&json!([])));
        assert!(defaults.extra.is_empty(), "an admitted empty record is distinct from a missing key");
    }
}
