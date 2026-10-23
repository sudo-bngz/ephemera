//! Rigging and animation for static models, done by the game itself.
//!
//! A model from Meshy (or anywhere) is one static mesh. Meshy only rigs
//! humanoids and quadrupeds, and an Ephemera is neither, so the game rigs
//! models itself when they load: it reads the mesh's shape, places a
//! skeleton, weights every vertex to the bones by where it sits, and turns
//! the mesh into a GPU-skinned mesh. Then the same pose code drives the
//! skeletons of the models and the limbs of the procedural placeholders.
//!
//! - **Wings rig** (the Ephemera): body, left/right wing, left/right legs,
//!   abdomen, head. Wings are the wide, thin parts above the body's middle;
//!   legs hang below it.
//! - **Humanoid rig** (Judith, the riders): hips, spine, head, shoulders,
//!   elbows, hip joints, knees. Meshy characters come in a T-pose, where the
//!   arms are cleanly apart from the body; an A-pose works too.
//!
//! Everything is planned in *character space*: Y up, forward −Z, X across.
//! Where a model's mesh sits in a different frame (node rotations, the fit's
//! yaw), positions are rotated into character space for planning, and bone
//! rotations are rotated back.

use bevy::math::{Affine3A, Mat4, Quat, Vec3};

/// Which skeleton a model gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RigKind {
    Wings,
    Humanoid,
}

/// Bones of the wings rig, in order.
pub mod wing {
    pub const BODY: usize = 0;
    pub const WING_L: usize = 1;
    pub const WING_R: usize = 2;
    pub const LEGS_L: usize = 3;
    pub const LEGS_R: usize = 4;
    pub const ABDOMEN: usize = 5;
    pub const HEAD: usize = 6;
    pub const COUNT: usize = 7;
    pub const PARENTS: [Option<usize>; COUNT] = [
        None,
        Some(BODY),
        Some(BODY),
        Some(BODY),
        Some(BODY),
        Some(BODY),
        Some(BODY),
    ];
}

/// Bones of the humanoid rig, in order. Index 0 of each pair is the +X
/// side of the character.
pub mod human {
    pub const HIPS: usize = 0;
    pub const SPINE: usize = 1;
    pub const HEAD: usize = 2;
    pub const SHOULDER: [usize; 2] = [3, 5];
    pub const ELBOW: [usize; 2] = [4, 6];
    pub const HIP: [usize; 2] = [7, 9];
    pub const KNEE: [usize; 2] = [8, 10];
    pub const COUNT: usize = 11;
    pub const PARENTS: [Option<usize>; COUNT] = [
        None,
        Some(HIPS),
        Some(SPINE),
        Some(SPINE),
        Some(3),
        Some(SPINE),
        Some(5),
        Some(HIPS),
        Some(7),
        Some(HIPS),
        Some(9),
    ];
}

/// Tunable thresholds (fractions of the model's size), from `models.cfg`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigTuning {
    /// Wings start this far from the centre line, as a fraction of the
    /// half span.
    pub wing_hinge: f32,
    /// Wings are above this height, as a fraction of the model's height.
    pub wing_min_y: f32,
    /// Legs are below this height.
    pub leg_max_y: f32,
}

impl Default for RigTuning {
    fn default() -> Self {
        Self {
            wing_hinge: 0.2,
            wing_min_y: 0.45,
            leg_max_y: 0.3,
        }
    }
}

