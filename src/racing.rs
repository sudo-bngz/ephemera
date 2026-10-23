//! Phase 3 — the race.
//!
//! Everything that flies — Judith's Ephemera and the tribe's riders — shares
//! one flight model (`Rider` + `FlightInput` → `integrate`). The player
//! system just copies `InputAxes` into its `FlightInput`; the AI system
//! computes one from the track spline. One integrator means the rivals are
//! bound by exactly the same physics as the player, and tuning the feel
//! happens in a single place.
//!
//! Speed sensation is layered: a low chase camera that widens its FOV with
//! speed (`camera.rs`), speed lines streaming past, banking into turns,
//! camera shake on boost, reed beds whipping past at ground level, and a
//! "skim" bonus for flying low over water — which is also where the
//! obstacles are.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::camera::ChaseCamera;
use crate::components::{CameraTarget, PhaseScoped, Velocity};
use crate::creature::{Coat, Flap, spawn_ephemera};
use crate::hud::Hud;
use crate::input::InputAxes;
use crate::state::{EphemeraStats, GamePhase, RunLog};
use crate::world::{
    Mood, Pool, ReedClump, WetlandAssets, WetlandsConfig, spawn_reed_clump, spawn_wetlands,
};

pub const LAPS: u32 = 3;
const GATE_RADIUS: f32 = 8.0;
const GATE_HEIGHT: f32 = 11.0;
const MIN_ALT: f32 = 1.0;
const MAX_ALT: f32 = 34.0;
const COUNTDOWN: f32 = 3.6;
const SPEED_LINES: usize = 56;

// ---------------------------------------------------------------- track --

#[derive(Resource)]
pub struct Track {
    /// Dense polyline (Catmull–Rom through the gates), y = 0.
    points: Vec<Vec3>,
    /// Gate centres, in order. Gate 0 is the start/finish.
    pub gates: Vec<Vec3>,
    /// Facing direction of each gate.
    gate_dirs: Vec<Vec3>,
}

impl Track {
    fn generate(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let n = 11;
        let mut ctrl = Vec::with_capacity(n);
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            // Gentle radius noise: enough to make every straight different,
            // not enough to pinch two gates together.
            let rx = 160.0 + rng.gen_range(-18.0..18.0);
            let rz = 110.0 + rng.gen_range(-14.0..14.0);
            ctrl.push(Vec3::new(a.cos() * rx, 0.0, a.sin() * rz));
        }
        // Catmull–Rom, 24 samples per segment.
        let mut points = Vec::new();
        for i in 0..n {
            let p0 = ctrl[(i + n - 1) % n];
            let p1 = ctrl[i];
            let p2 = ctrl[(i + 1) % n];
            let p3 = ctrl[(i + 2) % n];
            for s in 0..24 {
                let t = s as f32 / 24.0;
                let t2 = t * t;
                let t3 = t2 * t;
                let p = 0.5
                    * ((2.0 * p1)
                        + (-p0 + p2) * t
                        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);
                points.push(p);
            }
        }
        let gate_dirs = (0..n)
            .map(|i| (ctrl[(i + 1) % n] - ctrl[(i + n - 1) % n]).normalize())
            .collect();
        Self {
            points,
            gates: ctrl,
            gate_dirs,
        }
    }

    fn nearest_index(&self, p: Vec3) -> usize {
        let mut best = 0;
        let mut best_d = f32::MAX;
        for (i, q) in self.points.iter().enumerate() {
            let d = (q.xz() - p.xz()).length_squared();
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        best
    }
}

// --------------------------------------------------------------- racers --

/// Flight-model state. Same struct for player and AI.
#[derive(Component, Debug)]
pub struct Rider {
    pub speed: f32,
    yaw: f32,
    pitch: f32,
    roll: f32,
    top_speed: f32,
    acceleration: f32,
    yaw_rate: f32,
    boost: f32,
    boost_max: f32,
    /// Low trust makes the creature buck now and then.
    trust: f32,
    buck: f32,
    buck_dir: f32,
}

#[derive(Component, Default, Debug, Clone, Copy)]
struct FlightInput {
    steer: f32,
    throttle: f32,
    pitch: f32,
    boost: bool,
}

#[derive(Component, Debug)]
pub struct Racer {
    pub name: &'static str,
    pub next_gate: usize,
    pub lap: u32,
    lap_start: f32,
    pub laps: Vec<f32>,
    finished: Option<f32>,
    /// Progress along the whole race, for placing. Updated each frame.
    progress: f32,
    pub place: u32,
}

