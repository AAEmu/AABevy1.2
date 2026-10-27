//! Native repair OS cursor, independent of the item or image held by `X2Cursor`.
//!
//! The app owns repair mode. Its Lua bindings call [`set_repair_mode`]; a large cursor
//! can be requested explicitly with [`set_large`]. Images are read from the original
//! executable's resources, never substituted with scaled artwork.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use bevy::{
    asset::RenderAssetUsages,
    prelude::{
        Assets, Commands, Entity, Image, Local, NonSend, Query, Res, ResMut, Resource, With,
        default, warn,
    },
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow},
};

#[derive(Clone, Copy, Default)]
struct Request {
    repair: bool,
    large: bool,
    resize: Option<ResizeCursor>,
}

/// Native cursor enums 8..11; the large theme has separate, still 32-pixel artwork.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResizeCursor {
    DiagonalDown,
    DiagonalUp,
    Horizontal,
    Vertical,
}

pub(crate) fn set_resize(lua: &mlua::Lua, resize: Option<ResizeCursor>) {
    if let Some(mut request) = lua.app_data_mut::<Request>() {
        request.resize = resize;
    }
}

pub(crate) fn init(lua: &mlua::Lua) {
    lua.set_app_data(Request::default());
}

/// Select the native hammer cursor without changing `X2Cursor:GetHeld()` or its generation.
pub fn set_repair_mode(lua: &mlua::Lua, enabled: bool) {
    if let Some(mut request) = lua.app_data_mut::<Request>() {
        request.repair = enabled;
    }
}

/// Choose the separate native 64-pixel resource; false selects the 32-pixel resource.
/// This does not infer a theme from UI scale, display DPI or window resolution.
pub fn set_large(lua: &mlua::Lua, large: bool) {
    if let Some(mut request) = lua.app_data_mut::<Request>() {
        request.large = large;
    }
}

/// Read-only source of the original cursor resources. Override before entering repair mode.
/// A missing or unsupported executable leaves the existing OS cursor in place.
#[derive(Resource, Clone, Debug)]
pub struct CursorSource(pub PathBuf);

impl Default for CursorSource {
    /// `AA_EXE` if set; else `bin32/archeage.exe` next to the `AA_PAK` (or default) `game_pak`
    /// when that file exists; else the historical reverse-engineering copy.
    fn default() -> Self {
        Self(source_path(
            std::env::var_os("AA_EXE"),
            std::env::var_os("AA_PAK"),
        ))
    }
}

/// Where the exe is looked for when neither `AA_EXE` nor the pak's client folder has it (relative to the working
/// directory).
const FALLBACK_EXE: &str = "bin32/archeage.exe";

