//! What a loaded model tells the game about itself, and the animation
//! clips it brought along.
//!
//! **Bounds.** Once a model's meshes are in, `measure_models` works out
//! where its lowest point is (so the creature rests its legs on the ground
//! instead of its belly, and Judith's feet meet the floor), its top, and
//! the **saddle**: the top of the body at its middle, where a rider sits.
//!
//! **Clips.** A GLB that was rigged and animated (Meshy's *Animate*, Mixamo,
//! Blender…) arrives with a skeleton and clips. Those are played as they
//! are — the game's own rigger stays out of it — through Bevy's
//! `AnimationPlayer`, cross-fading between clips as the figure's `Gait`,
//! `Stance` or `Flap` asks: idle, walk, run, jump, swim, sit for a rider,
//! fly for a creature. Clips are matched to actions by name.

use std::collections::HashMap;
use std::time::Duration;

use bevy::gltf::Gltf;
use bevy::math::Affine3A;
use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy::prelude::*;

use crate::creature::Flap;
use crate::models::SwappedModel;
use crate::rig::{Gait, Stance};

/// The shape of a model, in the frame of the entity it replaced.
#[derive(Component, Debug, Clone, Copy)]
pub struct ModelBounds {
    /// Lowest point (usually ≤ 0: below the entity's origin).
    pub bottom: f32,
    pub top: f32,
    /// Where a rider sits: the top of the body at its middle.
    pub saddle: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Idle,
    Walk,
    Run,
    Crouch,
    Jump,
    Swim,
    Ride,
    Fly,
}

impl Action {
    /// Which action a clip's name describes.
    fn from_name(name: &str) -> Option<Action> {
        let n = name.to_ascii_lowercase();
        let has = |ws: &[&str]| ws.iter().any(|w| n.contains(w));
        if has(&["swim"]) {
            Some(Action::Swim)
        } else if has(&["jump", "leap", "hop"]) {
            Some(Action::Jump)
        } else if has(&["crouch", "sneak", "creep", "stealth"]) {
            Some(Action::Crouch)
        } else if has(&["sit", "ride", "riding", "horse", "saddle"]) {
            Some(Action::Ride)
        } else if has(&["run", "sprint", "jog"]) {
            Some(Action::Run)
        } else if has(&["walk"]) {
            Some(Action::Walk)
        } else if has(&["fly", "flap", "hover", "flight"]) {
            Some(Action::Fly)
        } else if has(&["idle", "stand", "breath", "rest"]) {
            Some(Action::Idle)
        } else {
            None
        }
    }

    /// What to play when an action has no clip of its own.
    fn fallbacks(self) -> &'static [Action] {
        match self {
            Action::Run => &[Action::Walk, Action::Idle],
            Action::Crouch => &[Action::Walk, Action::Idle],
            Action::Jump | Action::Swim | Action::Ride => &[Action::Idle, Action::Walk],
            Action::Walk => &[Action::Run, Action::Idle],
            Action::Fly => &[Action::Idle],
            Action::Idle => &[Action::Walk],
        }
    }
}

/// A model animated by its own clips.
#[derive(Component, Debug)]
pub struct Clips {
    pub player: Entity,
    pub actions: HashMap<Action, AnimationNodeIndex>,
    /// Any clip at all, for a model whose names say nothing.
    pub first: AnimationNodeIndex,
    pub current: Option<Action>,
}

/// Measured (and, if it has clips, wired up).
#[derive(Component)]
pub struct Measured;

fn descendants(e: Entity, children: &Query<&Children>, out: &mut Vec<Entity>) {
    if let Ok(kids) = children.get(e) {
        for k in kids.iter() {
            out.push(k);
            descendants(k, children, out);
        }
    }
}

fn frame_to(
    e: Entity,
    top: Entity,
    parents: &Query<&ChildOf>,
    transforms: &Query<&Transform>,
) -> Affine3A {
    let mut m = Affine3A::IDENTITY;
    let mut cur = e;
    while cur != top {
        if let Ok(t) = transforms.get(cur) {
            m = t.compute_affine() * m;
        }
        match parents.get(cur) {
            Ok(p) => cur = p.parent(),
            Err(_) => break,
        }
    }
    m
}