/// A skeleton and skin weights for one model.
#[derive(Clone, Debug)]
pub struct RigPlan {
    pub kind: RigKind,
    /// Bone positions in character space.
    pub bones: Vec<Vec3>,
    pub parents: Vec<Option<usize>>,
    /// Per vertex: up to four bones and their weights (summing to 1).
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    /// Rest orientation of each bone relative to the pose code's rest
    /// (arms down): a T-pose's shoulders and elbows are raised by ±90°.
    pub rest: Vec<Quat>,
    /// The model's height in character space (for bob amplitudes).
    pub height: f32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn bounds(points: &[Vec3]) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for p in points {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    (lo, hi)
}

/// Keeps the four strongest influences and normalises them.
fn pack(influences: &mut [(usize, f32)]) -> ([u16; 4], [f32; 4]) {
    influences.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut j = [0u16; 4];
    let mut w = [0f32; 4];
    let mut total = 0.0;
    for (k, (bone, weight)) in influences.iter().take(4).enumerate() {
        if *weight > 0.0 {
            j[k] = *bone as u16;
            w[k] = *weight;
            total += weight;
        }
    }
    if total <= 0.0 {
        return ([0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
    }
    for v in &mut w {
        *v /= total;
    }
    (j, w)
}

/// The wings rig, from vertex positions already in character space.
pub fn plan_wings(points: &[Vec3], tune: RigTuning) -> RigPlan {
    use wing::*;
    let (lo, hi) = bounds(points);
    let size = (hi - lo).max(Vec3::splat(1e-4));
    let c = (lo + hi) * 0.5;
    let half_span = size.x * 0.5;
    let hinge = tune.wing_hinge * half_span;
    let wing_y = lo.y + tune.wing_min_y * size.y;
    let leg_y = lo.y + tune.leg_max_y * size.y;
    let blend_x = 0.08 * half_span;
    let blend_y = 0.05 * size.y;

    // Where the wings meet the body: the average height of wing vertices
    // near the hinge (fallback: the wing threshold).
    let roots: Vec<f32> = points
        .iter()
        .filter(|p| {
            let ax = (p.x - c.x).abs();
            ax > hinge && ax < hinge * 1.6 + blend_x && p.y > wing_y
        })
        .map(|p| p.y)
        .collect();
    let root_y = if roots.is_empty() {
        wing_y
    } else {
        roots.iter().sum::<f32>() / roots.len() as f32
    };

    let mut bones = vec![Vec3::ZERO; COUNT];
    bones[BODY] = Vec3::new(c.x, (wing_y + leg_y) * 0.5, c.z);
    bones[WING_L] = Vec3::new(c.x + hinge, root_y, c.z);
    bones[WING_R] = Vec3::new(c.x - hinge, root_y, c.z);
    bones[LEGS_L] = Vec3::new(c.x + 0.05 * half_span, leg_y, c.z);
    bones[LEGS_R] = Vec3::new(c.x - 0.05 * half_span, leg_y, c.z);
    bones[ABDOMEN] = Vec3::new(c.x, bones[BODY].y, c.z + 0.12 * size.z);
    bones[HEAD] = Vec3::new(c.x, bones[BODY].y, c.z - 0.28 * size.z);

    let mut joints = Vec::with_capacity(points.len());
    let mut weights = Vec::with_capacity(points.len());
    for p in points {
        let dx = p.x - c.x;
        let dz = p.z - c.z;
        let ax = dx.abs();
        let w_wing = smoothstep(hinge - blend_x, hinge + blend_x, ax)
            * smoothstep(wing_y - blend_y, wing_y + blend_y, p.y);
        let w_leg = (1.0 - w_wing) * smoothstep(leg_y + blend_y, leg_y - blend_y, p.y);
        let left = smoothstep(-0.02 * half_span, 0.02 * half_span, dx);
        let rest = (1.0 - w_wing - w_leg).max(0.0);
        let w_abd = rest * smoothstep(0.08 * size.z, 0.25 * size.z, dz);
        let w_head = rest * smoothstep(-0.25 * size.z, -0.4 * size.z, dz);
        let w_body = (rest - w_abd - w_head).max(0.0);
        let mut inf = vec![
            (BODY, w_body),
            (WING_L, w_wing * left),
            (WING_R, w_wing * (1.0 - left)),
            (LEGS_L, w_leg * left),
            (LEGS_R, w_leg * (1.0 - left)),
            (ABDOMEN, w_abd),
            (HEAD, w_head),
        ];
        let (j, w) = pack(&mut inf);
        joints.push(j);
        weights.push(w);
    }
    RigPlan {
        kind: RigKind::Wings,
        bones,
        parents: PARENTS.to_vec(),
        joints,
        weights,
        rest: vec![Quat::IDENTITY; COUNT],
        height: size.y,
    }
}

/// The humanoid rig, from vertex positions already in character space,
/// feet at the bottom.
pub fn plan_humanoid(points: &[Vec3]) -> RigPlan {
    use human::*;
    let (lo, hi) = bounds(points);
    let size = (hi - lo).max(Vec3::splat(1e-4));
    let h = size.y;
    let cx = (lo.x + hi.x) * 0.5;
    let cz = (lo.z + hi.z) * 0.5;
    let norm = |p: &Vec3| ((p.y - lo.y) / h, (p.x - cx) / h);

    // A T-pose has vertices far out at shoulder height.
    let t_pose = points.iter().any(|p| {
        let (y, x) = norm(p);
        x.abs() > 0.32 && (0.62..0.95).contains(&y)
    });
    let arm_y = if t_pose {
        let ys: Vec<f32> = points
            .iter()
            .filter(|p| {
                let (y, x) = norm(p);
                x.abs() > 0.25 && (0.6..0.95).contains(&y)
            })
            .map(|p| (p.y - lo.y) / h)
            .collect();
        ys.iter().sum::<f32>() / ys.len().max(1) as f32
    } else {
        0.80
    };
    let sh = 0.12;
    let el = if t_pose { 0.30 } else { sh };

    let at = |x: f32, y: f32| Vec3::new(cx + x * h, lo.y + y * h, cz);
    let mut bones = vec![Vec3::ZERO; COUNT];
    bones[HIPS] = at(0.0, 0.52);
    bones[SPINE] = at(0.0, 0.64);
    bones[HEAD] = at(0.0, 0.84);
    for (i, s) in [(0usize, 1.0f32), (1, -1.0)] {
        bones[SHOULDER[i]] = at(s * sh, arm_y);
        bones[ELBOW[i]] = if t_pose {
            at(s * el, arm_y)
        } else {
            at(s * (sh + 0.01), arm_y - 0.18)
        };
        bones[HIP[i]] = at(s * 0.07, 0.50);
        bones[KNEE[i]] = at(s * 0.08, 0.27);
    }

    let b = 0.025;
    let mut joints = Vec::with_capacity(points.len());
    let mut weights = Vec::with_capacity(points.len());
    for p in points {
        let (y, x) = norm(p);
        let ax = x.abs();
        let side = smoothstep(-0.01, 0.01, x); // 1 on the +X side
        let (arm, fore) = if t_pose {
            let arm = smoothstep(sh - b, sh + b, ax) * smoothstep(arm_y - 0.16, arm_y - 0.12, y);
            (arm, arm * smoothstep(el - b, el + b, ax))
        } else {
            // Arms at the sides: outside the torso, between hip and shoulder.
            let arm =
                smoothstep(0.11, 0.14, ax) * smoothstep(0.40, 0.46, y) * smoothstep(0.86, 0.80, y);
            (arm, arm * smoothstep(arm_y - 0.16, arm_y - 0.22, y))
        };
        let upper = (arm - fore).max(0.0);
        let leg = (1.0 - arm) * smoothstep(0.50 + b, 0.50 - b, y);
        let shin = leg * smoothstep(0.27 + b, 0.27 - b, y);
        let thigh = (leg - shin).max(0.0);
        let rest = (1.0 - arm - leg).max(0.0);
        let head = rest * smoothstep(0.80 - b, 0.80 + b, y);
        let spine = (rest - head).max(0.0) * smoothstep(0.55, 0.66, y);
        let hips = (rest - head - spine).max(0.0);
        let mut inf = vec![
            (HIPS, hips),
            (SPINE, spine),
            (HEAD, head),
            (SHOULDER[0], upper * side),
            (ELBOW[0], fore * side),
            (SHOULDER[1], upper * (1.0 - side)),
            (ELBOW[1], fore * (1.0 - side)),
            (HIP[0], thigh * side),
            (KNEE[0], shin * side),
            (HIP[1], thigh * (1.0 - side)),
            (KNEE[1], shin * (1.0 - side)),
        ];
        let (j, w) = pack(&mut inf);
        joints.push(j);
        weights.push(w);
    }

    // Rest orientations relative to arms-down: a T-pose raises the arms.
    let mut rest = vec![Quat::IDENTITY; COUNT];
    if t_pose {
        for (i, s) in [(0usize, 1.0f32), (1, -1.0)] {
            let up = Quat::from_rotation_z(s * std::f32::consts::FRAC_PI_2);
            rest[SHOULDER[i]] = up;
            rest[ELBOW[i]] = up;
        }
    }
    RigPlan {
        kind: RigKind::Humanoid,
        bones,
        parents: PARENTS.to_vec(),
        joints,
        weights,
        rest,
        height: h,
    }
}

// ------------------------------------------------------------- poses --

/// A humanoid pose, in words rather than quaternions, so one pose drives
/// any rig whatever its rest pose. Angles in radians; index 0 of each pair
/// is the +X side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    /// Hips lowered, as a fraction of the height.
    pub drop: f32,
    /// Leaning forward (+) / back (−).
    pub spine_pitch: f32,
    /// Leaning to the +X side (+).
    pub spine_roll: f32,
    pub spine_yaw: f32,
    /// Looking down (+).
    pub head_pitch: f32,
    pub head_yaw: f32,
    /// Arm swung forward (+).
    pub shoulder_flex: [f32; 2],
    /// Arm raised out to the side (0 = down, π/2 = T-pose).
    pub shoulder_abduct: [f32; 2],
    /// Forearm bent forward (+).
    pub elbow: [f32; 2],
    /// Thigh swung forward (+).
    pub hip_flex: [f32; 2],
    /// Thigh out to the side (+).
    pub hip_abduct: [f32; 2],
    /// Shin bent back (+).
    pub knee: [f32; 2],
}

impl Pose {
    pub fn lerp(&self, o: &Pose, t: f32) -> Pose {
        let l = |a: f32, b: f32| a + (b - a) * t;
        let l2 = |a: [f32; 2], b: [f32; 2]| [l(a[0], b[0]), l(a[1], b[1])];
        Pose {
            drop: l(self.drop, o.drop),
            spine_pitch: l(self.spine_pitch, o.spine_pitch),
            spine_roll: l(self.spine_roll, o.spine_roll),
            spine_yaw: l(self.spine_yaw, o.spine_yaw),
            head_pitch: l(self.head_pitch, o.head_pitch),
            head_yaw: l(self.head_yaw, o.head_yaw),
            shoulder_flex: l2(self.shoulder_flex, o.shoulder_flex),
            shoulder_abduct: l2(self.shoulder_abduct, o.shoulder_abduct),
            elbow: l2(self.elbow, o.elbow),
            hip_flex: l2(self.hip_flex, o.hip_flex),
            hip_abduct: l2(self.hip_abduct, o.hip_abduct),
            knee: l2(self.knee, o.knee),
        }
    }

    /// Standing, breathing.
    pub fn idle(t: f32) -> Pose {
        let breath = (t * 1.6).sin();
        Pose {
            spine_pitch: 0.02 * breath,
            head_yaw: 0.25 * (t * 0.23).sin() * (t * 0.11).cos(),
            head_pitch: 0.05 * (t * 0.31).sin(),
            shoulder_abduct: [0.08 + 0.02 * breath; 2],
            elbow: [0.12; 2],
            ..Default::default()
        }
    }

    /// A walk (amount 0–1) or a run (above 1), at gait phase `phase`.
    pub fn walk(phase: f32, amount: f32) -> Pose {
        let s = phase.sin();
        let c = phase.cos();
        let a = amount.clamp(0.0, 1.6);
        // The swing leg bends as it comes through; the elbow follows the arm.
        let bend = |x: f32| (x * 0.5 + 0.5).powi(2);
        Pose {
            drop: 0.012 * a * (2.0 * phase).cos().abs(),
            spine_pitch: 0.05 * a + 0.03 * a * (2.0 * phase).sin(),
            spine_roll: 0.04 * a * c,
            spine_yaw: 0.1 * a * s,
            head_yaw: -0.05 * a * s,
            head_pitch: 0.02 * a * (2.0 * phase).sin(),
            shoulder_flex: [-0.4 * a * s, 0.4 * a * s],
            shoulder_abduct: [0.1 + 0.03 * a * c, 0.1 - 0.03 * a * c],
            elbow: [0.2 + 0.4 * a * bend(-s), 0.2 + 0.4 * a * bend(s)],
            hip_flex: [0.45 * a * s, -0.45 * a * s],
            hip_abduct: [0.03 * a * c, -0.03 * a * c],
            knee: [
                0.1 + 1.0 * a * bend(-s) * (0.3 + 0.7 * (-c).max(0.0)),
                0.1 + 1.0 * a * bend(s) * (0.3 + 0.7 * c.max(0.0)),
            ],
        }
    }

    /// Crouched low, creeping.
    pub fn crouch(phase: f32, moving: f32) -> Pose {
        let s = phase.sin() * moving;
        Pose {
            drop: 0.17,
            spine_pitch: 0.45,
            head_pitch: -0.35,
            shoulder_flex: [0.35 - 0.2 * s, 0.35 + 0.2 * s],
            shoulder_abduct: [0.15; 2],
            elbow: [0.9; 2],
            hip_flex: [1.05 + 0.25 * s, 1.05 - 0.25 * s],
            knee: [1.7 - 0.2 * s, 1.7 + 0.2 * s],
            ..Default::default()
        }
    }

    /// Astride a flying Ephemera: legs gripping, leaning into the bank.
    pub fn ride(roll: f32, bob: f32, speed: f32) -> Pose {
        Pose {
            drop: 0.24,
            spine_pitch: 0.2 + 0.25 * speed + 0.04 * bob,
            spine_roll: -roll * 0.6,
            head_pitch: -0.25 - 0.2 * speed,
            head_yaw: roll * 0.3,
            shoulder_flex: [0.95 + 0.2 * speed; 2],
            shoulder_abduct: [0.25; 2],
            elbow: [0.7 - 0.2 * speed; 2],
            hip_flex: [1.25; 2],
            hip_abduct: [0.5, 0.5],
            knee: [1.4; 2],
            spine_yaw: 0.0,
        }
    }

    /// In the air: knees up, arms out.
    pub fn jump(rise: f32) -> Pose {
        Pose {
            spine_pitch: 0.12,
            head_pitch: -0.1,
            shoulder_flex: [-0.3 - 0.3 * rise; 2],
            shoulder_abduct: [0.6 + 0.3 * rise; 2],
            elbow: [0.5; 2],
            hip_flex: [0.7 + 0.3 * rise, 0.35],
            knee: [1.1, 0.6],
            ..Default::default()
        }
    }

    /// Swimming a crawl at the surface: body flat, legs trailing and
    /// kicking, arms stroking in turn.
    pub fn swim(phase: f32, moving: f32) -> Pose {
        let s = phase.sin();
        let c = phase.cos();
        Pose {
            spine_pitch: 1.3,
            head_pitch: -1.0,
            shoulder_flex: [1.6 + 1.2 * s * moving, 1.6 - 1.2 * s * moving],
            shoulder_abduct: [0.35; 2],
            elbow: [0.4 + 0.3 * c.abs(); 2],
            hip_flex: [-1.15 + 0.25 * s * moving, -1.15 - 0.25 * s * moving],
            knee: [0.35 + 0.3 * (-s).max(0.0), 0.35 + 0.3 * s.max(0.0)],
            ..Default::default()
        }
    }

    /// Local rotation of every humanoid bone, relative to an arms-down
    /// rest, in character space.
    pub fn local_rotations(&self) -> [Quat; human::COUNT] {
        use human::*;
        let mut q = [Quat::IDENTITY; COUNT];
        q[SPINE] = Quat::from_rotation_y(self.spine_yaw)
            * Quat::from_rotation_z(-self.spine_roll)
            * Quat::from_rotation_x(-self.spine_pitch);
        q[HEAD] = Quat::from_rotation_y(self.head_yaw) * Quat::from_rotation_x(-self.head_pitch);
        for i in 0..2 {
            let s = if i == 0 { 1.0 } else { -1.0 };
            q[SHOULDER[i]] = Quat::from_rotation_x(self.shoulder_flex[i])
                * Quat::from_rotation_z(s * self.shoulder_abduct[i]);
            q[ELBOW[i]] = Quat::from_rotation_x(self.elbow[i]);
            q[HIP[i]] = Quat::from_rotation_x(self.hip_flex[i])
                * Quat::from_rotation_z(s * self.hip_abduct[i]);
            q[KNEE[i]] = Quat::from_rotation_x(-self.knee[i]);
        }
        q
    }
}

/// The wings rig's local rotations from the flap (phase, 0–1 intensity).
pub fn wing_rotations(phase: f32, intensity: f32, t: f32) -> [Quat; wing::COUNT] {
    use wing::*;
    let mut q = [Quat::IDENTITY; COUNT];
    let amp = 0.1 + 0.45 * intensity;
    // The downstroke is quicker than the upstroke.
    let s = phase.sin();
    let beat = (s + 0.25 * (2.0 * phase).sin()) * amp;
    // A figure-eight: the stroke sweeps a little fore and aft too.
    let sweep = phase.cos() * amp * 0.2;
    q[WING_L] = Quat::from_rotation_y(sweep) * Quat::from_rotation_z(beat);
    q[WING_R] = Quat::from_rotation_y(-sweep) * Quat::from_rotation_z(-beat);
    // Legs dangle at rest and draw up a little in flight.
    let twitch = (t * 3.1).sin() * 0.06 * (1.0 - intensity);
    q[LEGS_L] = Quat::from_rotation_x(-0.45 * intensity + twitch) * Quat::from_rotation_z(0.05);
    q[LEGS_R] = Quat::from_rotation_x(-0.45 * intensity - twitch) * Quat::from_rotation_z(-0.05);
    q[ABDOMEN] = Quat::from_rotation_x(0.07 * (phase * 0.5).sin() - 0.1 * intensity);
    q[HEAD] = Quat::from_rotation_y(0.12 * (t * 0.7).sin() * (1.0 - intensity));
    q
}

/// A rig's local rotation for bone `i`, from a pose rotation in character
/// space: corrected for the rig's rest (Q' = Rp·Q·Ri⁻¹) and turned into the
/// skeleton's frame (`to_char` maps skeleton space to character space).
pub fn bone_rotation(
    plan_rest: &[Quat],
    parents: &[Option<usize>],
    i: usize,
    q: Quat,
    to_char: Quat,
) -> Quat {
    let rp = parents[i].map(|p| plan_rest[p]).unwrap_or(Quat::IDENTITY);
    let corrected = rp * q * plan_rest[i].inverse();
    to_char.inverse() * corrected * to_char
}

/// Inverse bind matrix of a bone at `pos` (skeleton space) for a mesh
/// whose vertices map into skeleton space by `mesh_to_skel`.
pub fn inverse_bind(pos: Vec3, mesh_to_skel: Affine3A) -> Mat4 {
    Mat4::from_translation(-pos) * Mat4::from(mesh_to_skel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mosquito-ish point cloud: a body along Z, two flat wings out to
    /// the sides above it, legs hanging below.
    fn mosquito() -> Vec<Vec3> {
        let mut p = Vec::new();
        for i in 0..40 {
            let z = -3.0 + i as f32 * 0.15;
            for a in 0..8 {
                let t = a as f32 / 8.0 * std::f32::consts::TAU;
                p.push(Vec3::new(t.cos() * 0.4, 1.5 + t.sin() * 0.4, z));
            }
        }
        for i in 0..20 {
            for j in 0..6 {
                let x = 0.9 + i as f32 * 0.18;
                let z = -0.8 + j as f32 * 0.3;
                p.push(Vec3::new(x, 1.95, z));
                p.push(Vec3::new(-x, 1.95, z));
            }
        }
        for i in 0..10 {
            let y = 1.0 - i as f32 * 0.1;
            p.push(Vec3::new(0.3 + i as f32 * 0.05, y, 0.0));
            p.push(Vec3::new(-0.3 - i as f32 * 0.05, y, 0.0));
        }
        p
    }

    fn main_bone(plan: &RigPlan, v: usize) -> usize {
        plan.joints[v][0] as usize
    }

    #[test]
    fn wings_are_found() {
        let pts = mosquito();
        let plan = plan_wings(&pts, RigTuning::default());
        for (v, p) in pts.iter().enumerate() {
            let w: f32 = plan.weights[v].iter().sum();
            assert!((w - 1.0).abs() < 1e-4, "weights must sum to 1");
            if p.y > 1.9 && p.x > 1.5 {
                assert_eq!(main_bone(&plan, v), wing::WING_L, "wing vertex at {p}");
            }
            if p.y > 1.9 && p.x < -1.5 {
                assert_eq!(main_bone(&plan, v), wing::WING_R, "wing vertex at {p}");
            }
            if p.y < 0.6 {
                let b = main_bone(&plan, v);
                assert!(
                    b == wing::LEGS_L || b == wing::LEGS_R,
                    "leg vertex at {p} → {b}"
                );
            }
            if p.x.abs() < 0.45 && p.y > 1.2 && p.z.abs() < 0.3 {
                assert_eq!(main_bone(&plan, v), wing::BODY, "body vertex at {p}");
            }
        }
        // The wing hinge sits at the body's side, at wing height.
        let hinge = plan.bones[wing::WING_L];
        assert!(hinge.x > 0.3 && hinge.x < 1.2, "hinge x {}", hinge.x);
        assert!(hinge.y > 1.5, "hinge y {}", hinge.y);
    }

    /// A T-posed figure, 1.8 tall: torso box, head, arms out, legs apart.
    fn t_pose() -> Vec<Vec3> {
        let mut p = Vec::new();
        let mut boxy = |c: Vec3, s: Vec3, n: usize| {
            for i in 0..n {
                for j in 0..n {
                    for k in 0..n {
                        let f = |a| a as f32 / (n - 1) as f32 - 0.5;
                        p.push(c + s * Vec3::new(f(i), f(j), f(k)));
                    }
                }
            }
        };
        boxy(Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.36, 0.55, 0.2), 6); // torso
        boxy(Vec3::new(0.0, 1.66, 0.0), Vec3::new(0.2, 0.25, 0.2), 5); // head
        boxy(Vec3::new(0.52, 1.42, 0.0), Vec3::new(0.7, 0.1, 0.1), 8); // +X arm
        boxy(Vec3::new(-0.52, 1.42, 0.0), Vec3::new(0.7, 0.1, 0.1), 8); // −X arm
        boxy(Vec3::new(0.11, 0.45, 0.0), Vec3::new(0.14, 0.9, 0.14), 7); // +X leg
        boxy(Vec3::new(-0.11, 0.45, 0.0), Vec3::new(0.14, 0.9, 0.14), 7); // −X leg
        p
    }

    #[test]
    fn humanoid_limbs_are_found() {
        use human::*;
        let pts = t_pose();
        let plan = plan_humanoid(&pts);
        assert_ne!(
            plan.rest[SHOULDER[0]],
            Quat::IDENTITY,
            "a T-pose was detected"
        );
        for (v, p) in pts.iter().enumerate() {
            let b = main_bone(&plan, v);
            if p.x > 0.7 && p.y > 1.3 {
                assert_eq!(b, ELBOW[0], "+X forearm at {p}");
            }
            if p.x < -0.7 && p.y > 1.3 {
                assert_eq!(b, ELBOW[1], "−X forearm at {p}");
            }
            if p.y < 0.3 && p.x > 0.05 {
                assert_eq!(b, KNEE[0], "+X shin at {p}");
            }
            if p.y < 0.3 && p.x < -0.05 {
                assert_eq!(b, KNEE[1], "−X shin at {p}");
            }
            if p.y > 1.62 && p.x.abs() < 0.08 {
                assert_eq!(b, HEAD, "head at {p}");
            }
        }
    }

    /// Correcting a T-pose: pose rotations meant for arms-down move the
    /// T-posed arm exactly where the arms-down arm would go.
    #[test]
    fn rest_correction_maps_poses_between_rests() {
        use human::*;
        let plan = plan_humanoid(&t_pose());
        let pose = Pose::walk(0.7, 1.0);
        let q = pose.local_rotations();
        // Arms-down rig: global shoulder = q[SPINE]·q[SHOULDER]; forearm
        // global = that · q[ELBOW]. A hand at the end of an arms-down arm…
        let down = Vec3::new(0.0, -1.0, 0.0);
        let g_down = q[SPINE] * q[SHOULDER[0]] * q[ELBOW[0]];
        // …and the same for the T-pose rig, whose rest arm points +X.
        let t_rest = Vec3::X;
        let r = |i| bone_rotation(&plan.rest, &plan.parents, i, q[i], Quat::IDENTITY);
        let g_t = r(SPINE) * r(SHOULDER[0]) * r(ELBOW[0]);
        assert!((g_down * down - g_t * t_rest).length() < 1e-4);
    }

    #[test]
    fn skin_is_identity_at_rest() {
        // jointMatrix = joint_global · inverse_bind; at rest (joint global
        // = translation to its position) it must leave vertices in place.
        let mesh_to_skel = Affine3A::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(1.0),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let pos = Vec3::new(0.4, 1.1, -0.2);
        let m = Mat4::from_translation(pos) * inverse_bind(pos, mesh_to_skel);
        let v = Vec3::new(0.3, -0.7, 0.9);
        assert!((m.transform_point3(v) - mesh_to_skel.transform_point3(v)).length() < 1e-5);
    }

    #[test]
    fn wings_beat() {
        let a = wing_rotations(0.3, 0.8, 0.0);
        let b = wing_rotations(1.9, 0.8, 0.0);
        assert_ne!(a[wing::WING_L], b[wing::WING_L]);
        // Mirror: the two wings go up together.
        let tip_l = a[wing::WING_L] * Vec3::X;
        let tip_r = a[wing::WING_R] * Vec3::NEG_X;
        assert!((tip_l.y - tip_r.y).abs() < 1e-5);
    }
}

// ------------------------------------------------------- in the game --

use std::collections::HashMap;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy::prelude::*;

use crate::creature::Flap;
use crate::input::InputAxes;
use crate::models::{ModelKind, SwappedModel};
use crate::state::GamePhase;

/// A skeleton in the world: the entities of its bones, and how to pose it.
#[derive(Component, Debug)]
pub struct Rig {
    pub kind: RigKind,
    pub joints: Vec<Entity>,
    /// Each bone's translation relative to its parent, at rest.
    pub base: Vec<Vec3>,
    pub rest: Vec<Quat>,
    pub parents: Vec<Option<usize>>,
    /// Skeleton space → character space.
    pub to_char: Quat,
    pub height: f32,
    /// The entity whose motion drives this rig (its `Flap` or `Gait`).
    pub driver: Entity,
}

/// What a figure on foot is doing, set by whoever moves it. Without one,
/// a figure is standing or walking (crouching when the capture says so).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stance {
    #[default]
    Stand,
    Crouch,
    /// Off the ground; the value is whether she is still rising.
    Air {
        rising: bool,
    },
    Swim,
}

/// A walking, crouching, riding figure: Judith and the tribe's riders.
#[derive(Component, Debug, Default)]
pub struct Gait {
    pub phase: f32,
    pub speed: f32,
    pub last: Option<Vec3>,
    pub pose: Pose,
}

/// Already rigged (or found not riggable, or animated by its own clips).
#[derive(Component)]
pub struct Rigged;

/// Skinned copies of models' meshes, shared by every instance of a model.
#[derive(Resource, Default)]
pub(crate) struct RigCache {
    meshes: HashMap<AssetId<Mesh>, (Handle<Mesh>, Handle<SkinnedMeshInverseBindposes>)>,
}

/// Rigs a primitive placeholder whose limbs are pivot entities, already
/// in arms-down rest, in the root's own frame.
pub fn primitive_rig(
    kind: RigKind,
    joints: Vec<Entity>,
    base: Vec<Vec3>,
    height: f32,
    driver: Entity,
) -> Rig {
    let n = joints.len();
    Rig {
        kind,
        joints,
        base,
        rest: vec![Quat::IDENTITY; n],
        parents: match kind {
            RigKind::Humanoid => human::PARENTS.to_vec(),
            RigKind::Wings => wing::PARENTS.to_vec(),
        },
        to_char: Quat::IDENTITY,
        height,
        driver,
    }
}

/// Transform from `e`'s frame up to (not including) `top`.
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

fn descendants_with_mesh(
    e: Entity,
    children: &Query<&Children>,
    has_mesh: &Query<&Mesh3d>,
    out: &mut Vec<Entity>,
) {
    if let Ok(kids) = children.get(e) {
        for k in kids.iter() {
            if has_mesh.contains(k) {
                out.push(k);
            }
            descendants_with_mesh(k, children, has_mesh, out);
        }
    }
}

/// Rigs models as soon as their scene has spawned and their meshes loaded.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rig_models(
    mut commands: Commands,
    swapped: Query<(Entity, &SwappedModel, &Transform, &ChildOf), Without<Rigged>>,
    children: Query<&Children>,
    has_mesh: Query<&Mesh3d>,
    skinned: Query<(), With<SkinnedMesh>>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut binds: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut cache: ResMut<RigCache>,
) {
    for (root, swap, tf, parent) in &swapped {
        let kind = match swap.kind {
            ModelKind::Ephemera => RigKind::Wings,
            ModelKind::Judith | ModelKind::Villager => RigKind::Humanoid,
            _ => {
                commands.entity(root).insert(Rigged);
                continue;
            }
        };
        let mut mesh_entities = Vec::new();
        descendants_with_mesh(root, &children, &has_mesh, &mut mesh_entities);
        if mesh_entities.is_empty() {
            continue; // the scene hasn't spawned yet
        }
        if mesh_entities.iter().any(|e| skinned.contains(*e)) {
            // It came with a skeleton of its own (rigged in Meshy, Blender…):
            // its clips animate it (`anim.rs`), not this rigger.
            info!(
                "{:?} model is already rigged; using its own skeleton",
                swap.kind
            );
            commands.entity(root).insert(Rigged);
            continue;
        }
        let handles: Vec<Handle<Mesh>> = mesh_entities
            .iter()
            .filter_map(|e| has_mesh.get(*e).ok().map(|m| m.0.clone()))
            .collect();
        if handles.iter().any(|h| meshes.get(h).is_none()) {
            continue; // still loading
        }

        // All vertices, in skeleton space (the swapped model's frame), then
        // in character space for planning.
        let to_char = tf.rotation;
        let frames: Vec<Affine3A> = mesh_entities
            .iter()
            .map(|e| frame_to(*e, root, &parents, &transforms))
            .collect();
        let mut points = Vec::new();
        let mut ranges = Vec::new();
        for (h, m) in handles.iter().zip(&frames) {
            let start = points.len();
            if let Some(VertexAttributeValues::Float32x3(ps)) = meshes
                .get(h)
                .and_then(|mesh| mesh.attribute(Mesh::ATTRIBUTE_POSITION))
            {
                for p in ps {
                    points.push(to_char * m.transform_point3(Vec3::from(*p)));
                }
            }
            ranges.push(start..points.len());
        }
        if points.len() < 8 {
            commands.entity(root).insert(Rigged);
            continue;
        }
        let plan = match kind {
            RigKind::Wings => plan_wings(&points, swap.rig()),
            RigKind::Humanoid => plan_humanoid(&points),
        };

        // The skeleton: bones as entities under the model, at rest.
        let from_char = to_char.inverse();
        let skel: Vec<Vec3> = plan.bones.iter().map(|b| from_char * *b).collect();
        let mut joints: Vec<Entity> = Vec::with_capacity(skel.len());
        let mut base = Vec::with_capacity(skel.len());
        for (i, p) in skel.iter().enumerate() {
            let (parent_e, parent_p) = match plan.parents[i] {
                Some(pi) => (joints[pi], skel[pi]),
                None => (root, Vec3::ZERO),
            };
            let local = *p - parent_p;
            let j = commands
                .spawn((
                    Transform::from_translation(local),
                    Visibility::default(),
                    ChildOf(parent_e),
                ))
                .id();
            joints.push(j);
            base.push(local);
        }

        for ((e, h), (m, range)) in mesh_entities
            .iter()
            .zip(&handles)
            .zip(frames.iter().zip(&ranges))
        {
            let (skinned, bind) = cache
                .meshes
                .entry(h.id())
                .or_insert_with(|| {
                    let mut mesh = meshes.get(h).cloned().expect("loaded");
                    let js: Vec<[u16; 4]> = plan.joints[range.clone()].to_vec();
                    let ws: Vec<[f32; 4]> = plan.weights[range.clone()].to_vec();
                    mesh.insert_attribute(
                        Mesh::ATTRIBUTE_JOINT_INDEX,
                        VertexAttributeValues::Uint16x4(js),
                    );
                    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, ws);
                    let inv: Vec<Mat4> = skel.iter().map(|p| inverse_bind(*p, *m)).collect();
                    (
                        meshes.add(mesh),
                        binds.add(SkinnedMeshInverseBindposes::from(inv)),
                    )
                })
                .clone();
            commands.entity(*e).insert((
                Mesh3d(skinned),
                SkinnedMesh {
                    inverse_bindposes: bind,
                    joints: joints.clone(),
                },
                // The bounds of a deforming mesh move with it.
                NoFrustumCulling,
            ));
        }
        info!(
            "rigged {:?} model: {} bones, {} vertices",
            swap.kind,
            joints.len(),
            points.len()
        );
        commands.entity(root).insert((
            Rigged,
            Rig {
                kind: plan.kind,
                joints,
                base,
                rest: plan.rest,
                parents: plan.parents,
                to_char,
                height: plan.height,
                driver: parent.parent(),
            },
        ));
    }
}

