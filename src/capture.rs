//! Phase 1 — stealth capture.
//!
//! A wild Ephemera rests on a lily pad somewhere in the dusk marsh. Judith has
//! to get within net range without its *awareness* reaching the top: sound
//! (sprinting, splashing through pools) and being seen (standing in the open)
//! both feed it, while crouching in the reeds lets it decay. Push it too far
//! and the creature bolts to a new resting place.

use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::camera::ChaseCamera;
use crate::components::{CameraTarget, Ephemera, Judith, PhaseScoped, Velocity, spawn_judith};
use crate::creature::{Coat, Flap, spawn_ephemera};
use crate::hud::Hud;
use crate::input::InputAxes;
use crate::state::{EphemeraStats, GamePhase, RunLog};
use crate::world::{Mood, Pool, ReedClump, WetlandAssets, WetlandsConfig, spawn_wetlands};

const NET_RANGE: f32 = 6.5;
const SENSE_RANGE: f32 = 38.0;
const CATCH_THRESHOLD: f32 = 0.72;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Stalking,
    /// Net is in the air; timer until resolution.
    Netting(f32),
    Caught(f32),
    Spooked(f32),
}

#[derive(Resource)]
struct Capture {
    stage: Stage,
    awareness: f32,
    /// Diagnostics for the HUD.
    noise: f32,
    visibility: f32,
    distance: f32,
    in_cover: bool,
    rng: StdRng,
    /// Outcome text for the banner.
    message: String,
}

#[derive(Component)]
struct Wild {
    /// Where it is heading when fleeing.
    target: Vec3,
    fleeing: bool,
    /// Resting altitude.
    rest_y: f32,
}

#[derive(Component)]
struct Net {
    from: Vec3,
    to: Vec3,
    t: f32,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<WetlandAssets>,
    mut hud: ResMut<Hud>,
    mut log: ResMut<RunLog>,
) {
    spawn_wetlands(
        &mut commands,
        &mut meshes,
        &mut materials,
        &assets,
        WetlandsConfig {
            seed: 1984,
            half_size: 110.0,
            reed_clumps: 170,
            pools: 26,
            spores: 120,
            mood: Mood::Dusk,
            clearing: 0.0,
        },
        &[],
    );

    let judith = spawn_judith(
        &mut commands,
        &mut meshes,
        &mut materials,
        Vec3::new(-60.0, 0.0, 55.0),
    );
    commands.entity(judith).insert(CameraTarget::ON_FOOT);

    let nest = Vec3::new(38.0, 2.0, -40.0);
    commands.spawn((
        PhaseScoped,
        Mesh3d(assets.pad_mesh.clone()),
        MeshMaterial3d(assets.pad_mat.clone()),
        Transform::from_xyz(nest.x, 0.05, nest.z)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(5.0)),
    ));
    spawn_ephemera(
        &mut commands,
        &mut meshes,
        &mut materials,
        nest,
        Coat::WILD,
        0.05,
        (
            PhaseScoped,
            Wild {
                target: nest,
                fleeing: false,
                rest_y: nest.y,
            },
        ),
    );

    commands.insert_resource(Capture {
        stage: Stage::Stalking,
        awareness: 0.15,
        noise: 0.0,
        visibility: 1.0,
        distance: 0.0,
        in_cover: false,
        rng: StdRng::seed_from_u64(42),
        message: String::new(),
    });
    log.capture_attempts += 1;

    *hud = Hud {
        title: "I. THE STALK".into(),
        body: "Dusk. A wild Ephemera rests on a lily pad to the north-east.\n\
               Crouch through the reeds. Avoid the pools — they splash.\n\
               Get within reach and throw the net while it is still calm."
            .into(),
        banner: String::new(),
        footer: "W/S walk · A/D turn · C crouch · SHIFT sprint · SPACE throw net (close range)"
            .into(),
        side: String::new(),
    };
}

/// The creature panics: pick a new resting place well away from Judith and
/// start the flee flight. Shared by the awareness overflow and a failed net.
fn spook(cap: &mut Capture, wild: &mut Wild, judith_pos: Vec3, log: &mut RunLog, message: &str) {
    let a = cap.rng.gen_range(0.0..std::f32::consts::TAU);
    let r = cap.rng.gen_range(45.0..75.0);
    let mut target = judith_pos + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
    target.x = target.x.clamp(-100.0, 100.0);
    target.z = target.z.clamp(-100.0, 100.0);
    target.y = wild.rest_y;
    wild.target = target;
    wild.fleeing = true;
    cap.stage = Stage::Spooked(2.6);
    cap.awareness = 1.0;
    cap.message = message.into();
    log.creature_spooked += 1;
}

fn teardown(mut commands: Commands) {
    commands.remove_resource::<Capture>();
}