/// Measures every loaded model and adopts its clips.
fn measure_models(
    mut commands: Commands,
    swapped: Query<(Entity, &SwappedModel, &ChildOf), Without<Measured>>,
    children: Query<&Children>,
    meshes3d: Query<&Mesh3d>,
    players: Query<(), With<AnimationPlayer>>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    meshes: Res<Assets<Mesh>>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for (root, swap, parent) in &swapped {
        let mut all = Vec::new();
        descendants(root, &children, &mut all);
        let mesh_entities: Vec<Entity> = all
            .iter()
            .copied()
            .filter(|e| meshes3d.contains(*e))
            .collect();
        if mesh_entities.is_empty() {
            continue; // the scene hasn't spawned yet
        }
        let handles: Vec<Handle<Mesh>> = mesh_entities
            .iter()
            .filter_map(|e| meshes3d.get(*e).ok().map(|m| m.0.clone()))
            .collect();
        if handles.iter().any(|h| meshes.get(h).is_none()) {
            continue; // still loading
        }
        // Points in the frame of the entity the model replaced.
        let mut points = Vec::new();
        for (e, h) in mesh_entities.iter().zip(&handles) {
            let m = frame_to(*e, parent.parent(), &parents, &transforms);
            if let Some(VertexAttributeValues::Float32x3(ps)) = meshes
                .get(h)
                .and_then(|mesh| mesh.attribute(Mesh::ATTRIBUTE_POSITION))
            {
                points.extend(ps.iter().map(|p| m.transform_point3(Vec3::from(*p))));
            }
        }
        if points.is_empty() {
            commands.entity(root).insert(Measured);
            continue;
        }
        let bounds = measure(&points);
        info!(
            "{:?} model: bottom {:.2} m, top {:.2} m, saddle at {:.2} m",
            swap.kind, bounds.bottom, bounds.top, bounds.saddle.y
        );
        commands.entity(parent.parent()).insert(bounds);

        // Clips: the glTF's animations, played on the scene's own player.
        let gltf = gltfs.get(&swap.gltf);
        let player = all.iter().copied().find(|e| players.contains(*e));
        if let (Some(gltf), Some(player)) = (gltf, player)
            && !gltf.animations.is_empty()
        {
            let names: Vec<(String, Handle<AnimationClip>)> = gltf
                .named_animations
                .iter()
                .map(|(n, h)| (n.to_string(), h.clone()))
                .collect();
            let (graph, nodes) = AnimationGraph::from_clips(names.iter().map(|(_, h)| h.clone()));
            let mut actions = HashMap::new();
            for ((name, _), node) in names.iter().zip(&nodes) {
                if let Some(a) = Action::from_name(name) {
                    actions.entry(a).or_insert(*node);
                }
            }
            info!(
                "{:?} model brings {} clip(s): {} → {:?}",
                swap.kind,
                names.len(),
                names
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                actions.keys().collect::<Vec<_>>()
            );
            commands.entity(player).insert((
                AnimationGraphHandle(graphs.add(graph)),
                AnimationTransitions::new(),
            ));
            commands.entity(root).insert((
                Clips {
                    player,
                    actions,
                    first: nodes[0],
                    current: None,
                },
                crate::rig::Rigged,
            ));
        }
        commands.entity(root).insert(Measured);
    }
}

