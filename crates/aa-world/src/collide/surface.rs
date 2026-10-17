//! Surface types of the collision: the physical material of a triangle (`mat_wood`, `mat_rock`, ...), for footsteps
//! and impact effects.
//!
//! A surface is the `SurfaceType` of a sub-material of a `.mtl`, the names of `libs/materialeffects/surfacetypes.xml`
//! and the columns of `materialeffects.xml` (`mat_dirt` is the column of `footstep_nuian_walk` for the dirt). The
//! triangles of the collision keep a small id into a table of the names seen so far; [`Surface`] is the handle the
//! queries return.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use aa_cry::material::Material;

/// Id of `mat_default`, the surface of everything that names none.
pub const DEFAULT: u16 = 0;

/// A surface type by name, `mat_wood`. Cheap to copy and compare; the names are the lower case ones of the
/// `.mtl` files (see [`Surface::name`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Surface(&'static str);

impl Surface {
    /// What nothing names: the `mat_default` column of the footstep sheet.
    pub const DEFAULT: Surface = Surface("mat_default");

    /// The surface with this name (any case); the handle is the same for every call.
    pub fn named(name: &str) -> Surface {
        Surface(name_of(id_of(name)))
    }

    /// Lower case name, `mat_wood`, as the columns of `materialeffects.xml` spell it.
    pub fn name(self) -> &'static str {
        self.0
    }

    pub(super) fn from_id(id: u16) -> Surface {
        Surface(name_of(id))
    }

    /// Coarse class of the name, for code that only tells wood from stone.
    pub fn kind(self) -> SurfaceKind {
        SurfaceKind::of(self.0)
    }
}

impl Default for Surface {
    fn default() -> Self {
        Surface::DEFAULT
    }
}

impl fmt::Debug for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// What a [`Surface`] is made of, roughly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceKind {
    /// `mat_default` and the names this does not know.
    Other,
    Dirt,
    Mud,
    Sand,
    Grass,
    Stone,
    Wood,
    Metal,
    Water,
    Snow,
    Ice,
    Fabric,
}

impl SurfaceKind {
    /// Class of a surface name (`mat_wood_solid` is wood, `mat_rubble_stone` stone, `mat_sand_wet` sand).
    pub fn of(name: &str) -> SurfaceKind {
        let n = name.strip_prefix("mat_").unwrap_or(name);
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        if has(&["water"]) {
            SurfaceKind::Water
        } else if has(&["wood", "tree", "twigs", "wheel"]) {
            SurfaceKind::Wood
        } else if has(&["metal", "kevlar"]) {
            SurfaceKind::Metal
        } else if has(&["rock", "rubble", "concrete", "tile", "pebble", "gravel", "road", "asphalt", "stone", "glass"]) {
            SurfaceKind::Stone
        } else if has(&["sand"]) {
            SurfaceKind::Sand
        } else if has(&["mud", "wet"]) {
            SurfaceKind::Mud
        } else if has(&["dirt", "soil"]) {
            SurfaceKind::Dirt
        } else if has(&["grass", "leaves", "canopy", "vegetation", "feathers"]) {
            SurfaceKind::Grass
        } else if has(&["snow"]) {
            SurfaceKind::Snow
        } else if has(&["ice"]) {
            SurfaceKind::Ice
        } else if has(&["fabric", "canvas", "rubber", "plastic"]) {
            SurfaceKind::Fabric
        } else {
            SurfaceKind::Other
        }
    }
}

// ---------------------------------------------------------------- the table of names

/// Most distinct names kept (a malformed file cannot grow the table without bound; the real data has ~60).
const MAX_NAMES: usize = 1024;

struct Table {
    names: Vec<&'static str>,
    ids: HashMap<&'static str, u16>,
}

fn table() -> &'static Mutex<Table> {
    static TABLE: OnceLock<Mutex<Table>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(Table { names: vec!["mat_default"], ids: HashMap::from([("mat_default", DEFAULT)]) }))
}

/// Id of a surface name (any case); an empty name is `mat_default`.
pub fn id_of(name: &str) -> u16 {
    let name = name.trim();
    if name.is_empty() {
        return DEFAULT;
    }
    let mut t = table().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(&id) = t.ids.get(name) {
        return id;
    }
    let lower = name.to_ascii_lowercase();
    if let Some(&id) = t.ids.get(lower.as_str()) {
        return id;
    }
    if t.names.len() >= MAX_NAMES {
        return DEFAULT;
    }
    let leaked: &'static str = Box::leak(lower.into_boxed_str());
    let id = t.names.len() as u16;
    t.names.push(leaked);
    t.ids.insert(leaked, id);
    id
}

pub fn name_of(id: u16) -> &'static str {
    table().lock().unwrap_or_else(PoisonError::into_inner).names.get(id as usize).copied().unwrap_or("mat_default")
}

