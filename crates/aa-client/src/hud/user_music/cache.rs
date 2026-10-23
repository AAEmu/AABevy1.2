//! Native world/note cache and request sequencing. Notes never constitute a play receipt.
use aa_net::{
    Event, MusicCommand,
    notice::{NoticeEvent, UserNote},
};
use mlua::{Lua, Table};
use std::{cell::RefCell, collections::BTreeSet, path::PathBuf, rc::Rc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Score {
    pub title: String,
    pub notes: String,
}

#[derive(Default)]
pub(super) struct Cache {
    world: Option<u8>,
    owner: u32,
    actor: u32,
    directory: Option<PathBuf>,
    pending: BTreeSet<u64>,
    requests: Vec<(u32, MusicCommand)>,
    // The native lookup has one scratch score, not an unbounded in-memory song library.
    last: Option<(u64, Score)>,
    pub notes: String,
}

pub(super) type Shared = Rc<RefCell<Cache>>;
struct BoundCache(Shared);

fn c_string(value: &str, limit: usize) -> String {
    let value = value.split('\0').next().unwrap_or_default();
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

impl Cache {
    pub(super) fn identity(&mut self, world: Option<u8>, owner: u32, actor: u32) {
        if self.world != world {
            self.last = None;
        }
        if (self.world, self.owner, self.actor) != (world, owner, actor) {
            self.pending.clear();
            self.requests.clear();
            self.notes.clear();
        }
        (self.world, self.owner, self.actor) = (world, owner, actor);
    }

    pub fn deactivate(&mut self) {
        self.identity(self.world, 0, 0);
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::ReadyToConnect { world_id, .. } => self.identity(Some(*world_id), 0, 0),
            Event::CharacterSelected(selected) => {
                self.identity(self.world, selected.character.id, 0)
            }
            Event::PlayerSpawn(spawn) => {
                self.identity(self.world, spawn.character().id, spawn.obj_id)
            }
            Event::LeftWorld { .. } => self.deactivate(),
            Event::LoginSucceeded(_) | Event::Disconnected { .. } | Event::Kicked { .. } => {
                self.identity(None, 0, 0)
            }
            Event::UnitsRemoved(ids) if ids.contains(&self.actor) => self.deactivate(),
            Event::Notice(NoticeEvent::UserNoteLoaded(Some(note))) if self.active() => {
                self.receive(note)
            }
            Event::MusicRequestRejected {
                character_id,
                command: MusicCommand::RequestNotes { note_id, .. },
                ..
            } if *character_id == self.owner => {
                self.pending.remove(note_id);
            }
            _ => {}
        }
    }

    pub(super) fn active(&self) -> bool {
        self.world.is_some() && self.owner != 0 && self.actor != 0
    }

    pub(super) fn context(&self) -> Option<(u32, u32)> {
        self.active().then_some((self.owner, self.actor))
    }

    pub(super) fn owns(&self, owner: u32, actor: u32) -> bool {
        self.active() && self.owner == owner && self.actor == actor
    }

    fn path(&self, id: u64) -> Option<PathBuf> {
        // Native Windows "%016lx" consumes the low u32; pending/request IDs stay u64.
        let id = id as u32;
        Some(
            self.directory
                .as_ref()?
                .join(format!("user_music_{:02x}_{id:016x}.json", self.world?)),
        )
    }

    fn load(&mut self, id: u64) -> Option<Score> {
        if let Some((key, score)) = &self.last
            && *key == id
        {
            return Some(score.clone());
        }
        let path = self.path(id)?;
        // Worst-case JSON escaping of the native 96/12000 byte strings fits in 73 KiB.
        if std::fs::metadata(&path).ok()?.len() > 73 * 1024 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let title = value.get("title")?.as_str()?;
        let notes = value.get("notes")?.as_str()?;
        if title.len() > 96 || notes.len() > 12000 || title.contains('\0') || notes.contains('\0') {
            return None;
        }
        let score = Score {
            title: title.into(),
            notes: notes.into(),
        };
        self.last = Some((id, score.clone()));
        Some(score)
    }

    pub fn lookup(&mut self, id: u64, tooltip: bool) -> Option<Score> {
        if !self.active() {
            return None;
        }
        if let Some(score) = self.load(id) {
            if !tooltip {
                self.requests.push((
                    self.owner,
                    MusicCommand::RequestNotes {
                        note_id: id,
                        is_tooltip: false,
                        show_buff_only: true,
                    },
                ));
                self.notes = score.notes.clone();
            }
            return Some(score);
        }
        // Original39082550 deduplicates by ID, including a tooltip/use overlap.
        if self.pending.insert(id) {
            self.requests.push((
                self.owner,
                MusicCommand::RequestNotes {
                    note_id: id,
                    is_tooltip: tooltip,
                    show_buff_only: false,
                },
            ));
        }
        None
    }

    fn receive(&mut self, note: &UserNote) {
        // Original391e1691 CDQ: high-bit SC1e0 IDs sign-extend into the u64 cache key.
        let id = i64::from(note.song_id as i32) as u64;
        let score = Score {
            title: c_string(&note.title, 96),
            notes: c_string(&note.notes, 12000),
        };
        let result = self
            .path(id)
            .ok_or_else(|| std::io::Error::other("music cache path unavailable"))
            .and_then(|path| {
                std::fs::create_dir_all(path.parent().unwrap())?;
                let temp = path.with_extension("tmp");
                let bytes = serde_json::to_vec(
                    &serde_json::json!({"title":score.title,"notes":score.notes}),
                )
                .map_err(std::io::Error::other)?;
                std::fs::write(&temp, bytes)?;
                std::fs::rename(temp, path)
            });
        if let Err(error) = result {
            bevy::log::warn!("user music: cache save: {error}");
            self.last = None;
        } else {
            self.last = Some((id, score.clone()));
        }
        // Original39082260 clears the pending ID and copies notes even if storage fails.
        self.pending.remove(&id);
        self.notes = score.notes;
    }

    pub fn drain(&mut self) -> Vec<(u32, MusicCommand)> {
        std::mem::take(&mut self.requests)
    }
}

pub(super) fn bind(lua: &Lua, cache: &Shared) {
    sync_directory(lua, cache);
    lua.set_app_data(BoundCache(cache.clone()));
}

pub(super) fn sync_directory(lua: &Lua, cache: &Shared) {
    let directory = crate::settings::music_directory(lua);
    let mut state = cache.borrow_mut();
    if state.directory.as_ref() != Some(&directory) {
        state.last = None;
    }
    state.directory = Some(directory);
}

pub(in crate::hud) fn tooltip(
    lua: &Lua,
    game: &super::super::data::GameData,
    table: &Table,
    item_type: u32,
    extra: &super::super::tips::ItemExtra,
) -> mlua::Result<()> {
    if item_type != 28051
        || !game
            .items
            .get(&item_type)
            .is_some_and(|item| item.impl_id == 26)
    {
        return Ok(());
    }
    let Some(id) = extra.music_note else {
        return Ok(());
    };
    let Some(cache) = lua
        .app_data_ref::<BoundCache>()
        .map(|bound| bound.0.clone())
    else {
        return Ok(());
    };
    sync_directory(lua, &cache);
    let score = cache.borrow_mut().lookup(id, true);
    if let Some(score) = score {
        table.raw_set("userMusicTitle", score.title)?;
        // Native39081f80 returns zero for both empty text and no matching limit.
        table.raw_set(
            "userMusicRequireGrade",
            game.user_music.grade(&score.notes).unwrap_or(0),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[allow(dead_code)]
pub(in crate::hud) mod fixture;
