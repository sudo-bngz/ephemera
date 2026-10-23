//! The beasts of the wetlands: what lives there besides the Ephemera.
//!
//! - **Waders**: tall marsh birds stalking the shallows on stilt legs; they
//!   take off when the player comes close and settle again further on.
//! - **Grazers**: barrel-bodied herd animals with lichen glowing on their
//!   backs; they drift about their pasture and bolt from a diving rider.
//! - **Kites**: flat gliders riding the air in loose flocks, high over the
//!   strange lands; they scatter when flown through.
//! - **Leapers**: things in the lakes that break the surface in silver arcs.
//!
//! Each is a handful of primitives with jointed limbs (`Limb`), animated
//! from the animal's own gait phase, and a `Critter` that decides where it
//! goes. Animals are chunk children in chunk-local coordinates, so they
//! stream in and out with the land they live on.

use bevy::ecs::system::RunSystemOnce;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use rand::Rng;
use rand::rngs::StdRng;

use crate::components::CameraTarget;
use crate::models::{ModelKind, Swappable};
use crate::terrain::{Props, SEA_LEVEL, Terrain};
use crate::world::{CHUNK, ChunkData, WorldTerrain};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Beast {
    Wader,
    Grazer,
    Kite,
    Leaper,
}

#[derive(Component, Debug)]
pub struct Critter {
    pub kind: Beast,
    /// Where it lives (chunk-local) and how far it strays.
    pub home: Vec3,
    pub radius: f32,
    target: Vec3,
    pause: f32,
    speed: f32,
    /// Gait phase, radians.
    pub phase: f32,
    /// How fast it is actually going (0–1 of a run), for the limbs.
    pub effort: f32,
    /// Seconds of panic left.
    pub flee: f32,
    /// Height above the ground it flies at (0 on the ground).
    pub air: f32,
    /// Flock/leap parameters.
    seed: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum LimbKind {
    /// Swings fore and aft with the gait; `side` sets the phase.
    Leg,
    /// Beats or undulates.
    Wing,
    /// Bobs with the stride.
    Head,
    /// Lashes side to side.
    Tail,
}

/// A moving part of a critter, animated relative to its rest transform.
#[derive(Component, Debug)]
pub struct Limb {
    pub kind: LimbKind,
    pub side: f32,
    pub rest: Transform,
}

#[derive(Resource)]
pub struct FaunaAssets {
    capsule: Handle<Mesh>,
    sphere: Handle<Mesh>,
    rod: Handle<Mesh>,
    cone: Handle<Mesh>,
    plate: Handle<Mesh>,
    wader: Handle<StandardMaterial>,
    grazer: Handle<StandardMaterial>,
    goat: Handle<StandardMaterial>,
    lichen: Handle<StandardMaterial>,
    kite: Handle<StandardMaterial>,
    leaper: Handle<StandardMaterial>,
    eye: Handle<StandardMaterial>,
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let plain = |m: &mut Assets<StandardMaterial>, c: Color, rough: f32| {
        m.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: rough,
            ..default()
        })
    };
    commands.insert_resource(FaunaAssets {
        capsule: meshes.add(Capsule3d::new(0.5, 1.0)),
        sphere: meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
        rod: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(6)),
        cone: meshes.add(Cone::new(1.0, 1.0).mesh().resolution(6)),
        plate: meshes.add(Cuboid::new(1.0, 0.04, 1.0)),
        wader: plain(&mut materials, Color::srgb(0.82, 0.70, 0.72), 0.8),
        grazer: plain(&mut materials, Color::srgb(0.22, 0.26, 0.18), 0.95),
        goat: plain(&mut materials, Color::srgb(0.58, 0.56, 0.52), 0.95),
        lichen: materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 1.0, 0.6),
            emissive: LinearRgba::rgb(0.8, 3.0, 1.2),
            ..default()
        }),
        kite: materials.add(StandardMaterial {
            base_color: Color::srgba(0.55, 0.40, 0.80, 0.85),
            emissive: LinearRgba::rgb(0.5, 0.2, 1.2),
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            perceptual_roughness: 0.4,
            ..default()
        }),
        leaper: materials.add(StandardMaterial {
            base_color: Color::srgb(0.70, 0.82, 0.90),
            metallic: 0.6,
            perceptual_roughness: 0.2,
            ..default()
        }),
        eye: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.9, 0.4),
            emissive: LinearRgba::rgb(5.0, 4.0, 1.0),
            ..default()
        }),
    });
}

