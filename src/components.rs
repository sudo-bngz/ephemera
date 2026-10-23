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

/// Helper to spawn a simple capsule-and-sphere humanoid for Judith.
pub fn spawn_judith(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
) -> Entity {
    let cloth = materials.add(StandardMaterial {
        base_color: Color::srgb(0.78, 0.52, 0.30),
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

    commands
        .spawn((
            Judith,
            PhaseScoped,
            crate::models::Swappable::new(crate::models::ModelKind::Judith),
            Name::new("Judith"),
            Transform::from_translation(at),
            Visibility::default(),
            Velocity::default(),
        ))
        .with_children(|p| {
            // torso
            p.spawn((
                Mesh3d(meshes.add(Capsule3d::new(0.32, 0.9))),
                MeshMaterial3d(cloth.clone()),
                Transform::from_xyz(0.0, 0.95, 0.0),
            ));
            // head
            p.spawn((
                Mesh3d(meshes.add(Sphere::new(0.26).mesh().ico(3).unwrap())),
                MeshMaterial3d(skin),
                Transform::from_xyz(0.0, 1.75, 0.0),
            ));
            // hair bun
            p.spawn((
                Mesh3d(meshes.add(Sphere::new(0.16).mesh().ico(2).unwrap())),
                MeshMaterial3d(hair),
                Transform::from_xyz(0.0, 1.95, -0.12),
            ));
            // net on her back
            p.spawn((
                Mesh3d(meshes.add(Cylinder::new(0.03, 1.6))),
                MeshMaterial3d(cloth),
                Transform::from_xyz(0.2, 1.1, -0.3).with_rotation(Quat::from_rotation_x(0.35)),
            ));
        })
        .id()
}
