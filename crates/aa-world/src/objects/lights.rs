//! Authored local lights, with cell ownership and a bounded nearby render set.
//!
//! Bevy lights use inverse-square falloff. Native points use (1-d²/r²)² and projectors
//! use (1-d²/r²); conversions match diffuse illuminance at half the authored radius. They do not
//! invent light sources, scale radii with entity Scale, or convert projectors into omni lights.

use std::collections::HashSet;

use aa_cry::shade::illum::IllumLightSpecular;
use bevy::light::SpotLightTexture;
use bevy::prelude::*;

mod hdr;
mod projectors;
pub mod styles;

use crate::coords::{cell_origin, cry_to_bevy};
use crate::entities::PlacedLight;
use crate::stream::{WorldConfig, WorldFocus};

#[derive(Resource, Clone, Debug)]
pub struct LocalLightSettings {
    pub enabled: bool,
    /// Distance to the light's influence sphere, in metres.
    pub distance: f32,
    /// GPU light components (an RGB projector costs three); capped at 64.
    pub max_lights: usize,
    /// Point-light shadows require six views each; nearest authored requests, capped at two.
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
    /// RGB sources; each one consumes three of `rendered`'s light components.
    pub projected_sources: usize,
    pub cookie_assets: usize,
    pub pending_cookies: usize,
}

/// Authored local-light rendering without terrain streaming (also added by AaWorldPlugin).
/// The application supplies WorldConfig and owns the entities returned by spawn_cell.
pub struct LocalLightPlugin;

#[derive(Resource)]
struct ProjectorSupport(bool);

impl Plugin for LocalLightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalLightSettings>()
            .init_resource::<LocalLightStats>()
            .init_resource::<projectors::Cookies>()
            .init_resource::<Assets<Image>>()
            .init_resource::<styles::LightStyles>()
            .init_resource::<styles::LightStyleSettings>()
            .insert_resource(ProjectorSupport(false))
            .add_systems(Update, (update_lights, styles::animate).chain().after(super::stream_objects));
    }

    fn finish(&self, app: &mut App) {
        use bevy::render::{
            RenderApp,
            renderer::{RenderAdapter, RenderDevice},
        };
        let supported = app.get_sub_app(RenderApp).is_some_and(|render| {
            render
                .world()
                .get_resource::<RenderDevice>()
                .zip(render.world().get_resource::<RenderAdapter>())
                .is_some_and(|(device, adapter)| {
                    bevy::pbr::decal::clustered::clustered_decals_are_usable(device, adapter)
                })
        });
        app.insert_resource(ProjectorSupport(supported));
    }
}

pub(crate) fn build(app: &mut App) {
    app.add_plugins(LocalLightPlugin);
}