fn source_path(exe: Option<std::ffi::OsString>, pak: Option<std::ffi::OsString>) -> PathBuf {
    if let Some(exe) = exe.filter(|e| !e.is_empty()) {
        return PathBuf::from(exe);
    }
    let pak = pak
        .filter(|p| !p.is_empty())
        .map_or_else(|| PathBuf::from(aa_pak::DEFAULT_PAK), PathBuf::from);
    if let Some(dir) = pak.parent() {
        let candidate = dir.join("bin32").join("archeage.exe");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(FALLBACK_EXE)
}

#[derive(Default)]
pub(crate) struct CursorCache {
    source: Option<PathBuf>,
    icons: HashMap<Option<ResizeCursor>, Option<[CursorIcon; 2]>>,
    previous: Option<(Entity, Option<CursorIcon>)>,
    applied: Option<CursorIcon>,
}

pub(crate) fn sync(
    rt: Option<NonSend<crate::UiRuntime>>,
    source: Res<CursorSource>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<(Entity, Option<&CursorIcon>), With<PrimaryWindow>>,
    mut commands: Commands,
    mut cache: Local<CursorCache>,
) {
    // The browser has no archeage.exe on disk: it keeps the page's own cursor.
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let request = rt
        .as_ref()
        .and_then(|rt| rt.lua.app_data_ref::<Request>().map(|r| *r))
        .unwrap_or_default();
    let Ok((window, current)) = windows.single() else {
        return;
    };
    if !request.repair && request.resize.is_none() {
        if let Some((old_window, previous)) = cache.previous.take()
            && old_window == window
            && current == cache.applied.as_ref()
        {
            if let Some(previous) = previous {
                commands.entity(window).insert(previous);
            } else {
                commands.entity(window).remove::<CursorIcon>();
            }
        }
        cache.applied = None;
        return;
    }
    if cache.source.as_ref() != Some(&source.0) {
        cache.source = Some(source.0.clone());
        cache.icons.clear();
    }
    if !cache.icons.contains_key(&request.resize) {
        match load_pair(&source.0, request.resize) {
            Ok(pair) => {
                let icons = pair.map(|(image, hotspot)| {
                    CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
                        handle: images.add(image),
                        hotspot,
                        ..default()
                    }))
                });
                cache.icons.insert(request.resize, Some(icons));
            }
            Err(error) => {
                warn!("aa-ui: native cursor {}: {error:#}", source.0.display());
                cache.icons.insert(request.resize, None);
            }
        }
    }
    let Some(icons) = cache.icons.get(&request.resize).and_then(Option::as_ref) else {
        if let Some((old_window, previous)) = cache.previous.take()
            && old_window == window
            && current == cache.applied.as_ref()
        {
            if let Some(previous) = previous {
                commands.entity(window).insert(previous);
            } else {
                commands.entity(window).remove::<CursorIcon>();
            }
        }
        cache.applied = None;
        return;
    };
    let icon = icons[usize::from(request.large)].clone();
    if cache.previous.as_ref().map(|p| p.0) != Some(window) {
        cache.previous = Some((window, current.cloned()));
    }
    if current != Some(&icon) {
        commands.entity(window).insert(icon.clone());
    }
    cache.applied = Some(icon);
}

/// Decode the source-proved group 141 (normal) or 142 (large), including its AND mask.
/// Exposed for native-resource previews; the window hook caches both images on first use.
pub fn load_repair_image(executable: &Path, large: bool) -> Result<(Image, (u16, u16))> {
    let bytes = std::fs::read(executable).with_context(|| executable.display().to_string())?;
    repair_image(&bytes, large)
}

fn load_pair(executable: &Path, resize: Option<ResizeCursor>) -> Result<[(Image, (u16, u16)); 2]> {
    let bytes = std::fs::read(executable).with_context(|| executable.display().to_string())?;
    if let Some(resize) = resize {
        Ok([
            resize_image(&bytes, resize, false)?,
            resize_image(&bytes, resize, true)?,
        ])
    } else {
        Ok([repair_image(&bytes, false)?, repair_image(&bytes, true)?])
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .context("truncated resource")?
            .try_into()?,
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated resource")?
            .try_into()?,
    ))
}