fn critter(kind: Beast, home: Vec3, radius: f32, speed: f32, rng: &mut StdRng) -> Critter {
    Critter {
        kind,
        home,
        radius,
        target: home,
        pause: rng.gen_range(0.0..3.0),
        speed,
        phase: rng.gen_range(0.0..std::f32::consts::TAU),
        effort: 0.0,
        flee: 0.0,
        air: 0.0,
        seed: rng.gen_range(0.0..100.0),
    }
}

fn part(
    p: &mut ChildSpawnerCommands,
    mesh: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
    t: Transform,
    limb: Option<Limb>,
) {
    match limb {
        Some(l) => {
            p.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), t, l));
        }
        None => {
            p.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), t));
        }
    }
}

fn limb(kind: LimbKind, side: f32, rest: Transform) -> Option<Limb> {
    Some(Limb { kind, side, rest })
}

fn spawn_wader(commands: &mut Commands, fa: &FaunaAssets, at: Vec3, rng: &mut StdRng) -> Entity {
    let size = rng.gen_range(0.9..1.3);
    let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
    commands
        .spawn((
            critter(Beast::Wader, at, 18.0, 1.4, rng),
            Swappable::sized(ModelKind::Wader, Vec3::new(1.6, 3.7, 2.2)).variant(rng.r#gen()),
            Name::new("Wader"),
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::splat(size)),
            Visibility::default(),
        ))
        .with_children(|p| {
            // Body 2.3 m up, legs beneath, neck and head forward (−Z).
            part(
                p,
                &fa.capsule,
                &fa.wader,
                Transform::from_xyz(0.0, 2.3, 0.0)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::new(0.55, 0.7, 0.5)),
                None,
            );
            for side in [-1.0, 1.0] {
                let rest = Transform::from_xyz(side * 0.22, 2.0, 0.0);
                part(p, &fa.rod, &fa.wader, rest, limb(LimbKind::Leg, side, rest));
            }
            let neck = Transform::from_xyz(0.0, 2.9, -0.6)
                .with_rotation(Quat::from_rotation_x(0.6))
                .with_scale(Vec3::new(0.08, 1.4, 0.08));
            part(p, &fa.rod, &fa.wader, neck, limb(LimbKind::Head, 0.0, neck));
            let head = Transform::from_xyz(0.0, 3.55, -1.05).with_scale(Vec3::splat(0.2));
            part(
                p,
                &fa.sphere,
                &fa.wader,
                head,
                limb(LimbKind::Head, 1.0, head),
            );
            let beak = Transform::from_xyz(0.0, 3.5, -1.5)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::new(0.06, 0.7, 0.06));
            part(
                p,
                &fa.cone,
                &fa.wader,
                beak,
                limb(LimbKind::Head, 2.0, beak),
            );
            part(
                p,
                &fa.sphere,
                &fa.eye,
                Transform::from_xyz(0.0, 3.6, -1.2).with_scale(Vec3::splat(0.07)),
                limb(
                    LimbKind::Head,
                    3.0,
                    Transform::from_xyz(0.0, 3.6, -1.2).with_scale(Vec3::splat(0.07)),
                ),
            );
            // Wings folded along the body; they open in flight.
            for side in [-1.0, 1.0] {
                let rest = Transform::from_xyz(side * 0.3, 2.55, 0.1)
                    .with_rotation(Quat::from_rotation_z(side * 1.4))
                    .with_scale(Vec3::new(1.6, 1.0, 0.9));
                part(
                    p,
                    &fa.plate,
                    &fa.wader,
                    rest,
                    limb(LimbKind::Wing, side, rest),
                );
            }
        })
        .id()
}