#[derive(Component)]
struct Player;

#[derive(Component)]
struct Ai {
    skill: f32,
    lateral: f32,
    lookahead: usize,
}

#[derive(Component)]
struct Gate {
    index: usize,
}

#[derive(Component)]
struct SpeedLine {
    depth: f32,
    off: Vec2,
}

#[derive(Resource)]
struct Race {
    clock: f32,
    started: bool,
    finish_timer: Option<f32>,
    rng: StdRng,
    gate_idle: Handle<StandardMaterial>,
    gate_next: Handle<StandardMaterial>,
}

const RIVALS: [(&str, f32, Coat); 3] = [
    (
        "Oren",
        0.88,
        Coat {
            body: Color::srgb(0.32, 0.10, 0.12),
            glow: Color::srgb(1.0, 0.45, 0.25),
        },
    ),
    (
        "Teshi",
        0.80,
        Coat {
            body: Color::srgb(0.10, 0.22, 0.30),
            glow: Color::srgb(0.4, 0.7, 1.0),
        },
    ),
    (
        "Mab",
        0.72,
        Coat {
            body: Color::srgb(0.25, 0.24, 0.10),
            glow: Color::srgb(1.0, 0.9, 0.3),
        },
    ),
];

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<WetlandAssets>,
    stats: Res<EphemeraStats>,
    mut hud: ResMut<Hud>,
) {
    let cam = spawn_wetlands(
        &mut commands,
        &mut meshes,
        &assets,
        WetlandsConfig {
            seed: 2026,
            half_size: 230.0,
            reed_clumps: 320,
            pools: 60,
            spores: 160,
            mood: Mood::Noon,
            clearing: 0.0,
        },
    );
    commands.entity(cam).insert(ChaseCamera {
        base_fov: 64f32.to_radians(),
        fov_gain: 30f32.to_radians(),
        fov_speed_ref: 95.0,
        ..default()
    });

    let track = Track::generate(777);
    let mut rng = StdRng::seed_from_u64(5);

    // Line the track with reed beds so the ground reads as rushing past,
    // and put tall clumps *on* the racing line as obstacles.
    for (i, p) in track.points.iter().enumerate() {
        let next = track.points[(i + 1) % track.points.len()];
        let dir = (next - *p).normalize_or(Vec3::X);
        let side = Vec3::new(-dir.z, 0.0, dir.x);
        if i % 3 == 0 {
            for s in [-1.0, 1.0] {
                let off = 16.0 + rng.gen_range(0.0..10.0);
                let (radius, height) = (rng.gen_range(2.0..4.0), rng.gen_range(3.0..6.0));
                spawn_reed_clump(
                    &mut commands,
                    &assets,
                    &mut rng,
                    *p + side * s * off,
                    radius,
                    height,
                    false,
                );
            }
        }
        let near_gate = track
            .gates
            .iter()
            .any(|g| (g.xz() - p.xz()).length() < 28.0);
        if i % 29 == 11 && !near_gate {
            let off = rng.gen_range(-6.0..6.0);
            let (radius, height) = (rng.gen_range(2.5..3.5), rng.gen_range(9.0..13.0));
            spawn_reed_clump(
                &mut commands,
                &assets,
                &mut rng,
                *p + side * off,
                radius,
                height,
                true,
            );
        }
    }

    // Gates.
    let gate_idle = materials.add(StandardMaterial {
        base_color: Color::srgb(0.7, 0.6, 0.4),
        emissive: LinearRgba::new(0.4, 0.3, 0.1, 1.0),
        ..default()
    });
    let gate_next = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 1.0, 0.8),
        emissive: LinearRgba::new(1.5, 6.0, 4.0, 1.0),
        ..default()
    });
    let pylon = meshes.add(Cylinder::new(0.55, GATE_HEIGHT));
    let beam = meshes.add(Cuboid::new(GATE_RADIUS * 2.0 + 1.0, 0.6, 0.6));
    let flag = meshes.add(Cuboid::new(0.1, 2.2, 3.0));
    for (i, (g, d)) in track.gates.iter().zip(&track.gate_dirs).enumerate() {
        let yaw = (-d.x).atan2(-d.z);
        commands
            .spawn((
                PhaseScoped,
                Gate { index: i },
                Transform::from_translation(*g).with_rotation(Quat::from_rotation_y(yaw)),
                Visibility::default(),
            ))
            .with_children(|p| {
                for s in [-1.0, 1.0] {
                    p.spawn((
                        Mesh3d(pylon.clone()),
                        MeshMaterial3d(gate_idle.clone()),
                        Transform::from_xyz(s * GATE_RADIUS, GATE_HEIGHT * 0.5, 0.0),
                    ));
                    p.spawn((
                        Mesh3d(flag.clone()),
                        MeshMaterial3d(gate_idle.clone()),
                        Transform::from_xyz(s * (GATE_RADIUS + 0.2), GATE_HEIGHT - 1.4, 0.0),
                        NotShadowCaster,
                    ));
                }
                p.spawn((
                    Mesh3d(beam.clone()),
                    MeshMaterial3d(gate_idle.clone()),
                    Transform::from_xyz(0.0, GATE_HEIGHT, 0.0),
                ));
            });
    }

    // Starting grid: all behind gate 0, spread sideways.
    let start = track.gates[0];
    let dir = track.gate_dirs[0];
    let side = Vec3::new(-dir.z, 0.0, dir.x);
    let yaw0 = (-dir.x).atan2(-dir.z);
    let grid =
        |slot: f32| start - dir * (14.0 + slot.abs() * 4.0) + side * slot * 6.0 + Vec3::Y * 3.0;

    // Player.
    let player = spawn_ephemera(
        &mut commands,
        &mut meshes,
        &mut materials,
        grid(-0.5),
        Coat::WILD,
        0.4,
        (
            PhaseScoped,
            Player,
            CameraTarget::FLIGHT,
            Velocity::default(),
            FlightInput::default(),
            Rider {
                speed: 0.0,
                yaw: yaw0,
                pitch: 0.0,
                roll: 0.0,
                top_speed: stats.top_speed(),
                acceleration: stats.acceleration(),
                yaw_rate: stats.yaw_rate(),
                boost: stats.boost_capacity(),
                boost_max: stats.boost_capacity(),
                trust: stats.trust,
                buck: 0.0,
                buck_dir: 1.0,
            },
            Racer::new("Judith"),
        ),
    );
    // Judith in the saddle.
    let saddle =
        crate::components::spawn_judith(&mut commands, &mut meshes, &mut materials, Vec3::ZERO);
    commands
        .entity(saddle)
        .remove::<Velocity>()
        .insert(Transform::from_xyz(0.0, 0.55, -0.6).with_scale(Vec3::splat(0.9)))
        .insert(ChildOf(player));

    // Rivals.
    for (i, (name, skill, coat)) in RIVALS.iter().enumerate() {
        let slot = [0.5, 1.5, -1.5][i];
        spawn_ephemera(
            &mut commands,
            &mut meshes,
            &mut materials,
            grid(slot),
            *coat,
            0.4,
            (
                PhaseScoped,
                Ai {
                    skill: *skill,
                    lateral: rng.gen_range(-4.0..4.0),
                    lookahead: 14 + i * 2,
                },
                Velocity::default(),
                FlightInput::default(),
                Rider {
                    speed: 0.0,
                    yaw: yaw0,
                    pitch: 0.0,
                    roll: 0.0,
                    top_speed: 50.0 + 30.0 * skill,
                    acceleration: 26.0 + 18.0 * skill,
                    yaw_rate: 2.0 + 1.1 * skill,
                    boost: 4.0,
                    boost_max: 4.0,
                    trust: 1.0,
                    buck: 0.0,
                    buck_dir: 1.0,
                },
                Racer::new(name),
            ),
        );
    }

    // Speed lines: thin streaks parented to nothing; positioned relative to
    // the camera every frame.
    let streak = meshes.add(Cuboid::new(0.03, 0.03, 3.5));
    let streak_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
        emissive: LinearRgba::new(2.0, 2.0, 2.0, 1.0),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    for _ in 0..SPEED_LINES {
        let a = rng.gen_range(0.0..std::f32::consts::TAU);
        let r = rng.gen_range(2.0_f32..7.0);
        commands.spawn((
            PhaseScoped,
            SpeedLine {
                depth: rng.gen_range(2.0..40.0),
                off: Vec2::new(a.cos() * r, a.sin() * r * 0.7),
            },
            Mesh3d(streak.clone()),
            MeshMaterial3d(streak_mat.clone()),
            Transform::default(),
            Visibility::Hidden,
            NotShadowCaster,
        ));
    }

    commands.insert_resource(track);
    commands.insert_resource(Race {
        clock: -COUNTDOWN,
        started: false,
        finish_timer: None,
        rng,
        gate_idle,
        gate_next,
    });

    hud.title = "III. THE GATHERING".into();
    hud.body = format!(
        "{} laps. Fly through every gate in order.\nSkim the pools for a push. The tall reeds will stop you dead.",
        LAPS
    );
    hud.banner = String::new();
    hud.footer = "W/S throttle · A/D bank · Q dive / E climb · SHIFT boost".into();
}

