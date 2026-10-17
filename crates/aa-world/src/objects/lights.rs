//! Authored local lights, with cell ownership and a bounded nearby render set.
//!
//! Bevy point lights use inverse-square falloff. Native Cry lights use (1-d²/r²)²; the
//! conversion below matches diffuse illuminance at half the authored radius. It does not
//! invent light sources, scale radii with entity Scale, or convert projectors into omni lights.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::coords::{cell_origin, cry_to_bevy};
use crate::entities::PlacedLight;
use crate::stream::{WorldConfig, WorldFocus};

#[derive(Resource, Clone, Debug)]
pub struct LocalLightSettings {
    pub enabled: bool,
    /// Distance to the light's influence sphere, in metres.
    pub distance: f32,
    pub max_lights: usize,
    /// Point-light shadows require six views each; only the nearest authored requests get them.
    pub max_shadow_lights: usize,
}

impl Default for LocalLightSettings {
    fn default() -> Self {
        Self {
            enabled: std::env::var_os("AA_NO_LOCAL_LIGHTS").is_none(),
            distance: 400.0,
            max_lights: 64,
            max_shadow_lights: 2,
        }
    }
}

/// An authored slot, retained even when inactive or outside the render budget.
#[derive(Component, Clone, Debug)]
pub struct LocalLight {
    pub cell: (i32, i32),
    pub placement: PlacedLight,
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct LocalLightStats {
    pub authored: usize,
    pub rendered: usize,
    pub shadowed: usize,
    pub unsupported: usize,
}

pub(crate) fn build(app: &mut App) {
    app.init_resource::<LocalLightSettings>()
        .init_resource::<LocalLightStats>()
        .add_systems(Update, update_lights.after(super::stream_objects));
}

pub(crate) fn spawn_cell(commands: &mut Commands, cell: (i32, i32), placed: &[PlacedLight]) -> Vec<Entity> {
    let origin = cell_origin(cell.0, cell.1);
    placed
        .iter()
        .map(|p| {
            commands
                .spawn((
                    Name::new(p.name.clone()),
                    LocalLight {
                        cell,
                        placement: p.clone(),
                    },
                    Transform::from_translation(cry_to_bevy(origin + p.matrix.w_axis.truncate())),
                    Visibility::default(),
                ))
                .id()
        })
        .collect()
}

/// Bevy: (1-q²)²/d², q=d²/r²; native: (1-q)². At d=r/2 the
/// required luminous intensity is 0.16*r² times the engine's light-unit illuminance.
fn lumens(radius: f32, lux_per_unit: f32) -> f32 {
    std::f32::consts::TAU * 2.0 * 0.16 * radius * radius * lux_per_unit
}

#[allow(clippy::too_many_arguments)]
fn update_lights(
    mut commands: Commands,
    time: Res<Time>,
    cfg: Res<WorldConfig>,
    settings: Res<LocalLightSettings>,
    mut stats: ResMut<LocalLightStats>,
    focus: Query<&GlobalTransform, With<WorldFocus>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    slots: Query<(Entity, &LocalLight, &Transform, Option<&PointLight>)>,
    mut next: Local<f32>,
) {
    // The source slots are static. Re-select at 5 Hz; there is no per-frame XML/model work.
    if time.elapsed_secs() < *next && !settings.is_changed() && !cfg.is_changed() {
        return;
    }
    *next = time.elapsed_secs() + 0.2;
    let camera = focus
        .iter()
        .next()
        .or_else(|| cameras.iter().next())
        .map(GlobalTransform::translation);
    let mut candidates = Vec::new();
    let mut result = LocalLightStats::default();
    for (entity, slot, transform, _) in &slots {
        result.authored += 1;
        let p = &slot.placement;
        if !p.supported() {
            result.unsupported += 1;
            continue;
        }
        if !settings.enabled || !cfg.objects || !p.active || p.hidden || p.diffuse.max_element() <= 0.0 {
            continue;
        }
        let Some(camera) = camera else { continue };
        let distance = camera.distance(transform.translation);
        if distance - p.radius <= settings.distance {
            candidates.push((distance, p.guid, entity));
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    candidates.truncate(settings.max_lights);
    let chosen: HashSet<_> = candidates.iter().map(|(_, _, e)| *e).collect();
    let mut shadowed = HashSet::new();
    if cfg.shadows {
        for (_, _, entity) in &candidates {
            let Ok((_, slot, _, _)) = slots.get(*entity) else { continue };
            if slot.placement.cast_shadow && slot.placement.radius > 0.2 && shadowed.len() < settings.max_shadow_lights {
                shadowed.insert(*entity);
            }
        }
    }
    for (entity, slot, _, current) in &slots {
        if !chosen.contains(&entity) {
            if current.is_some() {
                commands.entity(entity).remove::<PointLight>();
            }
            continue;
        }
        let p = &slot.placement;
        let color = Color::linear_rgb(p.diffuse.x, p.diffuse.y, p.diffuse.z);
        let intensity = lumens(p.radius, cfg.sun_lux_scale.max(0.0));
        let shadows = shadowed.contains(&entity);
        if current.is_none_or(|old| {
            old.color != color || old.range != p.radius || old.intensity != intensity || old.shadow_maps_enabled != shadows
        }) {
            commands.entity(entity).insert(PointLight {
                color,
                intensity,
                range: p.radius,
                shadow_maps_enabled: shadows,
                ..default()
            });
        }
    }
    result.rendered = chosen.len();
    result.shadowed = shadowed.len();
    *stats = result;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[test]
    fn local_light_budget_honors_inactive_hidden_slots_and_releases_render_components() {
        let source = r#"<Mission><Objects>
          <Entity Name="near" EntityClass="Light" Pos="1,0,0"><Properties Radius="5"><Options bCastShadow="1"/></Properties></Entity>
          <Entity Name="second" EntityClass="Light" Pos="3,0,0"><Properties Radius="8"><Options bCastShadow="1"/></Properties></Entity>
          <Entity Name="far" EntityClass="Light" Pos="10,0,0"><Properties Radius="6"/></Entity>
          <Entity Name="inactive" EntityClass="Light"><Properties bActive="0"/></Entity>
          <Entity Name="hidden" EntityClass="Light" HiddenInGame="1"><Properties/></Entity>
          <Entity Name="projector" EntityClass="Light"><Properties><Projector texture_Texture="textures/lights/pattern.dds"/></Properties></Entity>
          <Entity Name="fake" EntityClass="Light"><Properties><Options bFakeLight="1"/></Properties></Entity>
        </Objects></Mission>"#;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(WorldConfig::new("main_world"))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(250)))
            .insert_resource(LocalLightSettings {
                enabled: true,
                distance: 30.0,
                max_lights: 2,
                max_shadow_lights: 1,
            });
        build(&mut app);
        app.world_mut().spawn((WorldFocus, GlobalTransform::default()));
        let placed = crate::entities::parse_lights(source);
        app.world_mut().commands().queue(move |world: &mut World| {
            let mut commands = world.commands();
            spawn_cell(&mut commands, (0, 0), &placed);
        });
        app.update();
        app.update();
        let world = app.world_mut();
        let drawn: Vec<_> = world
            .query::<(&Name, &PointLight)>()
            .iter(world)
            .map(|(n, p)| (n.as_str().to_string(), p.range, p.shadow_maps_enabled))
            .collect();
        assert_eq!(drawn.len(), 2);
        assert!(drawn.contains(&("near".to_string(), 5.0, true)));
        assert!(drawn.contains(&("second".to_string(), 8.0, false)));
        assert_eq!(world.resource::<LocalLightStats>().authored, 7);
        assert_eq!(world.resource::<LocalLightStats>().unsupported, 2);
        world.resource_mut::<LocalLightSettings>().enabled = false;
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query::<&PointLight>().iter(world).count(), 0);
        assert_eq!(
            world.query::<&LocalLight>().iter(world).count(),
            7,
            "dormant authored slots retain their data"
        );
    }
}