fn spawn_grazer(
    commands: &mut Commands,
    fa: &FaunaAssets,
    at: Vec3,
    herd: Vec3,
    mountain: bool,
    rng: &mut StdRng,
) -> Entity {
    let size = if mountain {
        rng.gen_range(0.6..0.9)
    } else {
        rng.gen_range(1.1..1.7)
    };
    let hide = if mountain { &fa.goat } else { &fa.grazer };
    let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
    commands
        .spawn((
            critter(Beast::Grazer, herd, 16.0, 0.9, rng),
            Swappable::sized(
                if mountain {
                    ModelKind::Goat
                } else {
                    ModelKind::Grazer
                },
                Vec3::new(1.8, 2.1, 3.5),
            )
            .variant(rng.r#gen()),
            Name::new(if mountain { "Goat" } else { "Grazer" }),
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::splat(size)),
            Visibility::default(),
        ))
        .with_children(|p| {
            part(
                p,
                &fa.sphere,
                hide,
                Transform::from_xyz(0.0, 1.45, 0.0).with_scale(Vec3::new(0.8, 0.7, 1.3)),
                None,
            );
            for (i, z) in [-0.7_f32, 0.75].into_iter().enumerate() {
                for side in [-1.0, 1.0] {
                    let rest = Transform::from_xyz(side * 0.45, 0.85, z)
                        .with_scale(Vec3::new(0.14, 1.1, 0.14));
                    part(
                        p,
                        &fa.rod,
                        hide,
                        rest,
                        limb(LimbKind::Leg, side * if i == 0 { 1.0 } else { -1.0 }, rest),
                    );
                }
            }
            let head = Transform::from_xyz(0.0, 1.35, -1.45).with_scale(Vec3::new(0.32, 0.3, 0.5));
            part(p, &fa.sphere, hide, head, limb(LimbKind::Head, 0.0, head));
            let tail = Transform::from_xyz(0.0, 1.6, 1.3)
                .with_rotation(Quat::from_rotation_x(0.9))
                .with_scale(Vec3::new(0.05, 0.7, 0.05));
            part(p, &fa.rod, hide, tail, limb(LimbKind::Tail, 0.0, tail));
            if !mountain {
                for k in 0..4 {
                    let a = k as f32 * 1.7;
                    part(
                        p,
                        &fa.sphere,
                        &fa.lichen,
                        Transform::from_xyz(
                            a.cos() * 0.35,
                            1.95 + 0.05 * (a * 3.0).sin(),
                            a.sin() * 0.6,
                        )
                        .with_scale(Vec3::new(0.18, 0.1, 0.18)),
                        None,
                    );
                }
            } else {
                // Horns.
                for side in [-1.0, 1.0] {
                    part(
                        p,
                        &fa.cone,
                        &fa.goat,
                        Transform::from_xyz(side * 0.15, 1.65, -1.4)
                            .with_rotation(
                                Quat::from_rotation_x(-0.5) * Quat::from_rotation_z(-side * 0.4),
                            )
                            .with_scale(Vec3::new(0.05, 0.5, 0.05)),
                        None,
                    );
                }
            }
        })
        .id()
}

fn spawn_kite(
    commands: &mut Commands,
    fa: &FaunaAssets,
    centre: Vec3,
    air: f32,
    rng: &mut StdRng,
) -> Entity {
    let size = rng.gen_range(0.8..1.5);
    let mut c = critter(
        Beast::Kite,
        centre,
        rng.gen_range(10.0..26.0),
        rng.gen_range(5.0..9.0),
        rng,
    );
    c.air = air + rng.gen_range(-4.0..6.0);
    commands
        .spawn((
            c,
            Swappable::sized(ModelKind::Kite, Vec3::new(6.0, 0.6, 4.6)).variant(rng.r#gen()),
            Name::new("Kite"),
            Transform::from_translation(centre).with_scale(Vec3::splat(size)),
            Visibility::default(),
        ))
        .with_children(|p| {
            part(
                p,
                &fa.sphere,
                &fa.kite,
                Transform::from_scale(Vec3::new(0.6, 0.18, 1.2)),
                None,
            );
            for side in [-1.0, 1.0] {
                let rest =
                    Transform::from_xyz(side * 1.5, 0.0, 0.0).with_scale(Vec3::new(2.6, 1.0, 1.7));
                part(
                    p,
                    &fa.plate,
                    &fa.kite,
                    rest,
                    limb(LimbKind::Wing, side, rest),
                );
            }
            let tail = Transform::from_xyz(0.0, 0.0, 2.0)
                .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::new(0.05, 2.4, 0.05));
            part(p, &fa.rod, &fa.kite, tail, limb(LimbKind::Tail, 0.0, tail));
            part(
                p,
                &fa.sphere,
                &fa.eye,
                Transform::from_xyz(0.0, 0.1, -1.0).with_scale(Vec3::splat(0.12)),
                None,
            );
        })
        .id()
}