fn move_judith(
    time: Res<Time>,
    axes: Res<InputAxes>,
    cap: Res<Capture>,
    mut q: Query<(&mut Transform, &mut Velocity), With<Judith>>,
) {
    let Ok((mut tf, mut vel)) = q.single_mut() else {
        return;
    };
    if !matches!(cap.stage, Stage::Stalking) {
        vel.0 = Vec3::ZERO;
        return;
    }
    let dt = time.delta_secs();
    let turn_rate = if axes.crouch { 2.2 } else { 3.0 };
    tf.rotate_y(-axes.steer * turn_rate * dt);

    let speed = if axes.crouch {
        3.2
    } else if axes.boost {
        11.0
    } else {
        6.5
    };
    let forward = tf.forward().as_vec3();
    let wanted = forward * axes.throttle * speed;
    // Snappy but not instant: a short acceleration ramp.
    vel.0 = vel.0.lerp(wanted, 1.0 - (-14.0 * dt).exp());
    tf.translation += vel.0 * dt;
    tf.translation.x = tf.translation.x.clamp(-125.0, 125.0);
    tf.translation.z = tf.translation.z.clamp(-125.0, 125.0);
    tf.translation.y = 0.0;
}

fn sense(
    time: Res<Time>,
    axes: Res<InputAxes>,
    mut cap: ResMut<Capture>,
    judith: Query<(&Transform, &Velocity), With<Judith>>,
    mut creature: Query<(&Transform, &mut Flap, &mut Wild), With<Ephemera>>,
    reeds: Query<(&GlobalTransform, &ReedClump)>,
    pools: Query<(&GlobalTransform, &Pool)>,
    mut log: ResMut<RunLog>,
) {
    let Ok((jtf, jvel)) = judith.single() else {
        return;
    };
    let Ok((etf, mut flap, mut wild)) = creature.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    let jp = jtf.translation;

    // Cover & terrain.
    let in_cover = reeds
        .iter()
        .any(|(t, r)| (t.translation() - jp).xz().length() < r.radius);
    let in_pool = pools
        .iter()
        .any(|(t, p)| (t.translation() - jp).xz().length() < p.radius);

    let moving = jvel.0.length() > 0.3;
    let mut noise = match (moving, axes.crouch, axes.boost) {
        (false, _, _) => 0.0,
        (true, true, _) => 0.12,
        (true, false, true) => 1.0,
        (true, false, false) => 0.5,
    };
    if in_pool && moving {
        noise *= 2.4;
    }
    let mut visibility = if in_cover { 0.3 } else { 1.0 };
    if axes.crouch {
        visibility *= if in_cover { 0.25 } else { 0.5 };
    }

    let distance = (etf.translation - jp).length();
    let proximity = (1.0 - distance / SENSE_RANGE).clamp(0.0, 1.0).powf(1.4);

    if matches!(cap.stage, Stage::Stalking) && !wild.fleeing {
        // Awareness always bleeds off at a fixed rate; sound and sight push
        // against it. Still and hidden in the reeds, the creature calms down.
        // Standing in the open at close range, it never does.
        let gain = (noise * 1.1 + visibility * 0.35) * proximity;
        cap.awareness = (cap.awareness + (gain - 0.12) * dt).clamp(0.0, 1.0);

        if cap.awareness >= 1.0 {
            spook(
                &mut cap,
                &mut wild,
                jp,
                &mut log,
                "SPOOKED — it bolts into the dusk",
            );
        }
    }

    // Feedback: the creature's wings tremble as it grows nervous.
    flap.intensity = if wild.fleeing {
        0.9
    } else {
        0.05 + 0.45 * cap.awareness
    };

    cap.noise = noise;
    cap.visibility = visibility;
    cap.distance = distance;
    cap.in_cover = in_cover;
}

