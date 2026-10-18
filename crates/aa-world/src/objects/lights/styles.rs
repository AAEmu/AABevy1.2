//! Original Light.LightStyles tables and the ordinary DLight luminance consumer.
//! Renderer3822B7E0 interpolates RGBA;38179800 scales base RGB by its luminance.

use bevy::prelude::*;
use super::LocalLight;

/// Diagnostic controls; ordinary app frames follow Bevy's continuous virtual game seconds.
#[derive(Resource, Clone, Debug)]
pub struct LightStyleSettings {
    pub enabled: bool,
    /// Fixed native game-clock input for controlled captures; None follows the app clock.
    pub seconds: Option<f32>,
}
impl Default for LightStyleSettings {
    fn default() -> Self { Self { enabled: true, seconds: None } }
}

#[derive(Clone, Debug)]
struct Style {
    keys: Vec<Vec4>,
    speed: f32,
    phase: f32,
    gain: f32,
}

/// One original shader read at plugin initialization; at most256 byte-indexed styles.
/// Samples cache per style, not per entity. No per-frame source reads or allocations.
#[derive(Resource, Debug)]
pub struct LightStyles {
    entries: Vec<Option<Style>>,
}
impl Default for LightStyles {
    fn default() -> Self {
        let source = crate::pak_available().then(|| aa_pak::Pak::global()
            .read("game/shaders/hwscripts/cryfx/light.cfx").ok()).flatten();
        source.and_then(|s| String::from_utf8(s).ok()).and_then(|s| Self::parse(&s))
            .unwrap_or_else(|| Self { entries: Vec::new() })
    }
}

fn braced(text: &str) -> Option<(&str, &str)> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (i, c) in text[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 { return Some((&text[start + 1..start + i], &text[start + i + 1..])); }
            }
            _ => (),
        }
    }
    None
}

impl LightStyles {
    fn parse(source: &str) -> Option<Self> {
        if source.len() > 64 * 1024 { return None; }
        let (_, section) = source.split_once("technique LightStyles")?;
        let (mut body, _) = braced(section)?;
        let mut entries: Vec<Option<Style>> = (0..256).map(|_| None).collect();
        let mut total = 0usize;
        while let Some((_, rest)) = body.split_once("Style ") {
            let (index, _) = rest.split_once('{')?;
            let index: usize = index.trim().parse().ok()?;
            if index >= entries.len() || entries[index].is_some() { return None; }
            let (fields, tail) = braced(rest)?;
            body = tail;
            let speed = if let Some((_, speed)) = fields.split_once("Speed") {
                speed.split_once('=')?.1.split_once(';')?.0.trim().parse::<f32>().ok()?
            } else { 60.0 }; // Original constructor/ParseBinFX_LightStyle default at+44.
            if !speed.is_finite() || speed < 0.0 { return None; }
            let values = braced(fields.split_once("ValueString")?.1)?.0;
            let mut keys = Vec::new();
            for value in values.split('|').map(str::trim).filter(|v| !v.is_empty()) {
                let values: Vec<f32> = value.split_whitespace().map(str::parse).collect::<Result<_, _>>().ok()?;
                if !values.iter().all(|v| v.is_finite()) { return None; }
                let key = match values.as_slice() {
                    [v] => Vec4::new(*v, *v, *v, 1.0),
                    [r, g] => Vec4::new(*r, *g, 1.0, 1.0),
                    [r, g, b] => Vec4::new(*r, *g, *b, 1.0),
                    [r, g, b, a] => Vec4::new(*r, *g, *b, *a),
                    _ => return None,
                };
                keys.push(key);
                total += 1;
                if keys.len() > 1024 || total > 4096 { return None; }
            }
            if keys.is_empty() { return None; }
            // Original object starts with current white, intensity1 and cached phase0.
            entries[index] = Some(Style { keys, speed, phase: 0.0, gain: 1.0 });
        }
        Some(Self { entries })
    }

    pub fn count(&self) -> usize { self.entries.iter().flatten().count() }

    /// Native low-byte style identity, game-clock seconds and source float32 operation order.
    /// Style0 and absent tables retain base light RGB. Invalid diagnostic time is declined.
    pub fn gain(&mut self, style: u16, seconds: f32) -> f32 {
        let index = usize::from(style as u8);
        if index == 0 || !seconds.is_finite() || seconds < 0.0 { return 1.0; }
        let Some(style) = self.entries.get_mut(index).and_then(Option::as_mut) else { return 1.0; };
        let phase = seconds * style.speed;
        if !phase.is_finite() { return 1.0; }
        if phase != style.phase {
            style.phase = phase;
            // CVTTSS2SI returns INT_MIN outside its signed range; the native DIV uses
            // that bit pattern as unsigned. Ordinary game times remain well within it.
            let frame = if phase < 2147483648.0 { phase as i32 } else { i32::MIN };
            let a = (frame as u32) as usize % style.keys.len();
            let b = (frame.wrapping_add(1) as u32) as usize % style.keys.len();
            let fraction = phase - frame as f32;
            let from = style.keys[a];
            let to = style.keys[b];
            let value = if style.keys.len() == 1 { from } else { (to - from) * fraction + from };
            style.gain = (value.y * 0.58999997 + value.x * 0.3) + value.z * 0.11;
        }
        style.gain
    }
}