/// Only the original PE32 resource layout is needed, with all offsets bounds checked.
fn resource(bytes: &[u8], kind: u32, id: u32) -> Result<&[u8]> {
    ensure!(bytes.get(..2) == Some(b"MZ"), "not a PE executable");
    let pe = u32_at(bytes, 0x3c)? as usize;
    ensure!(
        bytes.get(pe..pe + 4) == Some(b"PE\0\0"),
        "missing PE signature"
    );
    let optional = pe + 24;
    ensure!(
        u16_at(bytes, optional)? == 0x10b,
        "expected original PE32 executable"
    );
    let sections = optional + u16_at(bytes, pe + 20)? as usize;
    let file_offset = |rva: u32| -> Result<usize> {
        for index in 0..u16_at(bytes, pe + 6)? as usize {
            let section = sections + index * 40;
            let start = u32_at(bytes, section + 12)?;
            let length = u32_at(bytes, section + 16)?;
            if let Some(relative) = rva.checked_sub(start).filter(|r| *r < length) {
                return Ok(u32_at(bytes, section + 20)? as usize + relative as usize);
            }
        }
        anyhow::bail!("resource RVA outside file sections")
    };
    let root = file_offset(u32_at(bytes, optional + 112)?)?;
    let entry = |directory: usize, wanted: Option<u32>| -> Result<u32> {
        let count =
            u16_at(bytes, directory + 12)? as usize + u16_at(bytes, directory + 14)? as usize;
        for index in 0..count {
            let at = directory + 16 + index * 8;
            if wanted.is_none_or(|id| u32_at(bytes, at).ok() == Some(id)) {
                return u32_at(bytes, at + 4);
            }
        }
        anyhow::bail!("missing cursor resource")
    };
    let directory = |offset: u32| -> Result<usize> {
        ensure!(offset & 0x8000_0000 != 0, "expected resource directory");
        Ok(root + (offset & 0x7fff_ffff) as usize)
    };
    let by_kind = directory(entry(root, Some(kind))?)?;
    let by_id = directory(entry(by_kind, Some(id))?)?;
    let data = entry(by_id, None)?;
    ensure!(data & 0x8000_0000 == 0, "expected resource data");
    let at = root + data as usize;
    let start = file_offset(u32_at(bytes, at)?)?;
    let length = u32_at(bytes, at + 4)? as usize;
    bytes
        .get(start..start + length)
        .context("truncated cursor resource")
}

fn repair_image(bytes: &[u8], large: bool) -> Result<(Image, (u16, u16))> {
    native_image(
        bytes,
        if large { 142 } else { 141 },
        if large { 64 } else { 32 },
        if large { 24 } else { 32 },
        (0, 0),
    )
}

fn resize_image(bytes: &[u8], kind: ResizeCursor, large: bool) -> Result<(Image, (u16, u16))> {
    let group =
        161 + 2 * match kind {
            ResizeCursor::DiagonalDown => 0,
            ResizeCursor::DiagonalUp => 1,
            ResizeCursor::Horizontal => 2,
            ResizeCursor::Vertical => 3,
        } + u32::from(large);
    native_image(bytes, group, 32, 32, (16, 16))
}