impl Racer {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            // The grid sits just behind gate 0, so the first target is gate 1
            // and crossing gate 0 again closes a lap.
            next_gate: 1,
            lap: 1,
            lap_start: 0.0,
            laps: Vec::new(),
            finished: None,
            progress: 0.0,
            place: 1,
        }
    }
}

fn teardown(mut commands: Commands) {
    commands.remove_resource::<Track>();
    commands.remove_resource::<Race>();
}

// -------------------------------------------------------------- systems --

fn clock(time: Res<Time>, mut race: ResMut<Race>) {
    race.clock += time.delta_secs();
    if !race.started && race.clock >= 0.0 {
        race.started = true;
    }
}

fn player_input(
    axes: Res<InputAxes>,
    race: Res<Race>,
    mut q: Query<&mut FlightInput, With<Player>>,
) {
    for mut fi in &mut q {
        *fi = if race.started {
            FlightInput {
                steer: axes.steer,
                throttle: axes.throttle,
                pitch: axes.pitch,
                boost: axes.boost,
            }
        } else {
            FlightInput::default()
        };
    }
}

fn ai_input(
    track: Res<Track>,
    race: Res<Race>,
    player: Query<&Racer, With<Player>>,
    mut q: Query<(&Transform, &Rider, &mut Ai, &Racer, &mut FlightInput), Without<Player>>,
    reeds: Query<(&GlobalTransform, &ReedClump)>,
) {
    let player_progress = player.single().map(|r| r.progress).unwrap_or(0.0);
    for (tf, rider, mut ai, racer, mut fi) in &mut q {
        if !race.started || racer.finished.is_some() {
            *fi = FlightInput::default();
            continue;
        }
        let idx = track.nearest_index(tf.translation);
        let n = track.points.len();
        let ahead = track.points[(idx + ai.lookahead) % n];
        let ahead2 = track.points[(idx + ai.lookahead * 2) % n];
        let dir = (ahead2 - ahead).normalize_or(Vec3::X);
        let side = Vec3::new(-dir.z, 0.0, dir.x);
        // Drift the lateral offset slowly so rivals don't fly a rail.
        ai.lateral += (((race.clock * 0.3) + ai.skill * 7.0).sin()) * 0.02;
        let mut target = ahead + side * ai.lateral.clamp(-5.0, 5.0);
        // Once the next gate is close, aim at the gate itself rather than the
        // spline, so the rider actually threads it instead of cutting inside.
        let gate = track.gates[racer.next_gate];
        if (gate.xz() - tf.translation.xz()).length() < 70.0 {
            let gdir = track.gate_dirs[racer.next_gate];
            let gside = Vec3::new(-gdir.z, 0.0, gdir.x);
            target = gate + gside * (ai.lateral * 0.5).clamp(-3.0, 3.0);
        }

        // Avoid tall clumps ahead: nudge the target sideways, away from any
        // obstacle that sits close to the line between here and the target.
        let fwd = (target - tf.translation).normalize_or(dir);
        let fwd_side = Vec3::new(-fwd.z, 0.0, fwd.x);
        for (rt, reed) in &reeds {
            if reed.height < 6.0 {
                continue;
            }
            let rel = rt.translation() - tf.translation;
            let along = rel.dot(fwd);
            if !(0.0..45.0).contains(&along) {
                continue;
            }
            let lateral = rel.dot(fwd_side);
            let clearance = reed.radius + 6.0;
            if lateral.abs() < clearance {
                let away = if lateral >= 0.0 { -1.0 } else { 1.0 };
                target += fwd_side * away * (clearance - lateral.abs()) * 1.5;
            }
        }

        let to = target - tf.translation;
        let wanted_yaw = (-to.x).atan2(-to.z);
        let mut dyaw = wanted_yaw - rider.yaw;
        while dyaw > std::f32::consts::PI {
            dyaw -= std::f32::consts::TAU;
        }
        while dyaw < -std::f32::consts::PI {
            dyaw += std::f32::consts::TAU;
        }
        // Rubber band: ease off when well ahead of the player.
        let gap = racer.progress - player_progress;
        let throttle = if gap > 1.5 {
            0.72
        } else if gap < -1.5 {
            1.0
        } else {
            0.9
        };
        let alt_target = 4.5;
        *fi = FlightInput {
            steer: (-dyaw * 2.2).clamp(-1.0, 1.0),
            throttle,
            pitch: ((alt_target - tf.translation.y) * 0.4).clamp(-1.0, 1.0),
            boost: gap < -0.5 && dyaw.abs() < 0.2,
        };
    }
}

