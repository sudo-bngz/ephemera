//! The chase camera. Speed sensation lives mostly here: the camera lags
//! behind on acceleration, pulls in and widens its field of view at high
//! speed, rolls into turns and shakes on boost.
//!
//! On foot it becomes an **orbit camera**: it hangs over Judith's shoulder
//! at the angle the mouse (or the arrow keys) set, drifts back behind her
//! when left alone, and never sinks into the ground.

use bevy::prelude::*;

use crate::components::{CameraTarget, Velocity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    /// Behind the target, along its heading.
    #[default]
    Chase,
    /// Around the target, where the player looks.
    Orbit,
}

#[derive(Component, Debug)]
pub struct ChaseCamera {
    pub base_fov: f32,
    /// Extra radians of FOV at `fov_speed_ref` world units/sec.
    pub fov_gain: f32,
    pub fov_speed_ref: f32,
    pub shake: f32,
    pub roll: f32,
    /// Set by phases that want the camera to orbit slowly (taming camp).
    pub orbit: Option<f32>,
    pub mode: CameraMode,
    /// Orbit angles: yaw of the view direction (the game's convention:
    /// forward = `R_y(yaw) · −Z`), pitch down (+) in radians.
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    /// Seconds since the player last turned the camera.
    pub idle: f32,
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self {
            base_fov: 62f32.to_radians(),
            fov_gain: 28f32.to_radians(),
            fov_speed_ref: 90.0,
            shake: 0.0,
            roll: 0.0,
            orbit: None,
            mode: CameraMode::Chase,
            orbit_yaw: 0.0,
            orbit_pitch: 0.25,
            idle: 10.0,
        }
    }
}

impl ChaseCamera {
    pub fn kick(&mut self, amount: f32) {
        self.shake = (self.shake + amount).min(1.0);
    }

    /// Turn the orbit camera by the player's look input (radians).
    pub fn turn(&mut self, yaw: f32, pitch: f32) {
        if yaw != 0.0 || pitch != 0.0 {
            self.idle = 0.0;
        }
        self.orbit_yaw += yaw;
        self.orbit_pitch = (self.orbit_pitch + pitch).clamp(-0.45, 1.15);
    }

    /// The horizontal direction the orbit camera looks along.
    pub fn orbit_forward(&self) -> Vec3 {
        Quat::from_rotation_y(self.orbit_yaw) * Vec3::NEG_Z
    }
}

pub fn follow(
    time: Res<Time>,
    terrain: Option<Res<crate::world::WorldTerrain>>,
    targets: Query<(&GlobalTransform, &CameraTarget, Option<&Velocity>)>,
    mut cams: Query<(&mut Transform, &mut Projection, &mut ChaseCamera)>,
) {
    let Ok((target_tf, follow, vel)) = targets.single() else {
        return;
    };
    let Ok((mut cam_tf, mut proj, mut chase)) = cams.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(0.05);
    // Belt and braces: a non-finite value reaching the camera would stick
    // forever (the smoothing feeds on its own output) and blank the screen.
    let velocity = vel
        .map(|v| v.0)
        .filter(|v| v.is_finite())
        .unwrap_or(Vec3::ZERO);
    let speed = velocity.length();
    if !target_tf.translation().is_finite() {
        return;
    }
    if !cam_tf.translation.is_finite() || !cam_tf.rotation.is_finite() {
        *cam_tf = Transform::from_translation(target_tf.translation() + Vec3::new(0.0, 4.0, 10.0));
    }
    if let Projection::Perspective(p) = &mut *proj {
        if !p.fov.is_finite() {
            p.fov = chase.base_fov;
        }
    }
    chase.idle += dt;

    let target_pos = target_tf.translation();
    let heading = {
        let f = target_tf.forward().as_vec3();
        Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z)
    };

    if chase.mode == CameraMode::Orbit {
        // Left alone while she moves, the camera swings round behind her.
        if chase.idle > 1.2 && speed > 0.5 {
            let wanted = (-heading.x).atan2(-heading.z);
            let d = wrap(wanted - chase.orbit_yaw);
            chase.orbit_yaw += d * (1.0 - (-1.6 * dt).exp());
        }
        let dir = Quat::from_rotation_y(chase.orbit_yaw)
            * Quat::from_rotation_x(-chase.orbit_pitch)
            * Vec3::NEG_Z;
        // Over the shoulder: the pivot sits a little to her right.
        let right = Vec3::new(-dir.z, 0.0, dir.x).normalize_or(Vec3::X);
        let pivot = target_pos + Vec3::Y * follow.height + right * 0.45;
        let mut desired = pivot - dir * follow.distance;
        // Never below the ground (or the water).
        if let Some(t) = &terrain {
            let floor = t.0.surface(desired.x, desired.z) + 0.5;
            if desired.y < floor {
                desired.y = floor;
            }
        }
        let k = 1.0 - (-follow.stiffness * dt).exp();
        cam_tf.translation = cam_tf.translation.lerp(desired, k);
        chase.shake = (chase.shake - dt * 2.5).max(0.0);
        cam_tf.look_at(pivot + Vec3::Y * 0.1, Vec3::Y);
        if let Projection::Perspective(p) = &mut *proj {
            let wanted = chase.base_fov + chase.fov_gain * 0.25 * (speed / 8.0).min(1.0);
            p.fov += (wanted - p.fov) * (1.0 - (-6.0 * dt).exp());
        }
        return;
    }

    let forward = if let Some(orbit) = chase.orbit {
        let a = time.elapsed_secs() * orbit;
        Vec3::new(a.sin(), 0.0, a.cos())
    } else {
        heading
    };

    // Desired camera position behind and above the target. At speed the
    // camera sits a little lower and closer, which makes the ground rush.
    let speed_t = (speed / chase.fov_speed_ref).clamp(0.0, 1.5);
    let distance = follow.distance * (1.0 - 0.18 * speed_t);
    let height = follow.height * (1.0 - 0.25 * speed_t);
    let desired = target_pos - forward * distance + Vec3::Y * height;

    // Critically damped-ish exponential smoothing: frame-rate independent.
    let k = 1.0 - (-follow.stiffness * dt).exp();
    cam_tf.translation = cam_tf.translation.lerp(desired, k);

    // Camera shake: decaying noise.
    chase.shake = (chase.shake - dt * 2.5).max(0.0);
    let t = time.elapsed_secs() * 47.0;
    let shake =
        Vec3::new((t).sin() * 0.7, (t * 1.3).cos(), (t * 0.9).sin() * 0.5) * chase.shake * 0.22;
    cam_tf.translation += shake;

    // Look slightly ahead of the target along its velocity.
    let look_at = target_pos + velocity * follow.look_ahead + Vec3::Y * 0.8;
    let up = Quat::from_axis_angle(forward, chase.roll) * Vec3::Y;
    cam_tf.look_at(look_at, up);

    // FOV widens with speed.
    if let Projection::Perspective(p) = &mut *proj {
        let wanted = chase.base_fov + chase.fov_gain * speed_t.min(1.0);
        p.fov += (wanted - p.fov) * (1.0 - (-6.0 * dt).exp());
    }
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

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, follow.in_set(crate::GameSet::Camera));
    }
}
