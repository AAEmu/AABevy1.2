//! The `AAUI` sound hooks (see `aa-ui`'s `lua/sound.rs`) and the sound options, bound into every UI Lua state.
//!
//! * `AAUI.PlaySound(name, duplicable)`: a pack item of the `global` pack (`login_stage_try_login`), played 2D on the UI
//!   volume; returns the voice id (0 when the name has no sound), which `AAUI.StopSound(id, mode)` stops.
//! * `AAUI.IsSoundPlaying(id)`: whether that voice has been asked for and has not ended.
//! * `AAUI.PlayMusic(name)` / `AAUI.StopMusic()`: handed to the music director ([`super::music`]), which crossfades.
//! * `AAUI.WidgetSound(set, event)`: `<set>_<event>` in the `window` (`bag_show`), `button` (`default_left_up`) or
//!   `global` (`edit_box_text_added`) pack. Not played while a screen's UI is being built (the gate).
//! * `AAUI.OptionValue(id)` / `AAUI.OptionChanged(id, value)`: the sound sliders of the Options window
//!   (`OPTION_ITEM_MASTER_VOLUME` 35, `BGM` 36, `SFX` 37, `USE_COMBAT_SOUND` 38, `CINEMA` 39,
//!   `USER_MUSIC_VOLUME` 40 and `USER_MUSIC_DISABLE_OTHERS` 41).
//!   master -> `Volumes::master`, BGM -> music, SFX -> effects, ambience, voices and UI.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use aa_audio::{Category, PlayOptions, Voice, Volumes};
use bevy::prelude::*;
use mlua::{Lua, Table};

use super::{BINDING, Binding, Shared, log_enabled};
use crate::Screen;

const MASTER: i64 = 35;
const BGM: i64 = 36;
const SFX: i64 = 37;
const USE_COMBAT_SOUND: i64 = 38;
const CINEMA: i64 = 39;
const USER_MUSIC: i64 = 40;
const MUTE_OTHER_MUSIC: i64 = 41;

/// Binds the sound hooks into a UI Lua state (called by the game API install of every stage).
pub fn bind(lua: &Lua) {
    let Some(binding) = BINDING.get() else { return };
    if let Err(e) = install(lua, binding) {
        warn!("sound: cannot bind the AAUI sound hooks: {e}");
    }
}

fn install(lua: &Lua, b: &Binding) -> mlua::Result<()> {
    let globals = lua.globals();
    let aaui = match globals.raw_get::<Option<Table>>("AAUI")? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            globals.raw_set("AAUI", &t)?;
            t
        }
    };

    let (engine, packs) = (b.engine.clone(), b.packs.clone());
    aaui.raw_set(
        "PlaySound",
        lua.create_function(move |_, (name, _duplicable): (String, Option<bool>)| {
            let Some(path) = packs.get().and_then(|p| p.ui(&name)) else {
                debug!("sound: UI sound {name:?} has no sound");
                return Ok(0i64);
            };
            if log_enabled() {
                info!("audio: ui sound {name} -> {path}");
            }
            let voice = engine.play_with(path, None, PlayOptions { category: Some(Category::Ui), ..default() });
            Ok(voice.0 as i64)
        })?,
    )?;

    let engine = b.engine.clone();
    aaui.raw_set(
        "StopSound",
        lua.create_function(move |_, (id, mode): (i64, Option<i64>)| {
            if id > 0 {
                // a stop mode is "at once", else the sound fades out
                engine.stop(Voice(id as u64), mode.is_none());
            }
            Ok(())
        })?,
    )?;

    let engine = b.engine.clone();
    aaui.raw_set(
        "IsSoundPlaying",
        lua.create_function(move |_, id: i64| Ok(id > 0 && engine.is_playing(Voice(id as u64))))?,
    )?;

    let shared = b.shared.clone();
    aaui.raw_set(
        "PlayMusic",
        lua.create_function(move |_, name: String| {
            *shared.music.lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(name));
            Ok(())
        })?,
    )?;
    let shared = b.shared.clone();
    aaui.raw_set(
        "StopMusic",
        lua.create_function(move |_, ()| {
            *shared.music.lock().unwrap_or_else(|e| e.into_inner()) = Some(None);
            Ok(())
        })?,
    )?;

    let (engine, packs, shared) = (b.engine.clone(), b.packs.clone(), b.shared.clone());
    aaui.raw_set(
        "WidgetSound",
        lua.create_function(move |_, (set, event): (String, String)| {
            if !shared.widget_sounds.load(Ordering::Relaxed) {
                return Ok(());
            }
            let Some(packs) = packs.get() else { return Ok(()) };
            let item = format!("{set}_{event}");
            let found = ["window", "button", "global"].into_iter().find_map(|pack| packs.path(pack, &item).map(|p| (pack, p)));
            if let Some((pack, path)) = found {
                if log_enabled() {
                    info!("audio: widget sound {pack}/{item} -> {path}");
                }
                engine.play_with(path, None, PlayOptions { category: Some(Category::Ui), ..default() });
            }
            Ok(())
        })?,
    )?;

    let shared = b.shared.clone();
    aaui.raw_set(
        "OptionValue",
        lua.create_function(move |_, id: i64| {
            let o = shared.options.lock().unwrap_or_else(|e| e.into_inner());
            Ok(match id {
                MASTER => Some(f64::from(o.master)),
                BGM => Some(f64::from(o.bgm)),
                SFX => Some(f64::from(o.sfx)),
                CINEMA => Some(f64::from(o.cinema)),
                USER_MUSIC => Some(f64::from(o.user_music)),
                MUTE_OTHER_MUSIC => Some(if o.mute_other_music { 1.0 } else { 0.0 }),
                USE_COMBAT_SOUND => Some(if o.combat_music { 1.0 } else { 0.0 }),
                _ => None,
            })
        })?,
    )?;
    let shared = b.shared.clone();
    aaui.raw_set(
        "OptionChanged",
        lua.create_function(move |_, (id, value): (i64, f64)| {
            let mut o = shared.options.lock().unwrap_or_else(|e| e.into_inner());
            let v = (value as f32).clamp(0.0, 1.0);
            let before = *o;
            match id {
                MASTER => o.master = v,
                BGM => o.bgm = v,
                SFX => o.sfx = v,
                CINEMA => o.cinema = v,
                USER_MUSIC if value.is_finite() => o.user_music = v,
                MUTE_OTHER_MUSIC if value.is_finite() => o.mute_other_music = (value as f32) as i32 == 1,
                USE_COMBAT_SOUND => o.combat_music = value != 0.0,
                _ => {}
            }
            o.changed |= *o != before;
            Ok(())
        })?,
    )?;
    Ok(())
}