/// Surfaces the physics lets through (`no_collide` in `surfacetypes.xml`: canopy, leaves, grass, ...), plus the
/// ones that only block the AI.
fn no_collide() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set: HashSet<String> = ["mat_obstruct".to_string()].into();
        let xml = aa_pak::Pak::global().read_string("libs/materialeffects/surfacetypes.xml").unwrap_or_default();
        if let Ok(doc) = roxmltree::Document::parse(&xml) {
            for s in doc.descendants().filter(|n| n.has_tag_name("SurfaceType")) {
                let blocked = s.children().any(|p| p.has_tag_name("Physics") && p.attribute("no_collide") == Some("1"));
                if let (true, Some(name)) = (blocked, s.attribute("name")) {
                    set.insert(name.to_ascii_lowercase());
                }
            }
        }
        set
    })
}

/// Whether things of this surface stop a walker.
pub fn collides(name: &str) -> bool {
    !no_collide().contains(&name.to_ascii_lowercase())
}

// ---------------------------------------------------------------- materials

/// What the sub-materials of a `.mtl` mean for collision.
#[derive(Debug, Default, PartialEq)]
pub struct SubSurfaces {
    /// `SurfaceType` of the material itself (the terrain layers are single materials).
    pub root: u16,
    /// Per sub-material: its surface id, `None` when triangles of it do not collide (`nocollide`, canopy, grass).
    pub subs: Vec<Option<u16>>,
}

impl SubSurfaces {
    pub fn from_material(m: &Material) -> SubSurfaces {
        let one = |m: &Material| {
            let name = m.surface_type.trim().to_ascii_lowercase();
            if m.name.to_ascii_lowercase().contains("nocollide") || !collides(&name) { None } else { Some(id_of(&name)) }
        };
        let subs = if m.sub_materials.is_empty() { vec![one(m)] } else { m.sub_materials.iter().map(one).collect() };
        SubSurfaces { root: id_of(&m.surface_type), subs }
    }

    /// Surface of a sub-material id: `Some(id)` collides, `None` does not. Ids past the end are `mat_default`.
    pub fn get(&self, sub: u16) -> Option<u16> {
        self.subs.get(sub as usize).copied().unwrap_or(Some(DEFAULT))
    }
}

/// `.mtl` files read so far, by pak path (`None`: missing or unreadable).
#[derive(Default)]
pub struct Materials {
    by_path: Mutex<HashMap<String, Option<Arc<SubSurfaces>>>>,
}

impl Materials {
    /// The surfaces of the first of `paths` that exists in the pak.
    pub fn first_of(&self, paths: &[String]) -> Option<Arc<SubSurfaces>> {
        let pak = aa_pak::Pak::global();
        let path = paths.iter().find(|p| pak.exists(p))?;
        self.load(path)
    }

    pub fn load(&self, path: &str) -> Option<Arc<SubSurfaces>> {
        let key = aa_pak::normalize(path);
        if let Some(found) = self.by_path.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
            return found.clone();
        }
        // read outside the lock; two threads may both read a new file, which is harmless
        let loaded = Material::load(&key).ok().map(|m| Arc::new(SubSurfaces::from_material(&m)));
        self.by_path.lock().unwrap_or_else(PoisonError::into_inner).insert(key, loaded.clone());
        loaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_interned() {
        let a = Surface::named("Mat_Wood");
        assert_eq!(a, Surface::named("mat_wood"));
        assert_eq!(a.name(), "mat_wood");
        assert_eq!(Surface::named(""), Surface::DEFAULT);
        assert_eq!(id_of("mat_default"), DEFAULT);
        assert_ne!(Surface::named("mat_rock"), a);
    }

    #[test]
    fn kinds() {
        for (name, kind) in [
            ("mat_wood_solid", SurfaceKind::Wood),
            ("mat_tree", SurfaceKind::Wood),
            ("mat_rock_dusty", SurfaceKind::Stone),
            ("mat_rubble_stone", SurfaceKind::Stone),
            ("mat_dirt", SurfaceKind::Dirt),
            ("mat_sand_wet", SurfaceKind::Sand),
            ("mat_metal_thick", SurfaceKind::Metal),
            ("mat_water_shallow", SurfaceKind::Water),
            ("mat_grass_tall", SurfaceKind::Grass),
            ("mat_default", SurfaceKind::Other),
        ] {
            assert_eq!(SurfaceKind::of(name), kind, "{name}");
        }
    }

    #[test]
    fn materials_say_what_collides() {
        let text = r#"<Material><SubMaterials>
            <Material Name="proxy" SurfaceType="mat_wood"/>
            <Material Name="leaves" SurfaceType="mat_leaves"/>
            <Material Name="nocollide" SurfaceType=""/>
            <Material Name="plain" SurfaceType=""/>
            </SubMaterials></Material>"#;
        let m = Material::parse(text).unwrap();
        let s = SubSurfaces::from_material(&m);
        // the table needs the pak for the list of surfaces that let things through; the AI-only one is built in
        assert_eq!(s.subs[0], Some(id_of("mat_wood")));
        assert_eq!(s.subs[2], None);
        assert_eq!(s.subs[3], Some(DEFAULT));
        assert_eq!(s.get(9), Some(DEFAULT));
        if crate::pak_available() {
            assert_eq!(s.subs[1], None, "leaves do not collide");
            assert!(!collides("mat_canopy") && !collides("mat_grass") && collides("mat_rock") && collides("mat_wood"));
        }
    }
}