fn native_image(
    bytes: &[u8],
    group: u32,
    expected: usize,
    expected_bits: usize,
    expected_hotspot: (u16, u16),
) -> Result<(Image, (u16, u16))> {
    let group = resource(bytes, 12, group)?;
    ensure!(
        u16_at(group, 0)? == 0 && u16_at(group, 2)? == 2 && u16_at(group, 4)? == 1,
        "unsupported cursor group"
    );
    let cursor = resource(bytes, 1, u16_at(group, 18)? as u32)?;
    let hotspot = (u16_at(cursor, 0)?, u16_at(cursor, 2)?);
    let dib = cursor.get(4..).context("missing cursor DIB")?;
    let width = u32_at(dib, 4)? as usize;
    let height = u32_at(dib, 8)? as usize / 2;
    let bits = u16_at(dib, 14)? as usize;
    ensure!(
        u32_at(dib, 0)? == 40
            && width == expected
            && height == expected
            && u16_at(dib, 12)? == 1
            && bits == expected_bits
            && u32_at(dib, 16)? == 0
            && hotspot == expected_hotspot,
        "unsupported native cursor"
    );
    let stride = (width * bits).div_ceil(32) * 4;
    let mask_stride = width.div_ceil(32) * 4;
    let pixels = dib
        .get(40..40 + stride * height)
        .context("truncated cursor pixels")?;
    let mask = dib
        .get(40 + stride * height..40 + (stride + mask_stride) * height)
        .context("truncated cursor mask")?;
    let alpha = bits == 32 && pixels.chunks_exact(4).any(|p| p[3] != 0);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in (0..height).rev() {
        for x in 0..width {
            let p = &pixels[y * stride + x * bits / 8..];
            let transparent = mask[y * mask_stride + x / 8] & (128 >> (x % 8)) != 0;
            ensure!(
                alpha || !transparent || p[..3] == [0, 0, 0],
                "inverting cursor pixels are unsupported"
            );
            rgba.extend_from_slice(&[
                p[2],
                p[1],
                p[0],
                if alpha {
                    p[3]
                } else if transparent {
                    0
                } else {
                    255
                },
            ]);
        }
    }
    Ok((
        Image::new(
            Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ),
        hotspot,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        app::{App, Update},
        window::{SystemCursorIcon, Window},
    };

    #[test]
    fn explicit_exe_wins_and_unresolvable_pak_falls_back() {
        assert_eq!(
            source_path(Some("x/a.exe".into()), Some("y/game_pak".into())),
            PathBuf::from("x/a.exe")
        );
        assert_eq!(
            source_path(None, Some("no-such-dir/game_pak".into())),
            PathBuf::from(FALLBACK_EXE)
        );
    }

    #[test]
    fn original_resources_keep_distinct_geometry_alpha_and_mask() {
        for (large, width, checksum) in [
            (false, 32, 0x6b36bbbedb4ba282),
            (true, 64, 0xc4cafb5645fc4ac7),
        ] {
            let (image, hotspot) = load_repair_image(&CursorSource::default().0, large).unwrap();
            assert_eq!(
                (image.width(), image.height(), hotspot),
                (width, width, (0, 0))
            );
            let bytes = image.data.unwrap();
            let hash = bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
            assert_eq!(hash, checksum);
            assert_eq!(bytes.chunks_exact(4).any(|p| p[3] == 0), !large);
            assert!(bytes.chunks_exact(4).any(|p| p[3] == 255));
            assert_eq!(
                bytes.chunks_exact(4).any(|p| p[3] != 0 && p[3] != 255),
                !large
            );
        }
        assert!(repair_image(b"MZ", false).is_err());
        let bytes = std::fs::read(CursorSource::default().0).unwrap();
        let large = resource(&bytes, 1, 20).unwrap();
        let end = large.as_ptr() as usize - bytes.as_ptr() as usize + large.len();
        assert!(repair_image(&bytes[..end - 1], true).is_err());
    }

    #[test]
    fn original_resize_resources_keep_authored_size_and_center_hotspot() {
        let bytes = std::fs::read(CursorSource::default().0).unwrap();
        for kind in [
            ResizeCursor::DiagonalDown,
            ResizeCursor::DiagonalUp,
            ResizeCursor::Horizontal,
            ResizeCursor::Vertical,
        ] {
            for large in [false, true] {
                let (image, hotspot) = resize_image(&bytes, kind, large).unwrap();
                assert_eq!((image.width(), image.height(), hotspot), (32, 32, (16, 16)));
                let pixels = image.data.unwrap();
                assert!(pixels.chunks_exact(4).any(|p| p[3] == 0));
                assert!(pixels.chunks_exact(4).any(|p| p[3] != 0));
            }
        }
    }

    fn app(icon: Option<CursorIcon>) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<CursorSource>()
            .init_resource::<Assets<Image>>()
            .insert_non_send(crate::UiRuntime::new("en_us", 1920., 1080.).unwrap())
            .add_systems(Update, sync);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        if let Some(icon) = icon {
            app.world_mut().entity_mut(window).insert(icon);
        }
        (app, window)
    }

    fn request(app: &App, repair: bool, large: bool) {
        let rt = app.world().get_non_send::<crate::UiRuntime>().unwrap();
        set_repair_mode(&rt.lua, repair);
        set_large(&rt.lua, large);
    }

    fn custom(app: &App, window: Entity) -> &CustomCursorImage {
        match app.world().get::<CursorIcon>(window).unwrap() {
            CursorIcon::Custom(CustomCursor::Image(image)) => image,
            _ => panic!("expected native image cursor"),
        }
    }

    #[test]
    fn repair_transitions_restore_os_cursor_without_touching_held_item() {
        let original = CursorIcon::System(SystemCursorIcon::Crosshair);
        let (mut app, window) = app(Some(original.clone()));
        let rt = app.world().get_non_send::<crate::UiRuntime>().unwrap();
        rt.exec(
            "@repair_cursor",
            "X2Cursor:SetHeld({kind='bag',slot=7,count=2, cursorInfo='Item'})",
        )
        .unwrap();
        let held_gen = crate::lua::ui(&rt.lua).drag.held_gen;
        app.update();
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 0);
        {
            let rt = app.world().get_non_send::<crate::UiRuntime>().unwrap();
            let _native_borrow = crate::lua::ui(&rt.lua);
            set_repair_mode(&rt.lua, true);
            set_large(&rt.lua, false);
        }
        app.update();
        let normal = custom(&app, window).clone();
        assert_eq!(normal.hotspot, (0, 0));
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&normal.handle)
                .unwrap()
                .width(),
            32
        );
        app.update();
        assert_eq!(custom(&app, window).handle, normal.handle);
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 2);
        request(&app, true, true);
        app.update();
        let large = custom(&app, window);
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&large.handle)
                .unwrap()
                .width(),
            64
        );
        request(&app, false, true);
        app.update();
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&original));
        let rt = app.world().get_non_send::<crate::UiRuntime>().unwrap();
        assert_eq!(crate::lua::ui(&rt.lua).drag.held_gen, held_gen);
        rt.exec(
            "@repair_cursor",
            "assert(X2Cursor:GetHeld().slot==7 and X2Cursor:GetCursorInfo()=='Item')",
        )
        .unwrap();
        request(&app, true, false);
        app.update();
        app.world_mut().remove_non_send::<crate::UiRuntime>();
        app.update();
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&original));
    }

    #[test]
    fn resize_override_restores_repair_and_then_the_prior_os_cursor() {
        let original = CursorIcon::System(SystemCursorIcon::Crosshair);
        let (mut app, window) = app(Some(original.clone()));
        request(&app, true, false);
        app.update();
        let repair = custom(&app, window).clone();
        {
            let rt = app.world().get_non_send::<crate::UiRuntime>().unwrap();
            rt.exec(
                "@resize_cursor",
                "X2Cursor:SetHeld({kind='bag',slot=4,count=2})",
            )
            .unwrap();
            set_resize(&rt.lua, Some(ResizeCursor::Horizontal));
        }
        app.update();
        let resize = custom(&app, window).clone();
        assert_eq!(resize.hotspot, (16, 16));
        assert_ne!(resize.handle, repair.handle);
        app.update();
        assert_eq!(custom(&app, window).handle, resize.handle);
        {
            let rt = app.world().get_non_send::<crate::UiRuntime>().unwrap();
            rt.exec("@resize_cursor", "assert(X2Cursor:GetHeld().slot==4)")
                .unwrap();
            set_resize(&rt.lua, None);
        }
        app.update();
        assert_eq!(custom(&app, window).handle, repair.handle);
        request(&app, false, false);
        app.update();
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&original));
    }

    #[test]
    fn absent_source_and_removed_runtime_leave_cursor_ownership_intact() {
        let (mut app, window) = app(None);
        app.world_mut().resource_mut::<CursorSource>().0 =
            PathBuf::from("missing-native-cursor.exe");
        request(&app, true, false);
        app.update();
        assert!(app.world().get::<CursorIcon>(window).is_none());
        app.world_mut().insert_resource(CursorSource::default());
        app.update();
        assert!(matches!(
            app.world().get::<CursorIcon>(window),
            Some(CursorIcon::Custom(_))
        ));
        app.world_mut().resource_mut::<CursorSource>().0 =
            PathBuf::from("missing-native-cursor.exe");
        app.update();
        assert!(app.world().get::<CursorIcon>(window).is_none());
        app.world_mut().insert_resource(CursorSource::default());
        app.update();
        let external = CursorIcon::System(SystemCursorIcon::Text);
        app.world_mut().entity_mut(window).insert(external.clone());
        request(&app, false, false);
        app.update();
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&external));
    }
}
