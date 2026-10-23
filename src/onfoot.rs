//! Judith on her own two feet.
//!
//! In the endless world, `F` brings the Ephemera down and Judith slides off
//! its back. She walks, runs, jumps, wades and swims through the land at her
//! own height — under the mushrooms, into the villages — with the camera
//! over her shoulder, turned by the mouse or the arrow keys. `F` beside the
//! waiting creature puts her back in the saddle; from further away it
//! whistles the Ephemera over.
//!
//! The creature keeps its flight model while parked (`racing::Parked`), so
//! it idles and answers the whistle with the same physics it races with.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::camera::{CameraMode, ChaseCamera};
use crate::components::{CameraTarget, Ephemera, Judith, Velocity};
use crate::input::InputAxes;
use crate::racing::{AutoLand, Parked, Player, Rider, mount};
use crate::rig::Stance;
use crate::state::GamePhase;
use crate::terrain::SEA_LEVEL;
use crate::world::{Obstacle, WorldTerrain};

const WALK: f32 = 4.6;
const RUN: f32 = 8.8;
const CREEP: f32 = 2.4;
const SWIM: f32 = 2.6;
const JUMP: f32 = 6.8;
const GRAVITY: f32 = 20.0;
/// How far from the Ephemera she can climb back on.
const MOUNT_REACH: f32 = 6.5;

/// Judith walking the world.
#[derive(Component, Debug, Default)]
pub struct Walker {
    pub vertical: f32,
    pub grounded: bool,
    pub swimming: bool,
    pub yaw: f32,
    /// Seconds since she last stood on the ground.
    pub airtime: f32,
}

/// The chase camera seen over her shoulder.
const SHOULDER: CameraTarget = CameraTarget {
    distance: 5.2,
    height: 1.55,
    stiffness: 9.0,
    look_ahead: 0.0,
};

/// `F`: land and get off, climb back on, or whistle.
fn interact(
    mut commands: Commands,
    axes: Res<InputAxes>,
    terrain: Res<WorldTerrain>,
    mut ephemera: Query<
        (Entity, &Transform, Option<&mut Parked>, Has<AutoLand>),
        (With<Player>, With<Ephemera>),
    >,
    judith: Query<(Entity, &GlobalTransform, Has<Walker>), With<Judith>>,
    mut cams: Query<&mut ChaseCamera>,
) {
    if !axes.interact_pressed {
        return;
    }
    let (Ok((eph, etf, parked, landing)), Ok((jud, jtf, on_foot))) =
        (ephemera.single_mut(), judith.single())
    else {
        return;
    };
    if on_foot {
        let d = (jtf.translation() - etf.translation).xz().length();
        if d < MOUNT_REACH {
            // Back in the saddle.
            mount(&mut commands, jud, eph);
            commands
                .entity(jud)
                .remove::<(Walker, Stance, CameraTarget)>();
            commands
                .entity(eph)
                .remove::<Parked>()
                .insert(CameraTarget::FLIGHT);
            if let Ok(mut cam) = cams.single_mut() {
                cam.mode = CameraMode::Chase;
            }
        } else if let Some(mut parked) = parked {
            // Whistle: come here.
            let here = jtf.translation();
            parked.summon = Some(here.with_y(terrain.0.surface(here.x, here.z)));
        }
    } else if !landing {
        // Come down; `land` dismounts once the creature is slow and low.
        commands.entity(eph).insert(AutoLand);
    }
}