/// The shared state, for the systems.
#[derive(Resource)]
pub(super) struct Gate(pub Arc<Shared>);

/// Moves the option sliders into the volumes of the audio engine.
pub(super) fn sync_options(gate: Res<Gate>, mut volumes: ResMut<Volumes>) {
    let mut o = gate.0.options.lock().unwrap_or_else(|e| e.into_inner());
    if o.changed {
        o.changed = false;
        let v = o.volumes();
        if *volumes != v {
            if log_enabled() {
                info!("audio: volumes master {:.1} music {:.1} effects {:.1}", v.master, v.music, v.sfx);
            }
            *volumes = v;
        }
    }
}

/// Widget sounds are allowed a moment after a screen came up (its scripts show and hide windows while they build), and not
/// on the loading screen (the in-game UI is built behind it).
pub(super) fn sync_gate(gate: Res<Gate>, screen: Res<State<Screen>>, time: Res<Time>, mut since: Local<(Option<Screen>, f32)>) {
    let now = *screen.get();
    if since.0 != Some(now) {
        *since = (Some(now), 0.0);
    }
    since.1 += time.delta_secs();
    gate.0.widget_sounds.store(now != Screen::Loading && since.1 > 1.0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_music_slider_is_independent_and_other_mute_uses_native_integer_one()
    -> mlua::Result<()> {
        let shared = Arc::new(Shared::default());
        let binding = Binding {
            engine: aa_audio::Engine::start(aa_audio::Settings { audible: false, ..default() }),
            packs: super::super::Packs(Arc::new(std::sync::OnceLock::new())),
            shared: shared.clone(),
        };
        let lua = Lua::new();
        install(&lua, &binding)?;
        lua.load(r#"
            assert(AAUI.OptionValue(40)==1 and AAUI.OptionValue(41)==0)
            AAUI.OptionChanged(35,.4); AAUI.OptionChanged(36,0); AAUI.OptionChanged(37,0)
            AAUI.OptionChanged(40,.3)
            assert(math.abs(AAUI.OptionValue(40)-.3)<.00001)
            AAUI.OptionChanged(41,2); assert(AAUI.OptionValue(41)==0)
            AAUI.OptionChanged(41,1); assert(AAUI.OptionValue(41)==1)
            AAUI.OptionChanged(40,0/0); AAUI.OptionChanged(41,0/0)
        "#).exec()?;
        let options = *shared.options.lock().unwrap();
        assert!(options.changed && options.mute_other_music);
        let volumes = options.volumes();
        assert_eq!(volumes.music, 0.0);
        assert_eq!(volumes.sfx, 0.0);
        assert!((volumes.get(Category::UserMusic) * volumes.master - 0.12).abs() < 0.00001);
        lua.load(r#"
            X2Option={};NET={option_get=function(id)
                if id==40 then return .25 elseif id==41 then return 1 end
            end,option_defaults=function() return {[40]=1,[41]=0} end,
            option_set=function() end}
        "#).exec()?;
        lua.load(include_str!("../settings_options.lua")).exec()?;
        assert_eq!(shared.options.lock().unwrap().user_music, 0.25);
        assert!(shared.options.lock().unwrap().mute_other_music);
        lua.load("X2Option:Reset()").exec()?;
        assert_eq!(shared.options.lock().unwrap().user_music, 1.0);
        assert!(!shared.options.lock().unwrap().mute_other_music);
        Ok(())
    }
}
