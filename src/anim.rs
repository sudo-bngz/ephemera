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
//!
//! **Strays.** Some animation sources bake a prop into the file with the
//! character — a horse under a riding clip, a chair under a sitting one.
//! For a character, only the skeleton that looks like a person (hips,
//! spine, arms, legs by name) and the meshes skinned to it or hung on its
//! bones are kept; anything else is hidden and left out of the measuring.
//!
//! **Seat.** A rider's hips are pinned to the saddle every frame, whatever
//! the clip or the pose does, so she sits on the creature and not above,
//! in front of or inside it.

use std::collections::HashMap;
use std::time::Duration;

use bevy::gltf::Gltf;
use bevy::math::Affine3A;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy::prelude::*;

use crate::creature::Flap;
use crate::fauna::{Beast, Critter};
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
            // No riding clip: a crouch, held still, is the nearest thing to
            // sitting astride.
            Action::Ride => &[Action::Crouch, Action::Idle],
            Action::Jump | Action::Swim => &[Action::Idle, Action::Walk],
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

/// The bone a rider sits on: pinned to the saddle while mounted. Put on the
/// figure's root; the placeholder's hips first, a model's once it loads.
#[derive(Component, Debug, Clone, Copy)]
pub struct Hips(pub Entity);

/// How much a skeleton looks like a person, from its bone names.
fn humanoid_score(names: &[String]) -> usize {
    const PARTS: [&str; 14] = [
        "hips", "pelvis", "spine", "chest", "neck", "head", "shoulder", "arm", "forearm", "hand",
        "upleg", "thigh", "leg", "foot",
    ];
    names
        .iter()
        .filter(|n| PARTS.iter().any(|p| n.contains(p)))
        .count()
}

fn bone_name(n: Option<&Name>) -> String {
    n.map(|n| {
        n.as_str()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    })
    .unwrap_or_default()
}