/// Decides what every figure is doing, and eases its pose towards it.
fn drive_gaits(
    time: Res<Time>,
    axes: Res<InputAxes>,
    phase: Res<State<GamePhase>>,
    mut figures: Query<(
        &GlobalTransform,
        &mut Gait,
        Option<&ChildOf>,
        Option<&Stance>,
    )>,
    mounts: Query<(&Flap, Option<&crate::racing::Rider>)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let t = time.elapsed_secs();
    for (tf, mut gait, parent, stance) in &mut figures {
        let p = tf.translation();
        let mount = parent.and_then(|c| mounts.get(c.parent()).ok());
        let target = if let Some((flap, rider)) = mount {
            let (roll, speed) = rider
                .map(|r| (r.roll(), r.speed_fraction()))
                .unwrap_or((0.0, 0.0));
            Pose::ride(roll, flap.phase.sin() * flap.intensity, speed.min(1.2))
        } else {
            let v = gait
                .last
                .map(|l| (p - l).with_y(0.0).length() / dt)
                .unwrap_or(0.0);
            gait.speed += (v - gait.speed) * (1.0 - (-8.0 * dt).exp());
            // A stride is about 1.4 m: the phase turns once every two.
            gait.phase += gait.speed * dt * std::f32::consts::TAU / 2.8;
            let stance = match stance {
                Some(s) => *s,
                None if *phase.get() == GamePhase::Capture && axes.crouch => Stance::Crouch,
                None => Stance::Stand,
            };
            match stance {
                Stance::Crouch => Pose::crouch(gait.phase, (gait.speed / 3.0).min(1.0)),
                Stance::Air { rising } => Pose::jump(if rising { 1.0 } else { 0.0 }),
                Stance::Swim => Pose::swim(gait.phase * 0.6, (gait.speed / 2.0).min(1.0)),
                Stance::Stand => {
                    let walk = (gait.speed / 4.5).min(1.6);
                    Pose::idle(t).lerp(
                        &Pose::walk(gait.phase, walk.max(0.2)),
                        (walk * 2.5).min(1.0),
                    )
                }
            }
        };
        gait.last = Some(p);
        gait.pose = gait.pose.lerp(&target, 1.0 - (-7.0 * dt).exp());
    }
}

