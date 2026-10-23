//! The chase camera. Speed sensation lives mostly here: the camera lags
//! behind on acceleration, pulls in and widens its field of view at high
//! speed, rolls into turns and shakes on boost.

use bevy::prelude::*;

use crate::components::{CameraTarget, Velocity};

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
        }
    }
}

impl ChaseCamera {
    pub fn kick(&mut self, amount: f32) {
        self.shake = (self.shake + amount).min(1.0);
    }
}

pub fn follow(
    time: Res<Time>,
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
    let velocity = vel.map(|v| v.0).unwrap_or(Vec3::ZERO);
    let speed = velocity.length();

    let target_pos = target_tf.translation();
    let forward = if let Some(orbit) = chase.orbit {
        let a = time.elapsed_secs() * orbit;
        Vec3::new(a.sin(), 0.0, a.cos())
    } else {
        let f = target_tf.forward().as_vec3();
        Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z)
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

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, follow.in_set(crate::GameSet::Camera));
    }
}