fn spawn_leaper(commands: &mut Commands, fa: &FaunaAssets, at: Vec3, rng: &mut StdRng) -> Entity {
    let size = rng.gen_range(0.6..1.4);
    commands
        .spawn((
            critter(Beast::Leaper, at, 12.0, 6.0, rng),
            Swappable::sized(ModelKind::Leaper, Vec3::new(0.7, 0.7, 2.3)).variant(rng.r#gen()),
            Name::new("Leaper"),
            Transform::from_translation(at.with_y(SEA_LEVEL - 2.0)).with_scale(Vec3::splat(size)),
            Visibility::default(),
            NotShadowCaster,
        ))
        .with_children(|p| {
            part(
                p,
                &fa.capsule,
                &fa.leaper,
                Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::new(0.45, 1.3, 0.35)),
                None,
            );
            let tail = Transform::from_xyz(0.0, 0.0, 1.3).with_scale(Vec3::new(0.05, 0.6, 0.5));
            part(
                p,
                &fa.plate,
                &fa.leaper,
                tail,
                limb(LimbKind::Tail, 0.0, tail),
            );
        })
        .id()
}

/// Populates one chunk: who lives where depends on the land.
pub fn spawn_in_chunk(
    commands: &mut Commands,
    fa: &FaunaAssets,
    t: &Terrain,
    data: &ChunkData,
    coord: IVec2,
    biome: Props,
    rng: &mut StdRng,
    kids: &mut Vec<Entity>,
) {
    let origin = coord.as_vec2() * CHUNK;
    let spot = |rng: &mut StdRng| {
        let l = Vec2::new(
            rng.gen_range(6.0..CHUNK - 6.0),
            rng.gen_range(6.0..CHUNK - 6.0),
        );
        (l, origin + l, data.height_at(l.x, l.y))
    };
    let open = |w: Vec2| t.sites_near(w, 0.0).is_empty() && t.river(w.x, w.y).0 < 0.3;
    let local = |l: Vec2, y: f32| Vec3::new(l.x, y, l.y);

    // Leapers in every lake.
    if data.min_height() < SEA_LEVEL - 1.5 {
        for _ in 0..rng.gen_range(1..4) {
            let (l, _, h) = spot(rng);
            if h < SEA_LEVEL - 1.5 {
                kids.push(spawn_leaper(commands, fa, local(l, SEA_LEVEL), rng));
            }
        }
    }
    // Waders in the shallows of the wet lands.
    if matches!(
        biome,
        Props::Marsh | Props::Grove | Props::Monoliths | Props::None
    ) {
        for _ in 0..rng.gen_range(0..4) {
            let (l, w, h) = spot(rng);
            if (SEA_LEVEL - 1.0..SEA_LEVEL + 0.4).contains(&h) && open(w) {
                kids.push(spawn_wader(
                    commands,
                    fa,
                    local(l, h.max(SEA_LEVEL - 0.6)),
                    rng,
                ));
            }
        }
    }
    // Herds on dry ground.
    let herd_odds = match biome {
        Props::Grove => 0.55,
        Props::Bones | Props::None | Props::Marsh => 0.2,
        Props::Peaks => 0.35,
        _ => 0.0,
    };
    if rng.gen_bool(herd_odds) {
        let (l, w, h) = spot(rng);
        if h > SEA_LEVEL + 0.5 && open(w) {
            let herd = local(l, h);
            for _ in 0..rng.gen_range(3..7) {
                let off = Vec2::new(rng.gen_range(-9.0..9.0), rng.gen_range(-9.0..9.0));
                let lp = (l + off).clamp(Vec2::splat(2.0), Vec2::splat(CHUNK - 2.0));
                let hh = data.height_at(lp.x, lp.y);
                if hh > SEA_LEVEL {
                    kids.push(spawn_grazer(
                        commands,
                        fa,
                        local(lp, hh),
                        herd,
                        biome == Props::Peaks,
                        rng,
                    ));
                }
            }
        }
    }
    // Kites over the high and the strange lands; a lone flock anywhere.
    let kite_odds = match biome {
        Props::Peaks => 0.5,
        Props::Crystal | Props::Fungal | Props::Monoliths => 0.3,
        _ => 0.08,
    };
    if rng.gen_bool(kite_odds) {
        let (l, _, h) = spot(rng);
        let air = rng.gen_range(18.0..38.0);
        let centre = local(l, h.max(SEA_LEVEL) + air);
        for _ in 0..rng.gen_range(3..8) {
            kids.push(spawn_kite(commands, fa, centre, air, rng));
        }
    }
}