fn integrate(
    time: Res<Time>,
    mut race: ResMut<Race>,
    mut cams: Query<&mut ChaseCamera>,
    mut q: Query<(
        &mut Transform,
        &mut Rider,
        &FlightInput,
        &mut Velocity,
        &mut Flap,
        Option<&Player>,
    )>,
    reeds: Query<(&GlobalTransform, &ReedClump)>,
    pools: Query<(&GlobalTransform, &Pool)>,
) {
    let dt = time.delta_secs().min(1.0 / 30.0);
    // The first frame of the app (e.g. `--race`) has a zero delta: there is
    // nothing to integrate, and `/ dt` below would poison everything with NaN.
    if dt <= f32::EPSILON {
        return;
    }
    for (mut tf, mut r, input, mut vel, mut flap, player) in &mut q {
        let is_player = player.is_some();
        let p = tf.translation;

        // --- bucking (low trust) -------------------------------------
        if is_player && r.buck <= 0.0 && race.started {
            let refusal = (0.35 - r.trust).max(0.0) * 0.9;
            if race.rng.gen_bool((refusal as f64 * dt as f64).min(1.0)) {
                r.buck = 0.7;
                r.buck_dir = if race.rng.gen_bool(0.5) { 1.0 } else { -1.0 };
                if let Ok(mut c) = cams.single_mut() {
                    c.kick(0.5);
                }
            }
        }
        let bucking = r.buck > 0.0;
        r.buck = (r.buck - dt).max(0.0);

        // --- boost --------------------------------------------------------
        let boosting = input.boost && r.boost > 0.0 && !bucking && input.throttle >= 0.0;
        if boosting {
            r.boost -= dt;
        } else {
            r.boost = (r.boost + dt * 0.22).min(r.boost_max);
        }

        // --- terrain modifiers ---------------------------------------------
        let over_pool = pools
            .iter()
            .any(|(t, pool)| (t.translation().xz() - p.xz()).length() < pool.radius);
        let skimming = over_pool && p.y < 2.6;

        // --- longitudinal ----------------------------------------------------
        let top =
            r.top_speed * if boosting { 1.45 } else { 1.0 } * if skimming { 1.08 } else { 1.0 };
        let accel = r.acceleration * if boosting { 1.9 } else { 1.0 };
        let throttle = if bucking {
            0.3
        } else {
            input.throttle.max(0.0)
        };
        let brake = (-input.throttle).max(0.0);
        // Quadratic drag tuned so terminal velocity == `top`.
        let drag = r.acceleration * (r.speed * r.speed) / (top * top);
        r.speed += (accel * throttle - drag - brake * r.acceleration * 1.6 - 2.0) * dt;
        r.speed = r.speed.clamp(0.0, top * 1.05);

        // --- yaw / roll --------------------------------------------------------
        let speed_t = (r.speed / r.top_speed).clamp(0.0, 1.0);
        let mut steer = input.steer;
        if bucking {
            steer = r.buck_dir * (r.buck * 9.0).sin().signum();
        }
        let rate = r.yaw_rate * (0.55 + 0.45 * (1.0 - speed_t * 0.8));
        r.yaw -= steer * rate * dt;
        let roll_target = -steer * (0.35 + 0.65 * speed_t) * 1.05;
        r.roll += (roll_target - r.roll) * (1.0 - (-7.0 * dt).exp());

        // --- pitch / altitude -------------------------------------------------
        let pitch_target = input.pitch * 0.55;
        r.pitch += (pitch_target - r.pitch) * (1.0 - (-6.0 * dt).exp());

        // --- integrate -----------------------------------------------------------
        let forward = Quat::from_rotation_y(r.yaw) * Vec3::NEG_Z;
        let mut v = forward * r.speed * r.pitch.cos();
        v.y = r.speed * r.pitch.sin() * 0.6;
        // Altitude is clamped softly: pushing into the limits bleeds pitch.
        let mut next_pos = p + v * dt;
        if next_pos.y < MIN_ALT {
            next_pos.y = MIN_ALT;
            if r.pitch < 0.0 {
                r.pitch *= 0.3;
            }
            // Dragging through the mud.
            r.speed *= 1.0 - 0.9 * dt;
        }
        if next_pos.y > MAX_ALT {
            next_pos.y = MAX_ALT;
            r.pitch = r.pitch.min(0.0);
        }

        // --- reed collisions --------------------------------------------------------
        for (rt, reed) in &reeds {
            let c = rt.translation();
            let d = (c.xz() - next_pos.xz()).length();
            if d < reed.radius + 0.8 && next_pos.y < reed.height + 0.5 {
                let push = (next_pos.xz() - c.xz()).normalize_or(Vec2::X);
                let out = c.xz() + push * (reed.radius + 1.6);
                next_pos.x = out.x;
                next_pos.z = out.y;
                let hit = (r.speed / r.top_speed).min(1.0);
                // Lose most of the speed, but never below a crawl, and get
                // deflected along the clump so nobody can wedge into it.
                r.speed = (r.speed * 0.45).max(12.0);
                let away = (-push.x).atan2(-push.y);
                let mut d_yaw = away - r.yaw;
                while d_yaw > std::f32::consts::PI {
                    d_yaw -= std::f32::consts::TAU;
                }
                while d_yaw < -std::f32::consts::PI {
                    d_yaw += std::f32::consts::TAU;
                }
                r.yaw += d_yaw * 0.5;
                if is_player {
                    if let Ok(mut cam) = cams.single_mut() {
                        cam.kick(0.3 + 0.6 * hit);
                    }
                }
            }
        }

        vel.0 = (next_pos - p) / dt;
        tf.translation = next_pos;
        tf.rotation = Quat::from_euler(EulerRot::YXZ, r.yaw, r.pitch * 0.7, r.roll);

        // --- feel ---------------------------------------------------------------------------
        flap.intensity = 0.35 + 0.65 * speed_t;
        if is_player {
            if let Ok(mut cam) = cams.single_mut() {
                cam.roll = r.roll * 0.35;
                if boosting {
                    cam.kick(0.12);
                }
                if skimming {
                    cam.kick(0.04);
                }
            }
        }
    }
}