/// Spawn authored slots; despawning these roots also removes their RGB light children.
pub fn spawn_cell(
    commands: &mut Commands,
    cell: (i32, i32),
    placed: &[PlacedLight],
) -> Vec<Entity> {
    let origin = cell_origin(cell.0, cell.1);
    placed
        .iter()
        .map(|p| {
            let mut transform =
                Transform::from_translation(cry_to_bevy(origin + p.matrix.w_axis.truncate()));
            if let Some(projection) = projectors::projection(p) {
                transform.rotation = projection.rotation;
                transform.scale = Vec3::new(projection.xy_scale, projection.xy_scale, 1.0);
            }
            commands
                .spawn((
                    Name::new(p.name.clone()),
                    LocalLight {
                        cell,
                        placement: p.clone(),
                    },
                    transform,
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
    specular_ratios: Query<&IllumLightSpecular>,
    slots: Query<(
        Entity,
        &LocalLight,
        &Transform,
        Option<&PointLight>,
        Option<&projectors::Channels>,
    )>,
    mut cookies: ResMut<projectors::Cookies>,
    mut images: ResMut<Assets<Image>>,
    projector_support: Res<ProjectorSupport>,
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
    for (entity, slot, transform, _, _) in &slots {
        result.authored += 1;
        let p = &slot.placement;
        let projection = projectors::projection(p);
        if (!p.supported() && (projection.is_none() || !projector_support.0))
            || (projection.is_none() && hdr::color(p.diffuse, 1.0, p.hdr_dynamic).is_none())
        {
            result.unsupported += 1;
            continue;
        }
        if !settings.enabled
            || !cfg.objects
            || !p.active
            || p.hidden
            || p.diffuse.max_element() <= 0.0
        {
            continue;
        }
        let Some(camera) = camera else { continue };
        let distance = camera.distance(transform.translation);
        if distance - p.radius <= settings.distance {
            candidates.push((distance, p.guid, entity, projection));
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    cookies.update(
        &cfg.world,
        candidates.iter().filter_map(|(_, _, entity, projection)| {
            let (_, slot, _, _, _) = slots.get(*entity).ok()?;
            projection
                .as_ref()
                .map(|_| slot.placement.projector_texture.as_str())
        }),
        &mut images,
    );
    let mut chosen = HashSet::new();
    for (_, _, entity, projection) in &candidates {
        let Ok((_, slot, _, _, _)) = slots.get(*entity) else {
            continue;
        };
        let cost = if projection.is_some() {
            if cookies.get(&slot.placement.projector_texture).is_none() {
                result.unsupported +=
                    usize::from(cookies.unsupported(&slot.placement.projector_texture));
                continue;
            }
            3
        } else {
            1
        };
        if result.rendered + cost > settings.max_lights.min(64) {
            continue;
        }
        chosen.insert(*entity);
        result.rendered += cost;
        result.projected_sources += usize::from(projection.is_some());
    }
    let mut shadowed = HashSet::new();
    if cfg.shadows {
        for (_, _, entity, _) in &candidates {
            let Ok((_, slot, _, _, _)) = slots.get(*entity) else {
                continue;
            };
            if chosen.contains(entity)
                && slot.placement.cast_shadow
                && slot.placement.radius > 0.2
                && shadowed.len() < settings.max_shadow_lights.min(2)
            {
                shadowed.insert(*entity);
            }
        }
    }
    for (entity, slot, _, current, projected) in &slots {
        if !chosen.contains(&entity) {
            if current.is_some() {
                commands
                    .entity(entity)
                    .remove::<(PointLight, IllumLightSpecular)>();
            }
            if let Some(projected) = projected {
                for child in projected.0 {
                    commands.entity(child).despawn();
                }
                commands.entity(entity).remove::<projectors::Channels>();
            }
            continue;
        }
        let p = &slot.placement;
        if let Some(projection) = projectors::projection(p) {
            let Some(images) = cookies.get(&p.projector_texture) else {
                continue;
            };
            let children = projected.map(|p| p.0).unwrap_or_else(|| {
                let children = std::array::from_fn(|_| {
                    commands
                        .spawn((ChildOf(entity), Transform::default(), Visibility::default()))
                        .id()
                });
                commands
                    .entity(entity)
                    .insert(projectors::Channels(children));
                children
            });
            for channel in 0..3 {
                let mut color = Vec3::ZERO;
                color[channel] = p.diffuse[channel];
                commands.entity(children[channel]).insert((
                    SpotLight {
                        color: Color::linear_rgb(color.x, color.y, color.z),
                        intensity: projectors::lumens(p.radius, cfg.sun_lux_scale.max(0.0)),
                        range: p.radius,
                        inner_angle: projection.inner_angle,
                        outer_angle: projection.outer_angle,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    SpotLightTexture {
                        image: images[channel].clone(),
                    },
                    IllumLightSpecular(p.specular_multiplier),
                ));
            }
            continue;
        }
        let color = Color::linear_rgb(p.diffuse.x, p.diffuse.y, p.diffuse.z);
        let intensity = lumens(p.radius, cfg.sun_lux_scale.max(0.0));
        let shadows = shadowed.contains(&entity);
        if current.is_none_or(|old| {
            old.range != p.radius
                || old.intensity != intensity
                || old.shadow_maps_enabled != shadows
        }) {
            commands.entity(entity).insert((
                PointLight {
                    color,
                    intensity,
                    range: p.radius,
                    shadow_maps_enabled: shadows,
                    ..default()
                },
                IllumLightSpecular(p.specular_multiplier),
            ));
        } else if !specular_ratios.get(entity).is_ok_and(|ratio| ratio.0 == p.specular_multiplier) {
            commands.entity(entity).insert(IllumLightSpecular(p.specular_multiplier));
        }
    }
    result.shadowed = shadowed.len();
    (result.cookie_assets, result.pending_cookies) = cookies.counts();
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
          <Entity Name="near" EntityClass="Light" Pos="1,0,0"><Properties Radius="5"><Color fDiffuseMultiplier="2" fSpecularMultiplier="1"/><Options bCastShadow="1"/></Properties></Entity>
          <Entity Name="second" EntityClass="Light" Pos="3,0,0"><Properties Radius="8"><Color fDiffuseMultiplier="3" fSpecularMultiplier="1"/><Options bCastShadow="1"/></Properties></Entity>
          <Entity Name="far" EntityClass="Light" Pos="10,0,0"><Properties Radius="6"><Color fSpecularMultiplier="0"/></Properties></Entity>
          <Entity Name="inactive" EntityClass="Light"><Properties bActive="0"/></Entity>
          <Entity Name="hidden" EntityClass="Light" HiddenInGame="1"><Properties/></Entity>
          <Entity Name="projector" EntityClass="Light"><Properties><Projector texture_Texture="textures/lights/pattern.dds"/></Properties></Entity>
          <Entity Name="fake" EntityClass="Light"><Properties><Options bFakeLight="1"/></Properties></Entity>
        </Objects></Mission>"#;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(WorldConfig::new("main_world"))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                250,
            )))
            .insert_resource(LocalLightSettings {
                enabled: true,
                distance: 30.0,
                max_lights: 2,
                max_shadow_lights: 1,
            });
        build(&mut app);
        app.world_mut()
            .spawn((WorldFocus, GlobalTransform::default()));
        let placed = crate::entities::parse_lights(source);
        app.world_mut().commands().queue(move |world: &mut World| {
            let mut commands = world.commands();
            spawn_cell(&mut commands, (0, 0), &placed);
        });
        app.update();
        app.update();
        let world = app.world_mut();
        let drawn: Vec<_> = world
            .query::<(&Name, &PointLight, &IllumLightSpecular)>()
            .iter(world)
            .map(|(n, p, s)| (n.as_str().to_string(), p.range, p.shadow_maps_enabled, s.0))
            .collect();
        assert_eq!(drawn.len(), 2);
        assert!(drawn.contains(&("near".to_string(), 5.0, true, 0.5)));
        assert!(drawn.contains(&("second".to_string(), 8.0, false, 1.0 / 3.0)));
        assert_eq!(world.resource::<LocalLightStats>().authored, 7);
        assert_eq!(world.resource::<LocalLightStats>().unsupported, 2);
        let near = world.query::<(Entity, &Name)>().iter(world)
            .find(|(_,name)|name.as_str()=="near").unwrap().0;
        let point_fields = |light: &PointLight| (light.color, light.intensity, light.range, light.shadow_maps_enabled);
        let original_point = point_fields(world.get::<PointLight>(near).unwrap());
        world.get_mut::<LocalLight>(near).unwrap().placement.specular_multiplier = 0.0;
        app.update();
        let world = app.world_mut();
        assert_eq!(world.get::<IllumLightSpecular>(near).unwrap().0, 0.0,
            "ratio-only changes must refresh even when PointLight fields are unchanged");
        assert_eq!(point_fields(world.get::<PointLight>(near).unwrap()), original_point);
        world.entity_mut(near).remove::<IllumLightSpecular>();
        app.update();
        let world = app.world_mut();
        assert_eq!(world.get::<IllumLightSpecular>(near).unwrap().0, 0.0,
            "missing ratios must be restored, including explicit zero");
        assert_eq!(point_fields(world.get::<PointLight>(near).unwrap()), original_point);
        world.resource_mut::<LocalLightSettings>().max_lights = 3;
        app.update();
        let world = app.world_mut();
        assert!(world.query::<(&Name, &PointLight, &IllumLightSpecular)>().iter(world)
            .any(|(name, light, specular)| name.as_str() == "far" && light.intensity > 0.0 && specular.0 == 0.0),
            "explicit zero suppresses specular while retaining the point's diffuse light");
        world.resource_mut::<LocalLightSettings>().enabled = false;
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query::<&PointLight>().iter(world).count(), 0);
        assert_eq!(world.query::<&IllumLightSpecular>().iter(world).count(), 0);
        assert_eq!(
            world.query::<&LocalLight>().iter(world).count(),
            7,
            "dormant authored slots retain their data"
        );
    }

    #[test]
    fn rgb_budget_is_atomic_and_root_unload_removes_all_channels_and_cookie_assets() {
        if !crate::pak_available() {
            return;
        }
        let xml = aa_pak::Pak::global()
            .read("game/worlds/instance_library/cells/000_004/client/entities.xml")
            .unwrap();
        let source = crate::entities::parse_lights(std::str::from_utf8(&xml).unwrap());
        let light = source.into_iter().find(|p| p.name == "Light_587").unwrap();
        let specular_ratio = light.specular_multiplier;
        let position = light.matrix.w_axis.truncate();
        let mut second = light.clone();
        second.guid += 1;
        second.matrix.w_axis.x += 1.0;
        let point_xml = format!(
            r#"<Mission><Objects><Entity Name="point" EntityClass="Light" Pos="{},{},{}"><Properties Radius="5"/></Entity></Objects></Mission>"#,
            position.x, position.y, position.z
        );
        let point = crate::entities::parse_lights(&point_xml).remove(0);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(WorldConfig::new("instance_library"))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                250,
            )))
            .insert_resource(LocalLightSettings {
                max_lights: 2,
                ..default()
            });
        build(&mut app);
        app.insert_resource(ProjectorSupport(true)); // CPU lifecycle test; no renderer exists.
        app.world_mut().spawn((
            WorldFocus,
            GlobalTransform::from_translation(cry_to_bevy(position)),
        ));
        app.world_mut().commands().queue(move |world: &mut World| {
            spawn_cell(&mut world.commands(), (0, 0), &[light, second, point]);
        });
        for _ in 0..200 {
            app.update();
            if app.world().resource::<LocalLightStats>().cookie_assets == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.world().resource::<LocalLightStats>().cookie_assets, 1);
        assert_eq!(
            app.world().resource::<LocalLightStats>().rendered,
            1,
            "a two-light cap can draw the point, never partial RGB"
        );
        assert_eq!(
            app.world_mut()
                .query::<&SpotLight>()
                .iter(app.world())
                .count(),
            0
        );
        app.world_mut()
            .resource_mut::<LocalLightSettings>()
            .max_lights = 4;
        app.update();
        assert_eq!(app.world().resource::<LocalLightStats>().rendered, 4);
        assert_eq!(
            app.world().resource::<LocalLightStats>().projected_sources,
            1
        );
        let spots: Vec<_> = app
            .world_mut()
            .query::<(&SpotLight, &IllumLightSpecular)>()
            .iter(app.world())
            .collect();
        assert_eq!(spots.len(), 3);
        assert!(spots.iter().all(|(light, specular)| !light.shadow_maps_enabled && specular.0 == specular_ratio));
        let roots: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<LocalLight>>()
            .iter(app.world())
            .collect();
        for entity in roots {
            app.world_mut().despawn(entity);
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&SpotLight>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(app.world().resource::<LocalLightStats>().rendered, 0);
        assert_eq!(app.world().resource::<LocalLightStats>().cookie_assets, 0);
    }
}