pub(super) fn animate(
    time: Res<Time>,
    settings: Res<LightStyleSettings>,
    mut styles: ResMut<LightStyles>,
    mut lights: Query<(&LocalLight, &mut PointLight)>,
) {
    let seconds = settings.seconds.unwrap_or(time.elapsed_secs());
    // Only selected PointLight components exist here: at most64. Authored dormant
    // slots and rejected nonzero-style projectors do not enter this per-frame walk.
    for (slot, mut light) in &mut lights {
        let gain = if settings.enabled { styles.gain(slot.placement.style, seconds) } else { 1.0 };
        // Native38179800 applies HDR after style/base multiplication. Decline a
        // nonfinite GPU colour even if diagnostic placement changes between selection ticks.
        let rgb = super::hdr::color(slot.placement.diffuse, gain, slot.placement.hdr_dynamic)
            .unwrap_or(Vec3::ZERO);
        let color = Color::linear_rgb(rgb.x, rgb.y, rgb.z);
        if light.color != color { light.color = color; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_tables_native_float_luminance_cache_wrap_and_reset() {
        if !crate::pak_available() { return; }
        let mut styles = LightStyles::default();
        assert_eq!(styles.count(), 37);
        // Original3822B7E0 and38179800 executed over all37 source tables:
        // 666 interpolation/cache controls and666 DLight upload controls, bit-exact.
        for (index, early, late) in [(2, 1.0_f32, 1.0_f32), (4, 0.33625, 0.58999997),
            (6, 0.125, 1.0), (12, 0.75, 0.5), (19, 0.775, 1.0)] {
            assert_eq!(styles.gain(index, 0.0), 1.0, "native initial zero-phase cache");
            assert_eq!(styles.gain(index, 0.125).to_bits(), early.to_bits());
            assert_eq!(styles.gain(index, 0.125).to_bits(), early.to_bits());
            assert_eq!(styles.gain(index, 1.0).to_bits(), late.to_bits());
        }
        assert_eq!(styles.gain(4, 0.0), 0.3, "reset after nonzero phase evaluates first key");
        assert_eq!(styles.gain(4, 0.0), 0.3);
        assert_eq!(styles.gain(4, 2.5), 0.3, "original25-key colour-table loop");
        assert_eq!(styles.gain(260, 2.5), 0.3, "native style field is a byte");
        assert_eq!(styles.gain(0, 1.0), 1.0);
        assert_eq!(styles.gain(255, 1.0), 1.0);
        assert_eq!(styles.gain(6, f32::NAN), 1.0);
        assert_eq!(styles.gain(6, -1.0), 1.0);
    }

    #[test]
    fn parser_preserves_native_value_expansion_default_speed_and_bounds() {
        let mut styles = LightStyles::parse("technique LightStyles { Style 1 { ValueString = {|0.2|}; } Style 2 { Speed=1; ValueString={|0.1 0.2|0.3 0.4 0.5 0.6|}; } }").unwrap();
        let s = styles.entries[1].as_ref().unwrap();
        assert_eq!(s.speed, 60.0);
        assert_eq!(s.keys, [Vec4::new(0.2, 0.2, 0.2, 1.0)]);
        assert_eq!(styles.entries[2].as_ref().unwrap().keys[0], Vec4::new(0.1, 0.2, 1.0, 1.0));
        assert_eq!(styles.gain(1, 0.0), 1.0);
        assert_eq!(styles.gain(1, 0.1), (0.2 * 0.58999997 + 0.2 * 0.3) + 0.2 * 0.11);
        assert!(LightStyles::parse("technique LightStyles { Style 256 { ValueString={|1|}; } }").is_none());
        assert!(LightStyles::parse("technique LightStyles { Style 1 { ValueString={|NaN|}; } }").is_none());
    }

    #[test]
    fn selected_point_styles_update_between_selection_ticks_pause_reset_and_unload() {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        if !crate::pak_available() { return; }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(crate::WorldConfig::new("main_world"))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(25)));
        app.add_plugins(super::super::LocalLightPlugin);
        let light = crate::entities::parse_lights(r#"<Mission><Objects><Entity EntityClass="Light"><Properties Radius="5"><Color clrDiffuse="0.7,0.4,0.2"/><Style nLightStyle="6"/></Properties></Entity></Objects></Mission>"#).remove(0);
        app.world_mut().spawn((crate::WorldFocus, GlobalTransform::default()));
        app.world_mut().commands().queue(move |world: &mut World| {
            super::super::spawn_cell(&mut world.commands(), (0, 0), &[light]);
        });
        app.update(); app.update();
        let snapshot = |app: &mut App| {
            let world = app.world_mut();
            let seconds = world.resource::<Time>().elapsed_secs();
            let actual = world.query::<&PointLight>().single(world).unwrap().color.to_linear().to_vec3();
            (seconds, actual)
        };
        let (t1, c1) = snapshot(&mut app);
        app.update();
        let (t2, c2) = snapshot(&mut app);
        assert!(t2 > t1 && t2 < 0.2 && c1 != c2, "animation must not run at the 5Hz selection rate");
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        app.update();
        assert_eq!(snapshot(&mut app), (t2, c2));
        *app.world_mut().resource_mut::<Time<Virtual>>() = Time::<Virtual>::default();
        app.update();
        let (reset, color) = snapshot(&mut app);
        assert_eq!(reset, 0.025);
        assert!(color.abs_diff_eq(Vec3::new(0.7, 0.4, 0.2) * 0.025, 1e-7));
        app.world_mut().resource_mut::<super::super::LocalLightSettings>().enabled = false;
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query::<&PointLight>().iter(world).count(), 0);
        assert_eq!(world.resource::<LightStyles>().count(), 37);
    }
}