fn gates(
    track: Res<Track>,
    mut race: ResMut<Race>,
    mut q: Query<(&Transform, &mut Racer, Option<&Player>)>,
    mut cams: Query<&mut ChaseCamera>,
    mut log: ResMut<RunLog>,
) {
    let n = track.gates.len();
    for (tf, mut racer, player) in &mut q {
        if racer.finished.is_some() || !race.started {
            continue;
        }
        let g = track.gates[racer.next_gate];
        let d = (g.xz() - tf.translation.xz()).length();
        if d < GATE_RADIUS && tf.translation.y < GATE_HEIGHT {
            racer.next_gate = (racer.next_gate + 1) % n;
            if racer.next_gate == 0 {
                let lap_time = race.clock - racer.lap_start;
                racer.laps.push(lap_time);
                if player.is_some() {
                    log.best_lap = Some(log.best_lap.map_or(lap_time, |b| b.min(lap_time)));
                }
                racer.lap += 1;
                racer.lap_start = race.clock;
                if racer.lap > LAPS {
                    racer.finished = Some(race.clock);
                    if player.is_some() {
                        log.total_time = race.clock;
                        race.finish_timer = Some(3.5);
                        if let Ok(mut c) = cams.single_mut() {
                            c.kick(0.8);
                        }
                    }
                }
            } else if player.is_some() {
                if let Ok(mut c) = cams.single_mut() {
                    c.kick(0.08);
                }
            }
        }
        // Progress for placings: laps + gates + fraction to next gate.
        let prev = track.gates[(racer.next_gate + n - 1) % n];
        let seg = (g - prev).length().max(1.0);
        let frac = 1.0 - ((g.xz() - tf.translation.xz()).length() / seg).clamp(0.0, 1.0);
        racer.progress = racer.lap as f32 * n as f32 + racer.next_gate as f32 + frac;
    }
}