fn is_beast_bone(n: &str) -> bool {
    [
        "horse", "saddle", "rein", "bridle", "mane", "hoof", "chair", "stool", "seat", "prop",
    ]
    .iter()
    .any(|w| n.contains(w))
}

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
    transforms: &Query<&Transform, Without<SwappedModel>>,
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
    transforms: Query<&Transform, Without<SwappedModel>>,
    mut meshes: ResMut<Assets<Mesh>>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut placements: Query<&mut Transform, With<SwappedModel>>,
    skins: Query<&SkinnedMesh>,
    names: Query<&Name>,
) {
    for (root, swap, parent) in &swapped {
        let mut all = Vec::new();
        descendants(root, &children, &mut all);
        let mut mesh_entities: Vec<Entity> = all
            .iter()
            .copied()
            .filter(|e| meshes3d.contains(*e))
            .collect();
        if mesh_entities.is_empty() {
            continue; // the scene hasn't spawned yet
        }
        if mesh_entities
            .iter()
            .filter_map(|e| meshes3d.get(*e).ok())
            .any(|m| meshes.get(&m.0).is_none())
        {
            continue; // still loading
        }

        // A character: keep the person, hide whatever came with her.
        let humanoid = matches!(
            swap.kind,
            crate::models::ModelKind::Judith | crate::models::ModelKind::Villager
        );
        let mut hips = None;
        // Vertices to leave out of the measuring (cut away with a prop).
        let mut cut: std::collections::HashMap<Entity, Vec<bool>> = Default::default();
        if humanoid {
            let skinned: Vec<(Entity, &SkinnedMesh)> = mesh_entities
                .iter()
                .filter_map(|e| skins.get(*e).ok().map(|s| (*e, s)))
                .collect();
            let best = skinned.iter().max_by_key(|(e, s)| {
                let bones: Vec<String> = s
                    .joints
                    .iter()
                    .map(|j| bone_name(names.get(*j).ok()))
                    .collect();
                let beast = bones.iter().filter(|b| is_beast_bone(b)).count();
                let verts = meshes3d
                    .get(*e)
                    .ok()
                    .and_then(|m| meshes.get(&m.0))
                    .map(|m| m.count_vertices())
                    .unwrap_or(0);
                (humanoid_score(&bones).saturating_sub(beast * 4), verts)
            });
            if let Some((_, main)) = best {
                let joints: std::collections::HashSet<Entity> =
                    main.joints.iter().copied().collect();
                let on_skeleton = |e: Entity| {
                    let mut cur = e;
                    while let Ok(p) = parents.get(cur) {
                        if joints.contains(&p.parent()) {
                            return true;
                        }
                        if p.parent() == root {
                            break;
                        }
                        cur = p.parent();
                    }
                    false
                };
                let mut kept = Vec::new();
                for e in &mesh_entities {
                    let keep = match skins.get(*e) {
                        Ok(s) => s.joints.iter().any(|j| joints.contains(j)),
                        Err(_) => on_skeleton(*e),
                    } && !is_beast_bone(&bone_name(names.get(*e).ok()));
                    if keep {
                        kept.push(*e);
                    } else {
                        info!(
                            "{:?} model: hiding `{}`, not part of the character",
                            swap.kind,
                            names.get(*e).map(|n| n.as_str()).unwrap_or("a mesh")
                        );
                        commands.entity(*e).insert(Visibility::Hidden);
                    }
                }
                if !kept.is_empty() {
                    mesh_entities = kept;
                }
                // Her hips: of the bones named so, the one whose subtree
                // looks most like a person.
                let subtree = |j: Entity| {
                    let mut sub = vec![j];
                    descendants(j, &children, &mut sub);
                    sub
                };
                hips = main
                    .joints
                    .iter()
                    .copied()
                    .filter(|j| {
                        let n = bone_name(names.get(*j).ok());
                        (n.contains("hips") || n.contains("pelvis")) && !is_beast_bone(&n)
                    })
                    .max_by_key(|j| {
                        let names: Vec<String> = subtree(*j)
                            .iter()
                            .map(|b| bone_name(names.get(*b).ok()))
                            .collect();
                        humanoid_score(&names)
                    })
                    .or_else(|| main.joints.first().copied());

                // A prop rigged into her own skeleton (Meshy's riding clips
                // put the horse's bones beside her hips in one armature):
                // cut away every triangle weighted to bones outside the
                // subtree of her hips.
                if let Some(h) = hips {
                    let human: std::collections::HashSet<Entity> = subtree(h).into_iter().collect();
                    for e in &mesh_entities {
                        let (Ok(skin), Ok(m3)) = (skins.get(*e), meshes3d.get(*e)) else {
                            continue;
                        };
                        let foreign: Vec<bool> =
                            skin.joints.iter().map(|j| !human.contains(j)).collect();
                        if !foreign.iter().any(|f| *f) {
                            continue;
                        }
                        let Some(mesh) = meshes.get(&m3.0) else {
                            continue;
                        };
                        if let Some((culled, keep, n)) = cull_foreign(mesh, &foreign) {
                            if n > 0 {
                                info!(
                                    "{:?} model: cut {n} triangles weighted to bones that aren't hers",
                                    swap.kind
                                );
                                let handle = meshes.add(culled);
                                commands.entity(*e).insert(Mesh3d(handle));
                                cut.insert(*e, keep);
                            }
                        }
                    }
                }
            }
        }
        if let Some(h) = hips {
            commands.entity(parent.parent()).insert(Hips(h));
        }
        let handles: Vec<Handle<Mesh>> = mesh_entities
            .iter()
            .filter_map(|e| meshes3d.get(*e).ok().map(|m| m.0.clone()))
            .collect();
        if handles.iter().any(|h| meshes.get(h).is_none()) {
            continue; // still loading
        }
        // Points in the model's own frame (under its placement transform).
        let mut local = Vec::new();
        for (e, h) in mesh_entities.iter().zip(&handles) {
            let m = frame_to(*e, root, &parents, &transforms);
            if let Some(VertexAttributeValues::Float32x3(ps)) = meshes
                .get(h)
                .and_then(|mesh| mesh.attribute(Mesh::ATTRIBUTE_POSITION))
            {
                let keep = cut.get(e);
                local.extend(
                    ps.iter()
                        .enumerate()
                        .filter(|(i, _)| keep.is_none_or(|k| k.get(*i).copied().unwrap_or(true)))
                        .map(|(_, p)| m.transform_point3(Vec3::from(*p))),
                );
            }
        }
        if local.is_empty() {
            commands.entity(root).insert(Measured);
            continue;
        }
        // Auto fit: the model's box becomes the box the world asked for,
        // its bottom on the ground, its middle on the origin.
        let mut placement = placements.get(root).copied().unwrap_or_default();
        if swap.fit.auto {
            let (lo, hi) = {
                let mut lo = Vec3::splat(f32::MAX);
                let mut hi = Vec3::splat(f32::MIN);
                for p in &local {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
                (lo, hi.max(lo + Vec3::splat(1e-4)))
            };
            let yaw = Quat::from_rotation_y(swap.fit.yaw_deg.to_radians());
            // The box, seen along the model's own axes.
            let target = (yaw.inverse() * swap.size).abs().max(Vec3::splat(1e-4));
            let scale = target / (hi - lo) * swap.fit.scale;
            let centre = Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5);
            placement = Transform {
                translation: yaw * (-centre * scale) + Vec3::Y * swap.fit.y,
                rotation: yaw,
                scale,
            };
            if let Ok(mut tf) = placements.get_mut(root) {
                *tf = placement;
            }
        }
        // …and in the frame of the entity the model replaced.
        let a = placement.compute_affine();
        let points: Vec<Vec3> = local.iter().map(|p| a.transform_point3(*p)).collect();
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

/// A copy of `mesh` without the triangles whose vertices are mostly weighted
/// to `foreign` bones, which vertices were kept, and how many triangles
/// went. `None` if the mesh isn't skinned or indexed as expected.
pub fn cull_foreign(mesh: &Mesh, foreign: &[bool]) -> Option<(Mesh, Vec<bool>, usize)> {
    let joints: Vec<[u16; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)? {
        VertexAttributeValues::Uint16x4(j) => j.clone(),
        VertexAttributeValues::Uint8x4(j) => j
            .iter()
            .map(|a| [a[0] as u16, a[1] as u16, a[2] as u16, a[3] as u16])
            .collect(),
        _ => return None,
    };
    let weights: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)? {
        VertexAttributeValues::Float32x4(w) => w.clone(),
        _ => return None,
    };
    let keep: Vec<bool> = joints
        .iter()
        .zip(&weights)
        .map(|(j, w)| {
            let away: f32 = (0..4)
                .filter(|k| foreign.get(j[*k] as usize).copied().unwrap_or(false))
                .map(|k| w[k])
                .sum();
            let total: f32 = w.iter().sum::<f32>().max(1e-6);
            away / total < 0.5
        })
        .collect();
    // Unindexed meshes (flat-shaded ones are) are consecutive triples.
    let indices: Vec<u32> = match mesh.indices() {
        Some(bevy::mesh::Indices::U16(i)) => i.iter().map(|v| *v as u32).collect(),
        Some(bevy::mesh::Indices::U32(i)) => i.clone(),
        None => (0..joints.len() as u32).collect(),
    };
    let mut kept = Vec::with_capacity(indices.len());
    let mut gone = 0;
    for tri in indices.chunks_exact(3) {
        if tri
            .iter()
            .all(|v| keep.get(*v as usize).copied().unwrap_or(true))
        {
            kept.extend_from_slice(tri);
        } else {
            gone += 1;
        }
    }
    let mut out = mesh.clone();
    out.insert_indices(bevy::mesh::Indices::U32(kept));
    Some((out, keep, gone))
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
        Option<&Critter>,
        Option<&ChildOf>,
    )>,
    mounts: Query<(), With<Flap>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    for (mut clips, parent) in &mut clips {
        let Ok((gait, stance, flap, critter, grand)) = drivers.get(parent.parent()) else {
            continue;
        };
        let (wanted, speed) = if let Some(flap) = flap {
            (Action::Fly, 0.6 + 0.9 * flap.intensity)
        } else if let Some(c) = critter {
            match c.kind {
                Beast::Kite => (Action::Fly, 0.7 + 0.6 * c.effort),
                Beast::Wader if c.air > 1.0 => (Action::Fly, 1.0),
                Beast::Leaper => (Action::Swim, 0.6 + c.effort),
                _ if c.effort > 0.8 => (Action::Run, 1.0),
                _ if c.effort > 0.15 => (Action::Walk, (c.effort * 2.0).clamp(0.6, 1.4)),
                _ => (Action::Idle, 1.0),
            }
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
        // A crouch standing in for a ride is held at mid-stride.
        let held = wanted == Action::Ride && action == Action::Crouch;
        if let Some(active) = player.animation_mut(node) {
            if held {
                active.set_speed(0.0);
                active.seek_to(0.25);
            } else {
                active.set_speed(speed);
            }
        }
    }
}

/// Where a rider's hips go on a mount: just above its saddle, or on the
/// placeholder's thorax.
pub fn saddle_point(bounds: Option<&ModelBounds>) -> Vec3 {
    match bounds {
        Some(b) => b.saddle + Vec3::Y * 0.12,
        None => Vec3::new(0.0, 0.62, -0.45),
    }
}

/// Keeps every mounted rider's hips on the saddle: after the animation has
/// posed the bones and transforms have propagated, the rider is moved by
/// whatever separates her hips from the seat. Stable under any clip.
fn pin_riders(
    mut riders: Query<(&Hips, &ChildOf, &mut Transform)>,
    mounts: Query<(&GlobalTransform, Option<&ModelBounds>), With<Flap>>,
    globals: Query<&GlobalTransform>,
) {
    for (hips, parent, mut tf) in &mut riders {
        let Ok((mount, bounds)) = mounts.get(parent.parent()) else {
            continue;
        };
        let Ok(h) = globals.get(hips.0) else {
            continue;
        };
        let seat = mount.transform_point(saddle_point(bounds));
        let delta = seat - h.translation();
        if !delta.is_finite() {
            continue;
        }
        let (scale, rot, _) = mount.to_scale_rotation_translation();
        let local = rot.inverse() * delta / scale.max(Vec3::splat(1e-4));
        tf.translation += local * 0.7;
        // Sitting straight, facing where the creature goes.
        tf.rotation = Quat::IDENTITY;
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
        )
        .add_systems(
            PostUpdate,
            pin_riders.after(bevy::transform::TransformSystems::Propagate),
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
