//! Tool props: the shovel of uproot, the watering pot, the tools an NPC pose holds.
//!
//! The data links a skill or an `anim_actions` pose to a tool item (`mainhand_tool_id` / `offhand_tool_id`), the item to its
//! model and attachment offsets through `item_tools` -> `item_assets`. A skill's [`SkillTool`] is put on its caster while one
//! of its clips plays (the main hand model on `item_hand_r`, the off hand one on `item_hand_l`, as [`super::weapons`] maps
//! weapons); a pose's [`PostureTool`] while the NPC stands in it. [`hold_tools`] keeps one child entity of the hand bone per
//! hand, and takes it away when the action ends, is interrupted or the unit dies: a tool never outlives the clip it belongs to.
//!
//! The attachment transform is the `item_assets` offset and rotation under the hand bone, handled exactly like the carried
//! weapons ([`super::weapons::attach_carried`]; the rotation is CryEngine's, `Rz * Ry * Rx`).

use aa_anim::Animator;
use aa_cry::scene::{CryAssets, ModelOptions, Shading, spawn_loaded};
use bevy::prelude::*;

use super::combat::{Acting, Dead};
use super::data::Tool;
use super::weapons;

/// The tools a skill's clip puts in its caster's hands: main hand first (`None` in a hand that holds none).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct SkillTool {
    pub items: [Option<Tool>; 2],
}

/// The tools an NPC's pose holds (`anim_actions.mainhand_tool_id` / `offhand_tool_id`): main hand first.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct PostureTool {
    pub items: [Option<Tool>; 2],
}

/// Which hand a tool is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Main,
    Off,
}

impl Hand {
    const ALL: [Hand; 2] = [Hand::Main, Hand::Off];

    fn index(self) -> usize {
        match self {
            Hand::Main => 0,
            Hand::Off => 1,
        }
    }

    /// The hand bone a weapon of the main or off hand is held by (`weapons::carried`: blades and tools in the right hand,
    /// the off hand ones in the left).
    fn bone(self) -> &'static str {
        match self {
            Hand::Main => "item_hand_r",
            Hand::Off => "item_hand_l",
        }
    }
}

/// A tool model spawned under the hand bone of its owner.
#[derive(Component)]
pub struct HeldTool {
    pub owner: Entity,
    pub hand: Hand,
    pub item: Tool,
}

/// The model parts of a [`HeldTool`] were loaded (or tried): do not load them again every frame.
#[derive(Component)]
struct Dressed;

pub fn plugin(app: &mut App) {
    app.add_systems(PostUpdate, (end_skill_tools, hold_tools, dress_tools).chain().before(TransformSystems::Propagate));
}

/// A skill's tool is part of its action: when [`Acting`] ends (the clip ran out, a stop marker, an end event, an interrupted
/// cast, a death) the tool goes with it.
pub(super) fn end_skill_tools(mut commands: Commands, units: Query<(Entity, &SkillTool), Without<Acting>>) {
    for (entity, _) in &units {
        commands.entity(entity).remove::<SkillTool>();
    }
}

/// The tool `hand` a unit holds now: a skill's over the posture's in the same hand, none while dead.
fn wanted<'a>(skill: Option<&'a SkillTool>, posture: Option<&'a PostureTool>, acting: bool, dead: bool, hand: Hand) -> Option<&'a Tool> {
    if dead {
        return None;
    }
    skill
        .filter(|_| acting)
        .and_then(|s| s.items[hand.index()].as_ref())
        .or_else(|| posture.and_then(|p| p.items[hand.index()].as_ref()))
}

/// Attaches the wanted tools to the hand bones and removes the ones that are no longer wanted (a skill ended, the pose
/// changed, the owner went).
#[allow(clippy::type_complexity)]
pub(super) fn hold_tools(
    mut commands: Commands,
    units: Query<(Entity, Option<&SkillTool>, Option<&PostureTool>, Has<Acting>, Has<Dead>), Or<(With<SkillTool>, With<PostureTool>)>>,
    animators: Query<&Animator>,
    held: Query<(Entity, &HeldTool)>,
) {
    // what no longer matches the unit's state goes
    for (entity, tool) in &held {
        let wanted_now = units.get(tool.owner).ok().and_then(|(_, skill, posture, acting, dead)| wanted(skill, posture, acting, dead, tool.hand));
        if wanted_now != Some(&tool.item) {
            commands.entity(entity).despawn();
        }
    }
    // and what is wanted and not there yet is spawned under the hand bone
    for (unit, skill, posture, acting, dead) in &units {
        if dead {
            continue;
        }
        let Some(animator) = animators.get(unit).ok() else { continue };
        for hand in Hand::ALL {
            let Some(item) = wanted(skill, posture, acting, dead, hand) else { continue };
            if held.iter().any(|(_, h)| h.owner == unit && h.hand == hand && h.item == *item) {
                continue;
            }
            let Some(bone) = animator.rig.skeleton.bones.iter().position(|b| b.name == hand.bone()).and_then(|i| animator.bones.get(i).copied()) else {
                continue;
            };
            commands.spawn((
                Name::new(format!("tool {}", item.path.rsplit('/').next().unwrap_or(""))),
                HeldTool { owner: unit, hand, item: item.clone() },
                tool_transform(item),
                Visibility::default(),
                ChildOf(bone),
            ));
        }
    }
}