/// Dismounts once the landing Ephemera is slow and near the ground.
fn land(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<WorldTerrain>,
    mut ephemera: Query<(Entity, &mut Transform, &Rider), (With<Player>, With<AutoLand>)>,
    judith: Query<Entity, (With<Judith>, Without<Walker>)>,
    mut cams: Query<&mut ChaseCamera>,
) {
    let (Ok((eph, mut etf, rider)), Ok(jud)) = (ephemera.single_mut(), judith.single()) else {
        return;
    };
    let p = etf.translation;
    let ground = terrain.0.surface(p.x, p.z);
    if rider.speed > 2.5 {
        return;
    }
    if p.y > ground + 1.8 {
        // Hovering: the flight model only sinks under way, so let it down.
        etf.translation.y -= 4.5 * time.delta_secs();
        return;
    }
    // Off the right-hand side, onto the ground (or into the water).
    let right = etf.right().as_vec3().with_y(0.0).normalize_or(Vec3::X);
    let at = p + right * 2.6;
    let floor = terrain.0.height(at.x, at.z);
    let y = if floor < SEA_LEVEL - 1.1 {
        SEA_LEVEL - 0.9
    } else {
        floor.max(SEA_LEVEL - 1.1)
    };
    let yaw = (-right.x).atan2(-right.z);
    commands.entity(jud).remove::<ChildOf>().insert((
        Transform::from_translation(at.with_y(y)).with_rotation(Quat::from_rotation_y(yaw)),
        Velocity::default(),
        Walker {
            yaw,
            grounded: true,
            ..default()
        },
        Stance::Stand,
        SHOULDER,
    ));
    commands
        .entity(eph)
        .remove::<(AutoLand, CameraTarget)>()
        .insert(Parked::default());
    if let Ok(mut cam) = cams.single_mut() {
        cam.mode = CameraMode::Orbit;
        // Start looking the way the creature was flying.
        let f = etf.forward().as_vec3();
        cam.orbit_yaw = (-f.x).atan2(-f.z);
        cam.orbit_pitch = 0.3;
        cam.idle = 10.0;
    }
}

/// The parked Ephemera has reached the whistle: stop there.
fn arrive(mut q: Query<(&Transform, &mut Parked)>) {
    for (tf, mut parked) in &mut q {
        if let Some(target) = parked.summon {
            if (target - tf.translation).xz().length() <= 3.0 {
                parked.summon = None;
            }
        }
    }
}

/// Turns the camera with the mouse and the arrows.
fn look(time: Res<Time>, axes: Res<InputAxes>, mut cams: Query<&mut ChaseCamera>) {
    let Ok(mut cam) = cams.single_mut() else {
        return;
    };
    if cam.mode != CameraMode::Orbit {
        return;
    }
    let dt = time.delta_secs();
    let yaw = -axes.look_mouse.x * 0.0028 - axes.look_keys.x * 2.4 * dt;
    let pitch = axes.look_mouse.y * 0.0022 - axes.look_keys.y * 1.6 * dt;
    cam.turn(yaw, pitch);
}