/// Where the animals go. Chunk-local positions; the ground is asked of the
/// terrain in world space (the chunk root sits at y = 0, so heights agree).
fn roam(
    time: Res<Time>,
    terrain: Option<Res<WorldTerrain>>,
    player: Query<&GlobalTransform, With<CameraTarget>>,
    mut q: Query<(&mut Critter, &mut Transform, &GlobalTransform)>,
) {
    let Some(terrain) = terrain else {
        return;
    };
    let dt = time.delta_secs().min(0.05);
    if dt <= 0.0 {
        return;
    }
    let t = time.elapsed_secs();
    let player = player.iter().next().map(|g| g.translation());
    for (mut c, mut tf, gt) in &mut q {
        let world = gt.translation();
        let origin = world - tf.translation;
        let p = tf.translation;
        // Fear: anything the camera follows, closer than it likes.
        if let Some(pp) = player {
            let scare = match c.kind {
                Beast::Wader => 22.0,
                Beast::Grazer => 18.0,
                Beast::Kite => 12.0,
                Beast::Leaper => 0.0,
            };
            if scare > 0.0 && (pp - world).length() < scare && c.flee <= 0.0 {
                c.flee = match c.kind {
                    Beast::Wader => 7.0,
                    Beast::Grazer => 4.5,
                    _ => 3.0,
                };
                let away = (world - pp).with_y(0.0).normalize_or(Vec3::X);
                c.target = p + away * 40.0;
                if c.kind == Beast::Wader {
                    c.home = c.target;
                }
            }
        }
        let fleeing = c.flee > 0.0;
        c.flee = (c.flee - dt).max(0.0);

        match c.kind {
            Beast::Wader | Beast::Grazer => {
                let to = (c.target - p).with_y(0.0);
                let arrived = to.length() < 0.5;
                if arrived && !fleeing {
                    c.pause -= dt;
                    if c.pause <= 0.0 {
                        let a = (t * 0.9 + c.seed).sin() * std::f32::consts::PI;
                        let d = c.radius * ((t * 0.37 + c.seed * 0.3).cos() * 0.5 + 0.5);
                        c.target = c.home + Vec3::new(a.cos() * d, 0.0, a.sin() * d);
                        c.pause = 2.0 + 6.0 * ((t + c.seed).sin() * 0.5 + 0.5);
                    }
                }
                let speed = if fleeing {
                    if c.kind == Beast::Grazer { 9.0 } else { 13.0 }
                } else {
                    c.speed
                };
                let mut moving = 0.0;
                if !arrived {
                    let dir = to.normalize();
                    let step = (speed * dt).min(to.length());
                    tf.translation += dir * step;
                    let yaw = (-dir.x).atan2(-dir.z);
                    tf.rotation = tf
                        .rotation
                        .slerp(Quat::from_rotation_y(yaw), 1.0 - (-5.0 * dt).exp());
                    moving = speed / 9.0;
                }
                // Waders fly when they flee; everyone else keeps to the ground.
                let ground = terrain
                    .0
                    .height(origin.x + tf.translation.x, origin.z + tf.translation.z);
                let wanted_air = if c.kind == Beast::Wader && fleeing && c.flee > 1.5 {
                    12.0
                } else {
                    0.0
                };
                c.air += (wanted_air - c.air) * (1.0 - (-2.5 * dt).exp());
                let floor = if c.kind == Beast::Wader {
                    ground.max(SEA_LEVEL - 0.8)
                } else {
                    ground.max(SEA_LEVEL - 0.3)
                };
                tf.translation.y = floor + c.air;
                c.effort += (moving.min(1.5) - c.effort) * (1.0 - (-6.0 * dt).exp());
                c.phase += dt * (1.5 + 7.0 * c.effort);
            }
            Beast::Kite => {
                // Circling a centre that drifts, banking into the turn.
                let r = c.radius * if fleeing { 1.8 } else { 1.0 };
                let w = c.speed / r * if fleeing { 2.0 } else { 1.0 };
                c.phase += dt * w;
                let drift = Vec3::new(
                    (t * 0.05 + c.seed).sin(),
                    0.0,
                    (t * 0.04 + c.seed * 0.7).cos(),
                ) * 25.0;
                let centre = c.home + drift;
                let a = c.phase + c.seed;
                let ground = terrain
                    .0
                    .height(
                        origin.x + centre.x + a.cos() * r,
                        origin.z + centre.z + a.sin() * r,
                    )
                    .max(SEA_LEVEL);
                let y = ground + c.air + 3.0 * (t * 0.3 + c.seed).sin();
                let next = Vec3::new(centre.x + a.cos() * r, y, centre.z + a.sin() * r);
                let vel = (next - p) / dt;
                tf.translation = next;
                if vel.length() > 0.1 {
                    let yaw = (-vel.x).atan2(-vel.z);
                    let pitch = (vel.y / vel.length()).clamp(-0.6, 0.6);
                    tf.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, -0.45 * w.signum());
                }
                c.effort = if fleeing { 1.0 } else { 0.3 };
            }
            Beast::Leaper => {
                // Rest under the surface, then an arc into the air.
                c.pause -= dt;
                if c.pause <= 0.0 {
                    // Leap: 1.3 s, 6 m forward, 3 m high, then a long wait.
                    let a = (t * 0.7 + c.seed).sin() * std::f32::consts::PI;
                    c.target = Vec3::new(a.cos(), 0.0, a.sin());
                    c.phase = 0.0;
                    c.pause = 5.0 + 9.0 * ((t + c.seed).cos() * 0.5 + 0.5);
                    c.flee = 1.3;
                    // Drift the start so it doesn't always leap in one spot.
                    let d = c.radius * ((t * 0.5 + c.seed).sin() * 0.5 + 0.5);
                    tf.translation =
                        c.home.with_y(SEA_LEVEL - 2.0) + Vec3::new(a.sin() * d, 0.0, a.cos() * d);
                }
                if c.flee > 0.0 {
                    let u = 1.0 - c.flee / 1.3;
                    let height = 3.0 * tf.scale.x * 4.0 * u * (1.0 - u) - 1.2;
                    let dir = c.target;
                    tf.translation += dir * 6.0 * dt;
                    tf.translation.y = SEA_LEVEL + height;
                    let vy = 3.0 * tf.scale.x * 4.0 * (1.0 - 2.0 * u);
                    let yaw = (-dir.x).atan2(-dir.z);
                    tf.rotation =
                        Quat::from_rotation_y(yaw) * Quat::from_rotation_x((vy / 6.0).atan());
                    c.effort = 1.0;
                } else {
                    tf.translation.y = SEA_LEVEL - 2.0;
                    c.effort = 0.0;
                }
                c.phase += dt * 6.0;
            }
        }
    }
}

