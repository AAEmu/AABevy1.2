//! Which scripts the client runs for each screen, in order.
//!
//! Recovered from `x2game.dll` (r208022, unpacked copy in `<client12-unpacked>`):
//!
//! * `InitUi` (once, when the UI script state is created) runs
//!   `scripts/x2ui/baselib/locale_helper.lua`.
//! * `UIContentsManager::LoadBaseLib` loads the contents `logic`, `baselib`, `components`,
//!   `option` (each = its `toc.g`), in that order. Every screen starts with it.
//! * `StartScreenLogin` / `…WorldSelect` / `…CharacterSelect` / `…CharacterCreate`: LoadBaseLib, then
//!   the content `loginstage/<screen>`; character select also loads `secondpassword`.
//! * `StartScreenWorld`: LoadBaseLib, `scripts/x2ui/ui_management.lua`, then
//!   `UIContentsManager::LoadAllContents` (every directory of `scriptsbin/x2ui` that has a `toc.g`,
//!   in file-system find order; we use alphabetical order and skip the four base contents), then
//!   twelve single files (`npc_common.lua` … `renew_item.lua`, [`WORLD_ROOT_SCRIPTS`]).
//! * The loading screen (`LoadGlobalUI`) is the content `loading` of `scriptsbin/globalui`.

use anyhow::Result;

use crate::toc::toc_files;

/// Contents loaded by `LoadBaseLib`, in order.
pub const BASE_CONTENTS: [&str; 4] = ["logic", "baselib", "components", "option"];

/// Run once when the UI state is created (`InitUi`).
pub const INIT_SCRIPT: &str = "scriptsbin/x2ui/baselib/locale_helper.alb";

/// Run by `StartScreenWorld` right after `LoadBaseLib`.
pub const WORLD_FIRST_SCRIPT: &str = "scriptsbin/x2ui/ui_management.alb";

/// Run by `StartScreenWorld` after all contents, in this order.
pub const WORLD_ROOT_SCRIPTS: [&str; 12] = [
    "scriptsbin/x2ui/npc_common.alb",
    "scriptsbin/x2ui/character_ability.alb",
    "scriptsbin/x2ui/faction.alb",
    "scriptsbin/x2ui/interaction.alb",
    "scriptsbin/x2ui/npc_quest_interaction.alb",
    "scriptsbin/x2ui/store_backpack.alb",
    "scriptsbin/x2ui/transfer.alb",
    "scriptsbin/x2ui/dynamic_action_bar.alb",
    "scriptsbin/x2ui/ucc.alb",
    "scriptsbin/x2ui/demo.alb",
    "scriptsbin/x2ui/npc_interaction.alb",
    "scriptsbin/x2ui/renew_item.alb",
];

/// A client screen with its own UI script set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    /// In game (HUD and every other content).
    World,
    /// The loading screen (`scriptsbin/globalui/loading`).
    Loading,
}

impl Stage {
    pub const ALL: [Stage; 6] =
        [Stage::Login, Stage::WorldSelect, Stage::CharacterSelect, Stage::CharacterCreate, Stage::World, Stage::Loading];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Login => "login",
            Stage::WorldSelect => "world_select",
            Stage::CharacterSelect => "character_select",
            Stage::CharacterCreate => "character_create",
            Stage::World => "world",
            Stage::Loading => "loading",
        }
    }

    pub fn is_login_stage(self) -> bool {
        matches!(self, Stage::Login | Stage::WorldSelect | Stage::CharacterSelect | Stage::CharacterCreate)
    }

    /// The `toc.g` files of this screen in load order (paths in the pak). The world stage also
    /// runs single files around them, see [`Stage::units`].
    pub fn tocs(self) -> Vec<String> {
        let content = |c: &str| format!("scriptsbin/x2ui/{c}/toc.g");
        let mut tocs: Vec<String> = match self {
            Stage::Loading => return vec!["scriptsbin/globalui/loading/toc.g".into()],
            _ => BASE_CONTENTS.iter().map(|c| content(c)).collect(),
        };
        match self {
            Stage::Login => tocs.push(content("loginstage/login")),
            Stage::WorldSelect => tocs.push(content("loginstage/world_select")),
            Stage::CharacterSelect => {
                tocs.push(content("loginstage/character_select"));
                tocs.push(content("secondpassword"));
            }
            Stage::CharacterCreate => tocs.push(content("loginstage/character_create")),
            Stage::World => tocs.extend(world_contents().iter().map(|c| content(c))),
            Stage::Loading => unreachable!(),
        }
        tocs
    }

    /// The load units of this screen in order: `toc.g` paths and single `.alb` scripts.
    pub fn units(self) -> Vec<String> {
        let mut units = self.tocs();
        if self == Stage::World {
            let n = BASE_CONTENTS.len();
            units.insert(n, WORLD_FIRST_SCRIPT.to_string());
            units.extend(WORLD_ROOT_SCRIPTS.iter().map(|s| s.to_string()));
        }
        units
    }

    /// Every script this screen runs, in order (excluding [`INIT_SCRIPT`] and scripts pulled in
    /// at run time by `Script.ReloadScript`).
    pub fn scripts(self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for unit in self.units() {
            if unit.ends_with("toc.g") {
                out.extend(toc_files(&unit)?);
            } else {
                out.push(unit);
            }
        }
        Ok(out)
    }
}

/// Every content directory of `scriptsbin/x2ui` with a `toc.g` (except the base contents), in
/// alphabetical order: what `LoadAllContents` loads in game.
pub fn world_contents() -> Vec<String> {
    let mut dirs: Vec<String> = aa_pak::Pak::global()
        .list("scriptsbin/x2ui/")
        .filter_map(|n| {
            let n = aa_pak::normalize(n);
            let rest = n.strip_prefix("game/").unwrap_or(&n).strip_prefix("scriptsbin/x2ui/")?.to_string();
            let (dir, file) = rest.split_once('/')?;
            (file == "toc.g" && !BASE_CONTENTS.contains(&dir)).then(|| dir.to_string())
        })
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}