/// Bottom, top and saddle of a cloud of points in a creature's own frame
/// (forward −Z, up +Y).
pub fn measure(points: &[Vec3]) -> ModelBounds {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for p in points {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let size = (hi - lo).max(Vec3::splat(1e-4));
    let c = (lo + hi) * 0.5;
    // The saddle: the top of the body along the centre line, around the
    // middle of its length (ahead of the abdomen, behind the head).
    let mut saddle_y = f32::MIN;
    let mut saddle_z = c.z;
    for p in points {
        if (p.x - c.x).abs() < 0.08 * size.x && (p.z - c.z).abs() < 0.12 * size.z && p.y > saddle_y
        {
            saddle_y = p.y;
            saddle_z = p.z;
        }
    }
    if saddle_y == f32::MIN {
        saddle_y = hi.y;
    }
    ModelBounds {
        bottom: lo.y,
        top: hi.y,
        saddle: Vec3::new(c.x, saddle_y, saddle_z),
    }
}

/// Picks and plays the clip each animated model should be in.
fn drive_clips(
    mut clips: Query<(&mut Clips, &ChildOf)>,
    drivers: Query<(
        Option<&Gait>,
        Option<&Stance>,
        Option<&Flap>,
        Option<&ChildOf>,
    )>,
    mounts: Query<(), With<Flap>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    for (mut clips, parent) in &mut clips {
        let Ok((gait, stance, flap, grand)) = drivers.get(parent.parent()) else {
            continue;
        };
        let (wanted, speed) = if let Some(flap) = flap {
            (Action::Fly, 0.6 + 0.9 * flap.intensity)
        } else if let Some(gait) = gait {
            let mounted = grand.is_some_and(|g| mounts.contains(g.parent()));
            if mounted {
                (Action::Ride, 1.0)
            } else {
                match stance.copied().unwrap_or_default() {
                    Stance::Swim => (Action::Swim, 1.0),
                    Stance::Air { .. } => (Action::Jump, 1.0),
                    Stance::Crouch if gait.speed > 0.3 => (Action::Crouch, 1.0),
                    Stance::Crouch => (Action::Idle, 1.0),
                    Stance::Stand if gait.speed < 0.3 => (Action::Idle, 1.0),
                    Stance::Stand if gait.speed < 5.5 => {
                        (Action::Walk, (gait.speed / 1.5).clamp(0.6, 1.6))
                    }
                    Stance::Stand => (Action::Run, (gait.speed / 4.5).clamp(0.7, 1.5)),
                }
            }
        } else {
            (Action::Idle, 1.0)
        };
        // The clip for it, or the nearest thing the model has.
        let (action, node) = std::iter::once(wanted)
            .chain(wanted.fallbacks().iter().copied())
            .find_map(|a| clips.actions.get(&a).map(|n| (a, *n)))
            .unwrap_or((Action::Idle, clips.first));
        let Ok((mut player, mut transitions)) = players.get_mut(clips.player) else {
            continue;
        };
        if clips.current != Some(action) {
            transitions
                .play(&mut player, node, Duration::from_millis(220))
                .repeat();
            clips.current = Some(action);
        }
        if let Some(active) = player.animation_mut(node) {
            active.set_speed(speed);
        }
    }
}

/// Seats riders on a model's saddle once it is known.
fn seat_riders(
    mounts: Query<(Entity, &ModelBounds), Added<ModelBounds>>,
    children: Query<&Children>,
    mut riders: Query<&mut Transform, With<Gait>>,
) {
    for (e, b) in &mounts {
        if let Ok(kids) = children.get(e) {
            for k in kids.iter() {
                if let Ok(mut tf) = riders.get_mut(k) {
                    tf.translation = saddle_seat(Some(b));
                }
            }
        }
    }
}

/// Where a rider's feet-level origin goes to sit on a mount: on the saddle
/// if the mount's shape is known, else on the placeholder's thorax.
pub fn saddle_seat(bounds: Option<&ModelBounds>) -> Vec3 {
    match bounds {
        // The riding pose drops the hips by about a quarter of her height,
        // so her pelvis lands on the saddle when her origin sits below it.
        Some(b) => b.saddle + Vec3::new(0.0, -0.42, 0.0),
        None => Vec3::new(0.0, 0.1, -0.5),
    }
}

pub struct AnimPlugin;

impl Plugin for AnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (measure_models, seat_riders)
                .chain()
                .before(crate::rig::rig_models)
                .in_set(crate::GameSet::Simulation),
        )
        .add_systems(
            Update,
            drive_clips
                .after(crate::camera::follow)
                .in_set(crate::GameSet::Camera),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_actions() {
        assert_eq!(Action::from_name("Walking"), Some(Action::Walk));
        assert_eq!(Action::from_name("Running_01"), Some(Action::Run));
        assert_eq!(Action::from_name("Idle Breathing"), Some(Action::Idle));
        assert_eq!(Action::from_name("Sitting Idle"), Some(Action::Ride));
        assert_eq!(Action::from_name("Jump"), Some(Action::Jump));
        assert_eq!(Action::from_name("Armature|mixamo.com|Layer0"), None);
    }

    #[test]
    fn saddle_is_the_top_of_the_middle() {
        // A mosquito-ish cloud: body along z, wings out along x above it,
        // legs below.
        let mut pts = Vec::new();
        for i in 0..40 {
            let z = i as f32 / 39.0 * 6.0 - 3.0;
            pts.push(Vec3::new(0.0, 1.0, z));
            pts.push(Vec3::new(0.0, 0.2, z));
        }
        pts.push(Vec3::new(0.0, 1.4, 0.2)); // the hump of the thorax
        pts.push(Vec3::new(3.0, 1.8, 0.0)); // wing tips, higher but not centred
        pts.push(Vec3::new(-3.0, 1.8, 0.0));
        pts.push(Vec3::new(0.3, -1.2, 0.5)); // legs
        let b = measure(&pts);
        assert_eq!(b.bottom, -1.2);
        assert_eq!(b.top, 1.8);
        assert!((b.saddle.y - 1.4).abs() < 1e-5, "saddle {:?}", b.saddle);
        assert!(b.saddle.z.abs() < 0.5);
    }
}