fn placings(mut q: Query<&mut Racer>) {
    // Finished racers rank by finish time, the rest by progress.
    let mut ranked: Vec<(usize, f32, Option<f32>)> = q
        .iter()
        .enumerate()
        .map(|(i, r)| (i, r.progress, r.finished))
        .collect();
    ranked.sort_by(|a, b| match (a.2, b.2) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.1.total_cmp(&a.1),
    });
    let mut place_of = vec![1u32; ranked.len()];
    for (place, (i, _, _)) in ranked.iter().enumerate() {
        place_of[*i] = place as u32 + 1;
    }
    for (i, mut r) in q.iter_mut().enumerate() {
        r.place = place_of[i];
    }
}

fn highlight_gates(
    race: Res<Race>,
    player: Query<&Racer, With<Player>>,
    gates: Query<(&Gate, &Children)>,
    mut mats: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    let Ok(p) = player.single() else {
        return;
    };
    for (gate, children) in &gates {
        let mat = if gate.index == p.next_gate {
            &race.gate_next
        } else {
            &race.gate_idle
        };
        for c in children.iter() {
            if let Ok(mut m) = mats.get_mut(c) {
                if m.0 != *mat {
                    m.0 = mat.clone();
                }
            }
        }
    }
}

/// Streaks that rush past the camera, scaled by the player's speed.
fn speed_lines(
    time: Res<Time>,
    player: Query<(&Rider, &Velocity), With<Player>>,
    cam: Query<&Transform, (With<ChaseCamera>, Without<SpeedLine>)>,
    mut lines: Query<(&mut SpeedLine, &mut Transform, &mut Visibility)>,
) {
    let (Ok((rider, vel)), Ok(cam_tf)) = (player.single(), cam.single()) else {
        return;
    };
    let speed = vel.0.length();
    let t = (speed / rider.top_speed - 0.55).max(0.0) / 0.6;
    let fwd = cam_tf.forward().as_vec3();
    let right = cam_tf.right().as_vec3();
    let up = cam_tf.up().as_vec3();
    for (mut line, mut tf, mut vis) in &mut lines {
        if t <= 0.0 {
            *vis = Visibility::Hidden;
            continue;
        }
        *vis = Visibility::Visible;
        line.depth -= speed * time.delta_secs() * 1.3;
        if line.depth < 1.5 {
            line.depth += 40.0;
        }
        tf.translation =
            cam_tf.translation + fwd * line.depth + right * line.off.x + up * line.off.y;
        tf.rotation = cam_tf.rotation;
        tf.scale = Vec3::new(1.0, 1.0, 0.5 + 3.0 * t);
    }
}

