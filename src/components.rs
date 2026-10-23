//! Components shared by more than one phase.

use bevy::prelude::*;

/// Anything tagged with this is despawned when the current phase exits.
#[derive(Component)]
pub struct PhaseScoped;

/// Judith — the player on foot (capture) or in the saddle (racing).
#[derive(Component)]
pub struct Judith;

/// The giant mosquito. One exists per phase; its appearance is built by
/// `creature::spawn_ephemera` and its behaviour by the phase's own systems.
#[derive(Component)]
pub struct Ephemera;

/// Simple kinematic velocity, integrated by `integrate_velocity`.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct Velocity(pub Vec3);

/// A thing the chase camera should follow.
#[derive(Component)]
pub struct CameraTarget {
    /// Distance behind the target.
    pub distance: f32,
    /// Height above the target.
    pub height: f32,
    /// How quickly the camera catches up (higher = snappier).
    pub stiffness: f32,
    /// Extra look-ahead along the target's velocity, in seconds.
    pub look_ahead: f32,
}

impl CameraTarget {
    pub const ON_FOOT: Self = Self {
        distance: 9.0,
        height: 5.0,
        stiffness: 6.0,
        look_ahead: 0.0,
    };
    pub const CAMP: Self = Self {
        distance: 12.0,
        height: 4.5,
        stiffness: 2.5,
        look_ahead: 0.0,
    };
    pub const FLIGHT: Self = Self {
        distance: 9.5,
        height: 3.2,
        stiffness: 9.0,
        look_ahead: 0.25,
    };
}

/// Judith, built from primitives with real joints: hips, spine, head,
/// shoulders, elbows, hip joints and knees are pivot entities, so the same
/// pose code (`rig.rs`) animates her as animates a rigged model.
pub fn spawn_judith(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
) -> Entity {
    let e = spawn_figure(
        commands,
        meshes,
        materials,
        at,
        Color::srgb(0.78, 0.52, 0.30),
        true,
    );
    commands.entity(e).insert((Judith, Name::new("Judith")));
    e
}

/// A rider of the tribe, dressed in `cloth`.
pub fn spawn_rider(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    cloth: Color,
) -> Entity {
    let e = spawn_figure(commands, meshes, materials, Vec3::ZERO, cloth, false);
    commands.entity(e).insert((
        crate::models::Swappable::new(crate::models::ModelKind::Villager),
        Name::new("Rider"),
    ));
    e
}

/// Someone of the tribe at home in a settlement, dressed in `cloth`.
pub fn spawn_villager(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    cloth: Color,
    variant: u32,
) -> Entity {
    let e = spawn_figure(commands, meshes, materials, Vec3::ZERO, cloth, false);
    commands.entity(e).remove::<PhaseScoped>().insert((
        crate::models::Swappable::new(crate::models::ModelKind::Villager).variant(variant),
        Name::new("Villager"),
    ));
    e
}

fn spawn_figure(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
    cloth_color: Color,
    net: bool,
) -> Entity {
    use crate::rig::human::*;
    let cloth = materials.add(StandardMaterial {
        base_color: cloth_color,
        perceptual_roughness: 0.9,
        ..default()
    });
    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.86, 0.70, 0.58),
        perceptual_roughness: 0.8,
        ..default()
    });
    let hair = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.08, 0.06),
        ..default()
    });
    let limb = meshes.add(Capsule3d::new(0.07, 0.3));
    let thigh = meshes.add(Capsule3d::new(0.09, 0.32));
    let shin = meshes.add(Capsule3d::new(0.075, 0.32));

    let root = commands
        .spawn((
            PhaseScoped,
            crate::models::Swappable::new(crate::models::ModelKind::Judith),
            Transform::from_translation(at),
            Visibility::default(),
            Velocity::default(),
            crate::rig::Gait::default(),
        ))
        .id();

    // Pivot positions relative to their parent pivot (arms down), 1.9 m tall.
    let base: [Vec3; COUNT] = [
        Vec3::new(0.0, 0.98, 0.0),   // hips
        Vec3::new(0.0, 0.22, 0.0),   // spine
        Vec3::new(0.0, 0.4, 0.0),    // head
        Vec3::new(0.24, 0.32, 0.0),  // shoulder +X
        Vec3::new(0.0, -0.3, 0.0),   // elbow +X
        Vec3::new(-0.24, 0.32, 0.0), // shoulder −X
        Vec3::new(0.0, -0.3, 0.0),   // elbow −X
        Vec3::new(0.1, -0.04, 0.0),  // hip +X
        Vec3::new(0.0, -0.46, 0.0),  // knee +X
        Vec3::new(-0.1, -0.04, 0.0), // hip −X
        Vec3::new(0.0, -0.46, 0.0),  // knee −X
    ];
    let mut joints: Vec<Entity> = Vec::with_capacity(COUNT);
    for (i, b) in base.iter().enumerate() {
        let parent = PARENTS[i].map(|p| joints[p]).unwrap_or(root);
        let j = commands
            .spawn((
                Transform::from_translation(*b),
                Visibility::default(),
                ChildOf(parent),
            ))
            .id();
        joints.push(j);
    }
    let part = |commands: &mut Commands,
                parent: Entity,
                mesh: Handle<Mesh>,
                mat: Handle<StandardMaterial>,
                t: Transform| {
        commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), t, ChildOf(parent)));
    };
    // Pelvis and torso.
    part(
        commands,
        joints[HIPS],
        meshes.add(Capsule3d::new(0.2, 0.12)),
        cloth.clone(),
        Transform::from_xyz(0.0, 0.04, 0.0),
    );
    part(
        commands,
        joints[SPINE],
        meshes.add(Capsule3d::new(0.22, 0.32)),
        cloth.clone(),
        Transform::from_xyz(0.0, 0.12, 0.0),
    );
    // Head and hair.
    part(
        commands,
        joints[HEAD],
        meshes.add(Sphere::new(0.15).mesh().ico(3).unwrap()),
        skin.clone(),
        Transform::from_xyz(0.0, 0.14, 0.0),
    );
    part(
        commands,
        joints[HEAD],
        meshes.add(Sphere::new(0.1).mesh().ico(2).unwrap()),
        hair.clone(),
        Transform::from_xyz(0.0, 0.27, 0.06),
    );
    // Arms: upper arm under the shoulder, forearm under the elbow.
    for i in 0..2 {
        part(
            commands,
            joints[SHOULDER[i]],
            limb.clone(),
            cloth.clone(),
            Transform::from_xyz(0.0, -0.15, 0.0),
        );
        part(
            commands,
            joints[ELBOW[i]],
            limb.clone(),
            skin.clone(),
            Transform::from_xyz(0.0, -0.15, 0.0),
        );
        part(
            commands,
            joints[HIP[i]],
            thigh.clone(),
            cloth.clone(),
            Transform::from_xyz(0.0, -0.23, 0.0),
        );
        part(
            commands,
            joints[KNEE[i]],
            shin.clone(),
            skin.clone(),
            Transform::from_xyz(0.0, -0.24, 0.0),
        );
    }
    if net {
        // The net on her back.
        part(
            commands,
            joints[SPINE],
            meshes.add(Cylinder::new(0.03, 1.6)),
            cloth,
            Transform::from_xyz(0.18, 0.15, 0.22).with_rotation(Quat::from_rotation_x(0.35)),
        );
    }
    let rig = crate::rig::primitive_rig(
        crate::rig::RigKind::Humanoid,
        joints,
        base.to_vec(),
        1.9,
        root,
    );
    commands
        .entity(root)
        .insert((crate::anim::Hips(rig.joints[HIPS]), rig));
    root
}