/// Walking, running, jumping, wading, swimming — relative to the camera.
fn walk(
    time: Res<Time>,
    axes: Res<InputAxes>,
    terrain: Res<WorldTerrain>,
    cams: Query<&ChaseCamera>,
    obstacles: Query<(&GlobalTransform, &Obstacle)>,
    mut q: Query<(&mut Transform, &mut Velocity, &mut Walker, &mut Stance), With<Judith>>,
) {
    let Ok((mut tf, mut vel, mut w, mut stance)) = q.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(1.0 / 30.0);
    if dt <= f32::EPSILON {
        return;
    }
    let forward = cams
        .single()
        .map(|c| c.orbit_forward())
        .unwrap_or(Vec3::NEG_Z);
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let wish = (forward * axes.walk.y + right * axes.walk.x).clamp_length_max(1.0);

    let p = tf.translation;
    let ground = terrain.0.height(p.x, p.z);
    let depth = SEA_LEVEL - ground;
    w.swimming = depth > 1.1;

    let speed = if w.swimming {
        SWIM
    } else if axes.crouch {
        CREEP
    } else if axes.boost {
        RUN
    } else {
        WALK
    } * if depth > 0.35 && !w.swimming {
        0.6
    } else {
        1.0
    };
    let k = if w.grounded || w.swimming { 12.0 } else { 3.0 };
    let wanted = wish * speed;
    let horizontal = vel.0.with_y(0.0).lerp(wanted, 1.0 - (-k * dt).exp());

    // Vertical: gravity and jumps on land, buoyancy in the water.
    if w.swimming {
        w.vertical = 0.0;
        w.grounded = false;
    } else {
        if w.grounded && axes.action_pressed && !axes.crouch {
            w.vertical = JUMP;
            w.grounded = false;
        }
        w.vertical -= GRAVITY * dt;
    }

    // Slopes: climbing is slow, and a face steeper than ~65° can't be
    // climbed at all — she slides along it instead of stopping dead.
    let mut horizontal = horizontal;
    let mut blocked = false;
    if (w.grounded || w.swimming) && horizontal.length() > 1e-3 {
        let e = 0.5;
        let grad = Vec2::new(
            terrain.0.height(p.x + e, p.z) - terrain.0.height(p.x - e, p.z),
            terrain.0.height(p.x, p.z + e) - terrain.0.height(p.x, p.z - e),
        ) / (2.0 * e);
        let up = grad.length();
        let climbing = horizontal
            .xz()
            .normalize_or_zero()
            .dot(grad.normalize_or_zero());
        if climbing > 0.0 && up > 0.3 {
            if up > 2.2 {
                // A wall: keep only the motion along the contour.
                let n = Vec3::new(grad.x, 0.0, grad.y).normalize();
                horizontal -= n * horizontal.dot(n).max(0.0);
                blocked = horizontal.length() < 0.3;
            } else {
                horizontal *= 1.0 / (1.0 + 0.8 * up * climbing);
            }
        }
    }
    let mut next = p + horizontal * dt;
    // Solid things: pushed out of their cylinders.
    for (ot, o) in &obstacles {
        let c = ot.translation();
        if next.y + 1.0 < c.y - o.below || next.y > c.y + o.above {
            continue;
        }
        let rel = next.xz() - c.xz();
        let reach = o.radius + 0.45;
        let d = rel.length();
        if d < reach {
            let push = rel.normalize_or(Vec2::X) * (reach - d);
            next.x += push.x;
            next.z += push.y;
        }
    }
    let floor = terrain.0.height(next.x, next.z);
    if w.swimming {
        next.y += (SEA_LEVEL - 0.9 - next.y) * (1.0 - (-6.0 * dt).exp());
        if floor > SEA_LEVEL - 1.1 {
            // Found her feet.
            w.swimming = false;
            w.grounded = true;
            next.y = floor;
        }
    } else {
        next.y += w.vertical * dt;
        let stand = floor.max(SEA_LEVEL - 1.1);
        if next.y <= stand {
            next.y = stand;
            w.vertical = 0.0;
            w.grounded = true;
        } else if next.y > stand + 0.05 {
            w.grounded = false;
        }
    }
    w.airtime = if w.grounded || w.swimming {
        0.0
    } else {
        w.airtime + dt
    };

    // Face the way she goes.
    let moving = horizontal.with_y(0.0);
    if moving.length() > 0.4 && !blocked {
        let wanted = (-moving.x).atan2(-moving.z);
        let d = wrap(wanted - w.yaw);
        w.yaw += d * (1.0 - (-12.0 * dt).exp());
    }
    vel.0 = (next - p) / dt;
    tf.translation = next;
    tf.rotation = Quat::from_rotation_y(w.yaw);

    *stance = if w.swimming {
        Stance::Swim
    } else if !w.grounded && w.airtime > 0.12 {
        Stance::Air {
            rising: w.vertical > 0.0,
        }
    } else if axes.crouch {
        Stance::Crouch
    } else {
        Stance::Stand
    };
}

fn wrap(mut a: f32) -> f32 {
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// The mouse is captured while she walks, free otherwise.
fn cursor(
    walkers: Query<(), With<Walker>>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = windows.single_mut() else {
        return;
    };
    let on_foot = !walkers.is_empty();
    let wanted = if on_foot {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    if cursor.grab_mode != wanted {
        cursor.grab_mode = wanted;
        cursor.visible = !on_foot;
    }
}

fn release_cursor(mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut cursor) = windows.single_mut() {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

pub struct OnFootPlugin;

impl Plugin for OnFootPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (interact, look)
                .in_set(crate::GameSet::Input)
                .after(crate::input::sample)
                .run_if(in_state(GamePhase::Explore)),
        )
        .add_systems(
            Update,
            (land, arrive, walk)
                .chain()
                .after(crate::racing::integrate)
                .in_set(crate::GameSet::Simulation)
                .run_if(in_state(GamePhase::Explore)),
        )
        .add_systems(
            Update,
            cursor
                .in_set(crate::GameSet::Hud)
                .run_if(in_state(GamePhase::Explore)),
        )
        .add_systems(OnExit(GamePhase::Explore), release_cursor);
    }
}