fn hud(
    race: Res<Race>,
    player: Query<(&Rider, &Racer, &Velocity), With<Player>>,
    racers: Query<&Racer>,
    mut hud: ResMut<Hud>,
) {
    let Ok((rider, racer, vel)) = player.single() else {
        return;
    };
    let bar = |v: f32| -> String {
        let n = ((v * 10.0).round() as usize).min(10);
        format!("{}{}", "█".repeat(n), "░".repeat(10 - n))
    };
    let kmh = vel.0.length() * 3.6;
    let lap_times: String = racer
        .laps
        .iter()
        .enumerate()
        .map(|(i, t)| format!("\nlap {}  {:>6.2}", i + 1, t))
        .collect();
    let mut standings: Vec<&Racer> = racers.iter().collect();
    standings.sort_by_key(|r| r.place);
    let board: String = standings
        .iter()
        .map(|r| format!("\n{}  {}", r.place, r.name))
        .collect();
    hud.side = format!(
        "{:>4.0} km/h\nlap {}/{}\nboost {}\n{}\n\nP{}{}",
        kmh,
        racer.lap.clamp(1, LAPS),
        LAPS,
        bar(rider.boost / rider.boost_max),
        lap_times,
        racer.place,
        board,
    );
    hud.banner = if race.clock < 0.0 {
        format!("{}", (-race.clock).ceil() as i32)
    } else if race.clock < 1.0 {
        "GO".into()
    } else if racer.finished.is_some() {
        match racer.place {
            1 => "FIRST — SHE HAS A NAME".into(),
            p => format!("FINISHED {}", ordinal(p)),
        }
    } else if rider.buck > 0.0 {
        "it bucks!".into()
    } else {
        String::new()
    };
    if race.clock > 1.0 {
        hud.body = String::new();
    }
}

fn ordinal(n: u32) -> String {
    match n {
        1 => "1st".into(),
        2 => "2nd".into(),
        3 => "3rd".into(),
        n => format!("{n}th"),
    }
}