/// Puts every rig's bones where its driver's pose or flap wants them.
fn animate_rigs(
    time: Res<Time>,
    rigs: Query<&Rig>,
    flaps: Query<&Flap>,
    gaits: Query<&Gait>,
    mut bones: Query<&mut Transform>,
) {
    let t = time.elapsed_secs();
    for rig in &rigs {
        let (rot, lift): (Vec<Quat>, f32) = match rig.kind {
            RigKind::Wings => {
                let Ok(flap) = flaps.get(rig.driver) else {
                    continue;
                };
                let q = wing_rotations(flap.phase, flap.intensity, t);
                let bob =
                    (t * (1.5 + 4.0 * flap.intensity)).sin() * (0.01 + 0.025 * flap.intensity);
                (q.to_vec(), bob)
            }
            RigKind::Humanoid => {
                let Ok(gait) = gaits.get(rig.driver) else {
                    continue;
                };
                (gait.pose.local_rotations().to_vec(), -gait.pose.drop)
            }
        };
        for (i, j) in rig.joints.iter().enumerate() {
            let Ok(mut tf) = bones.get_mut(*j) else {
                continue;
            };
            tf.rotation = bone_rotation(&rig.rest, &rig.parents, i, rot[i], rig.to_char);
            tf.translation = rig.base[i];
            if rig.parents[i].is_none() {
                tf.translation += rig.to_char.inverse() * Vec3::Y * (lift * rig.height);
            }
        }
    }
}

pub struct RigPlugin;

impl Plugin for RigPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RigCache>()
            .add_systems(Update, rig_models.in_set(crate::GameSet::Simulation))
            .add_systems(
                Update,
                (drive_gaits, animate_rigs)
                    .chain()
                    .after(crate::camera::follow)
                    .in_set(crate::GameSet::Camera),
            );
    }
}