/// Limbs follow their critter's gait.
fn animate_limbs(
    time: Res<Time>,
    critters: Query<&Critter>,
    mut limbs: Query<(&Limb, &ChildOf, &mut Transform)>,
) {
    let t = time.elapsed_secs();
    for (limb, parent, mut tf) in &mut limbs {
        let Ok(c) = critters.get(parent.parent()) else {
            continue;
        };
        let ph = c.phase;
        *tf = limb.rest;
        match (c.kind, limb.kind) {
            (Beast::Wader, LimbKind::Leg) => {
                // Pivot at the hip (top of the rod): swing, lifting the knee.
                let swing = (ph
                    + if limb.side > 0.0 {
                        0.0
                    } else {
                        std::f32::consts::PI
                    })
                .sin()
                    * (0.15 + 0.5 * c.effort.min(1.0));
                let tuck = c.air / 12.0;
                let len = 2.0;
                tf.rotation = Quat::from_rotation_x(swing + 1.2 * tuck);
                tf.scale = Vec3::new(0.06, len, 0.06);
                tf.translation =
                    limb.rest.translation + tf.rotation * Vec3::new(0.0, -len * 0.5, 0.0);
            }
            (Beast::Wader, LimbKind::Wing) => {
                // Folded on the ground, spread and beating in the air.
                let open = (c.air / 12.0).clamp(0.0, 1.0);
                let beat = (t * 9.0).sin() * 0.5 * open;
                let fold = limb.side * (1.4 * (1.0 - open) + beat);
                tf.rotation = Quat::from_rotation_z(fold);
                tf.translation =
                    limb.rest.translation + tf.rotation * Vec3::new(limb.side * 0.8, 0.0, 0.0);
            }
            (Beast::Wader, LimbKind::Head) => {
                // The whole neck-and-head group bobs forward with each stride.
                let bob =
                    (ph * 2.0).sin() * 0.12 * c.effort.min(1.0) + (t * 0.8 + c.seed).sin() * 0.03;
                tf.translation.z += bob;
                tf.translation.y -= bob * 0.5;
            }
            (Beast::Grazer, LimbKind::Leg) => {
                let swing = (ph
                    + if limb.side > 0.0 {
                        0.0
                    } else {
                        std::f32::consts::PI
                    })
                .sin()
                    * (0.1 + 0.45 * c.effort.min(1.0));
                tf.rotation = Quat::from_rotation_x(swing);
                tf.translation = limb.rest.translation
                    + Vec3::Y * 0.55
                    + tf.rotation * Vec3::new(0.0, -0.55, 0.0);
            }
            (Beast::Grazer, LimbKind::Head) => {
                // Grazing: head down between strides, up to look when moving.
                let graze = (1.0 - c.effort.min(1.0)) * (0.35 + 0.25 * (t * 0.6 + c.seed).sin());
                tf.translation.y -= graze;
                tf.translation.z += 0.1 * (ph * 2.0).sin() * c.effort;
            }
            (Beast::Grazer, LimbKind::Tail) => {
                tf.rotation =
                    limb.rest.rotation * Quat::from_rotation_z((t * 3.0 + c.seed).sin() * 0.5);
            }
            (Beast::Kite, LimbKind::Wing) => {
                // Slow undulation, the tips trailing the body.
                let w = (t * 2.2 + c.seed + limb.side * 0.3).sin() * (0.25 + 0.3 * c.effort);
                tf.rotation = Quat::from_rotation_z(limb.side * w);
                tf.translation = Vec3::ZERO + tf.rotation * limb.rest.translation;
            }
            (Beast::Kite, LimbKind::Tail) => {
                tf.rotation =
                    limb.rest.rotation * Quat::from_rotation_z((t * 1.7 + c.seed).sin() * 0.3);
            }
            (Beast::Leaper, LimbKind::Tail) => {
                tf.rotation = Quat::from_rotation_y((ph * 2.5).sin() * 0.6 * c.effort.max(0.2));
            }
            _ => {}
        }
    }
}

pub struct FaunaPlugin;

impl Plugin for FaunaPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .run_system_once(setup_assets)
            .expect("fauna assets need Assets<Mesh> and Assets<StandardMaterial>");
        app.add_systems(
            Update,
            (roam, animate_limbs)
                .chain()
                .in_set(crate::GameSet::Simulation),
        );
    }
}