fn finish(
    time: Res<Time>,
    mut race: ResMut<Race>,
    player: Query<&Racer, With<Player>>,
    mut log: ResMut<RunLog>,
    mut next: ResMut<NextState<GamePhase>>,
) {
    if let Some(t) = race.finish_timer {
        let t = t - time.delta_secs();
        if t <= 0.0 {
            if let Ok(r) = player.single() {
                log.place = Some(r.place);
            }
            next.set(GamePhase::Results);
        } else {
            race.finish_timer = Some(t);
        }
    }
}

// -------------------------------------------------------------- results --

fn results_setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<WetlandAssets>,
    log: Res<RunLog>,
    stats: Res<EphemeraStats>,
    mut hud: ResMut<Hud>,
) {
    let cam = spawn_wetlands(
        &mut commands,
        &mut meshes,
        &assets,
        WetlandsConfig {
            seed: 9,
            half_size: 80.0,
            reed_clumps: 90,
            pools: 16,
            spores: 100,
            mood: Mood::Dusk,
            clearing: 10.0,
        },
    );
    commands.entity(cam).insert(ChaseCamera {
        orbit: Some(0.15),
        ..default()
    });
    spawn_ephemera(
        &mut commands,
        &mut meshes,
        &mut materials,
        Vec3::new(0.0, 2.2, 0.0),
        Coat::WILD,
        0.1,
        (PhaseScoped, CameraTarget::CAMP),
    );
    let (verdict, story) = match log.place {
        Some(1) => (
            "THE TRIBE GIVES HER A NAME",
            "She crossed the last gate first. Tonight they will sing it to her: \
             a name for the orphan who rode the wind.",
        ),
        Some(2) | Some(3) => (
            "NOT THIS SEASON",
            "A good flight — the elders saw it. But only the first rider is named. \
             The marsh will still be here at the next Gathering.",
        ),
        _ => (
            "THE MARSH REMEMBERS",
            "She finished. The creature carried her home. That, too, is something.",
        ),
    };
    hud.title = verdict.into();
    hud.body = format!(
        "{story}\n\nPlace {} · total {:.2}s · best lap {}\n\
         Stalks {} · spooked {} · dawns at the tether {}\n\
         Final bond: trust {:.0}%, speed {:.0}%, agility {:.0}%",
        log.place.map_or("—".into(), ordinal),
        log.total_time,
        log.best_lap.map_or("—".into(), |t| format!("{t:.2}s")),
        log.capture_attempts,
        log.creature_spooked,
        log.dawns_spent,
        stats.trust * 100.0,
        stats.speed * 100.0,
        stats.agility * 100.0,
    );
    hud.banner = String::new();
    hud.side = String::new();
    hud.footer =
        "R — race again with this Ephemera      ENTER — begin a new rite      ESC — title".into();
}

fn results_update(
    keys: Res<ButtonInput<KeyCode>>,
    axes: Res<InputAxes>,
    mut next: ResMut<NextState<GamePhase>>,
    mut stats: ResMut<EphemeraStats>,
    mut log: ResMut<RunLog>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        log.place = None;
        next.set(GamePhase::Racing);
    } else if axes.confirm_pressed {
        *stats = EphemeraStats::default();
        *log = RunLog::default();
        next.set(GamePhase::Capture);
    }
}

pub struct RacingPlugin;

impl Plugin for RacingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GamePhase::Racing), setup)
            .add_systems(OnExit(GamePhase::Racing), teardown)
            .add_systems(
                Update,
                (
                    clock,
                    player_input,
                    ai_input,
                    integrate,
                    gates,
                    placings,
                    finish,
                )
                    .chain()
                    .in_set(crate::GameSet::Simulation)
                    .run_if(in_state(GamePhase::Racing)),
            )
            .add_systems(
                Update,
                (speed_lines, highlight_gates)
                    .after(crate::camera::follow)
                    .in_set(crate::GameSet::Camera)
                    .run_if(in_state(GamePhase::Racing)),
            )
            .add_systems(
                Update,
                hud.in_set(crate::GameSet::Hud)
                    .run_if(in_state(GamePhase::Racing)),
            )
            .add_systems(OnEnter(GamePhase::Results), results_setup)
            .add_systems(
                Update,
                results_update
                    .in_set(crate::GameSet::Simulation)
                    .run_if(in_state(GamePhase::Results)),
            );
    }
}