/// The local transform of a tool under its hand bone (the `item_assets` offset and CryEngine rotation, as the carried models).
fn tool_transform(item: &Tool) -> Transform {
    Transform {
        translation: Vec3::from(item.offset),
        rotation: weapons::attachment_rotation(item.rotation),
        scale: Vec3::ONE,
    }
}

/// Loads the model of a tool anchor once (the shovel is one `.cgf`; a `.cdf` may be several parts, as for weapons).
fn dress_tools(mut commands: Commands, mut cry: CryAssets, anchors: Query<(Entity, &HeldTool), Without<Dressed>>) {
    for (anchor, held) in &anchors {
        let options = ModelOptions { cry_space: true, shading: Shading::Full, ..default() };
        for (path, material) in weapons::model_files(&held.item.path) {
            let options = ModelOptions { material, ..options.clone() };
            let Ok(loaded) = cry.load_model(&path, &options) else { continue };
            let part = spawn_loaded(&mut commands, &path, &loaded, &options);
            commands.entity(part).insert(ChildOf(anchor));
        }
        commands.entity(anchor).insert(Dressed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_cry::scene::CryPlugin;
    use aa_cry::skeleton::{Bone, Skeleton};
    use bevy::asset::AssetPlugin;
    use bevy::mesh::skinning::SkinnedMeshInverseBindposes;

    fn test_item(name: &str) -> Tool {
        Tool { path: format!("objects/item/{name}.cgf"), offset: [-0.01, 0.16, 0.0], rotation: [10.0, 88.88, 0.0] }
    }

    /// A unit with an animator whose skeleton has both hand bones, and the bone entities it points at.
    fn unit_with_hands(app: &mut App) -> (Entity, Entity, Entity) {
        let bone = |name: &str| Bone {
            name: name.into(),
            controller_id: name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))),
            parent: None,
            limb_id: 0,
            bind_model: default(),
            bind_local: default(),
            physics: [default(); 2],
        };
        let rig = std::sync::Arc::new(aa_anim::pose::Rig::new(Skeleton { bones: vec![bone("item_hand_r"), bone("item_hand_l")] }));
        let right = app.world_mut().spawn(()).id();
        let left = app.world_mut().spawn(()).id();
        let unit = app.world_mut().spawn(Animator::new(rig, vec![right, left])).id();
        (unit, right, left)
    }

    #[test]
    fn a_skill_tool_is_spawned_under_the_hand_and_goes_when_the_action_ends() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, (end_skill_tools, hold_tools).chain());
        let (unit, right, left) = unit_with_hands(&mut app);
        let shovel = test_item("shovel_general");
        app.world_mut().entity_mut(unit).insert((
            Acting { until: 100.0, layered: false },
            SkillTool { items: [None, Some(shovel.clone())] },
        ));
        app.update();
        app.update();
        let held: Vec<(Entity, &HeldTool)> = app.world_mut().query::<(Entity, &HeldTool)>().iter(app.world()).collect();
        assert_eq!(held.len(), 1, "one off hand tool");
        let (entity, tool) = held[0];
        assert_eq!((tool.owner, tool.hand, tool.item.clone()), (unit, Hand::Off, shovel));
        assert_eq!(app.world().get::<ChildOf>(entity).unwrap().parent(), left, "the off hand bone");

        // the skill ended (an end event or an interrupted cast removes `Acting`): the tool goes too
        app.world_mut().entity_mut(unit).remove::<Acting>();
        app.update();
        app.update();
        assert!(app.world().get_entity(entity).is_err(), "the tool does not outlive the action");
        assert_eq!(app.world_mut().query::<&HeldTool>().iter(app.world()).count(), 0);
        assert!(app.world().get_entity(right).is_ok(), "the bones stay");
    }

    #[test]
    fn main_and_off_hand_tools_sit_on_their_own_bones_and_a_pose_tool_lives_while_standing() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, (end_skill_tools, hold_tools).chain());
        let (unit, right, left) = unit_with_hands(&mut app);
        let pot = test_item("quest_pot");
        app.world_mut().entity_mut(unit).insert(PostureTool { items: [Some(pot.clone()), None] });
        app.update();
        app.update();
        let held: Vec<(Entity, &HeldTool)> = app.world_mut().query::<(Entity, &HeldTool)>().iter(app.world()).collect();
        assert_eq!(held.len(), 1);
        assert_eq!((held[0].1.hand, held[0].1.item.clone()), (Hand::Main, pot));
        assert_eq!(app.world().get::<ChildOf>(held[0].0).unwrap().parent(), right, "the main hand bone");
        app.world_mut().entity_mut(unit).remove::<PostureTool>();
        app.update();
        app.update();
        assert_eq!(app.world_mut().query::<&HeldTool>().iter(app.world()).count(), 0);
        assert!(app.world().get_entity(left).is_ok());
    }

    #[test]
    fn the_plugin_validates_its_system_parameters() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), CryPlugin))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<Image>()
            .init_asset::<SkinnedMeshInverseBindposes>()
            .add_plugins(super::plugin);
        app.update();
        app.update();
    }
}