fn throw_net(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    axes: Res<InputAxes>,
    mut cap: ResMut<Capture>,
    judith: Query<&Transform, With<Judith>>,
    creature: Query<&Transform, With<Ephemera>>,
) {
    if !matches!(cap.stage, Stage::Stalking) || !axes.action_pressed {
        return;
    }
    let (Ok(jtf), Ok(etf)) = (judith.single(), creature.single()) else {
        return;
    };
    if cap.distance > NET_RANGE {
        cap.message = format!("Too far — {:.0} m", cap.distance);
        return;
    }
    cap.stage = Stage::Netting(0.75);
    cap.message = String::new();
    commands.spawn((
        PhaseScoped,
        Net {
            from: jtf.translation + Vec3::Y * 1.6,
            to: etf.translation,
            t: 0.0,
        },
        Mesh3d(meshes.add(Sphere::new(1.6).mesh().ico(2).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.9, 0.85, 0.6, 0.35),
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::from_translation(jtf.translation + Vec3::Y * 1.6),
    ));
}

fn fly_net(time: Res<Time>, mut q: Query<(&mut Net, &mut Transform)>) {
    for (mut net, mut tf) in &mut q {
        net.t = (net.t + time.delta_secs() / 0.6).min(1.0);
        let arc = Vec3::Y * (net.t * (1.0 - net.t)) * 6.0;
        tf.translation = net.from.lerp(net.to, net.t) + arc;
        tf.scale = Vec3::splat(0.6 + net.t * 0.8);
    }
}

fn resolve(
    time: Res<Time>,
    mut cap: ResMut<Capture>,
    mut next: ResMut<NextState<GamePhase>>,
    mut log: ResMut<RunLog>,
    judith: Query<&Transform, With<Judith>>,
    mut creature: Query<(&mut Transform, &mut Wild), (With<Ephemera>, Without<Judith>)>,
    mut cams: Query<&mut ChaseCamera>,
    mut stats: ResMut<EphemeraStats>,
) {
    let dt = time.delta_secs();
    let Ok((mut etf, mut wild)) = creature.single_mut() else {
        return;
    };

    // Flee flight: rise, then glide to the target.
    if wild.fleeing {
        let to = wild.target - etf.translation;
        let dist = to.xz().length();
        if dist < 1.0 {
            wild.fleeing = false;
            etf.translation = wild.target;
        } else {
            let dir = to.normalize_or_zero();
            let speed = 26.0;
            etf.translation += dir * speed * dt;
            etf.translation.y = wild.rest_y + (dist / 60.0).min(1.0) * 10.0;
            let yaw = (-dir.x).atan2(-dir.z);
            etf.rotation = etf
                .rotation
                .slerp(Quat::from_rotation_y(yaw), 1.0 - (-5.0 * dt).exp());
        }
    } else {
        // Idle: slowly turn to face Judith when nervous.
        if let Ok(jtf) = judith.single() {
            let to = (jtf.translation - etf.translation).xz();
            if cap.awareness > 0.5 && to.length() > 0.1 {
                let yaw = (-to.x).atan2(-to.y);
                etf.rotation = etf
                    .rotation
                    .slerp(Quat::from_rotation_y(yaw), 1.0 - (-0.8 * dt).exp());
            }
        }
    }

    match cap.stage {
        Stage::Stalking => {}
        Stage::Netting(t) => {
            let t = t - dt;
            if t <= 0.0 {
                if cap.awareness <= CATCH_THRESHOLD {
                    cap.stage = Stage::Caught(2.5);
                    cap.message = "CAUGHT!".into();
                    // A calm catch starts the bond; a panicked one hurts it.
                    stats.trust = 0.25 - cap.awareness * 0.2;
                    stats.clamp();
                    if let Ok(mut c) = cams.single_mut() {
                        c.kick(0.6);
                    }
                } else {
                    let jp = judith.single().map(|t| t.translation).unwrap_or(Vec3::ZERO);
                    spook(
                        &mut cap,
                        &mut wild,
                        jp,
                        &mut log,
                        "It saw the net coming — SPOOKED",
                    );
                }
            } else {
                cap.stage = Stage::Netting(t);
            }
        }
        Stage::Caught(t) => {
            let t = t - dt;
            if t <= 0.0 {
                next.set(GamePhase::Taming);
            } else {
                cap.stage = Stage::Caught(t);
            }
        }
        Stage::Spooked(t) => {
            let t = t - dt;
            if t <= 0.0 {
                cap.stage = Stage::Stalking;
                cap.awareness = 0.25;
                cap.message = String::new();
            } else {
                cap.stage = Stage::Spooked(t);
            }
        }
    }
}

fn hud(cap: Res<Capture>, mut hud: ResMut<Hud>) {
    let bar = |v: f32, w: usize| -> String {
        let n = ((v * w as f32).round() as usize).min(w);
        format!("{}{}", "█".repeat(n), "░".repeat(w - n))
    };
    let status = match cap.stage {
        Stage::Stalking => {
            if cap.distance <= NET_RANGE {
                "IN RANGE — throw!"
            } else if cap.in_cover {
                "hidden in reeds"
            } else {
                "in the open"
            }
        }
        Stage::Netting(_) => "net in the air…",
        Stage::Caught(_) => "caught",
        Stage::Spooked(_) => "it's gone",
    };
    hud.side = format!(
        "AWARENESS\n{}  {:>3.0}%\n\nnoise  {}\nseen   {}\n\n{:.0} m · {}",
        bar(cap.awareness, 16),
        cap.awareness * 100.0,
        bar(cap.noise.min(1.0), 8),
        bar(cap.visibility, 8),
        cap.distance,
        status,
    );
    hud.banner = cap.message.clone();
}

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GamePhase::Capture), setup)
            .add_systems(OnExit(GamePhase::Capture), teardown)
            .add_systems(
                Update,
                (move_judith, sense, throw_net, fly_net, resolve)
                    .chain()
                    .in_set(crate::GameSet::Simulation)
                    .run_if(in_state(GamePhase::Capture)),
            )
            .add_systems(
                Update,
                hud.in_set(crate::GameSet::Hud)
                    .run_if(in_state(GamePhase::Capture)),
            );
    }
}
