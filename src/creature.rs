//! The Ephemera — a house-sized mosquito, built from primitives so the game
//! runs with zero external assets. Its wings beat, its legs twitch and its
//! body bobs; the amount of all three is driven by `Flap::intensity`, which
//! each phase sets (resting → hovering → racing).

use bevy::prelude::*;

use crate::components::Ephemera;

/// Drives the animation of a whole Ephemera.
#[derive(Component, Debug)]
pub struct Flap {
    /// 0 = perched & still, 1 = full-speed flight.
    pub intensity: f32,
    pub phase: f32,
}

impl Default for Flap {
    fn default() -> Self {
        Self {
            intensity: 0.2,
            phase: 0.0,
        }
    }
}

#[derive(Component)]
struct Wing {
    side: f32,
}

#[derive(Component)]
struct Leg {
    offset: f32,
    rest: Quat,
}

#[derive(Component)]
struct Body {
    rest_y: f32,
}

/// Palette for an individual creature, so rivals can be recoloured.
#[derive(Clone, Copy)]
pub struct Coat {
    pub body: Color,
    pub glow: Color,
}

impl Coat {
    pub const WILD: Coat = Coat {
        body: Color::srgb(0.16, 0.13, 0.22),
        glow: Color::srgb(0.35, 0.95, 0.80),
    };
}

pub fn spawn_ephemera(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
    coat: Coat,
    flap_intensity: f32,
    extra: impl Bundle,
) -> Entity {
    let chitin = materials.add(StandardMaterial {
        base_color: coat.body,
        perceptual_roughness: 0.35,
        metallic: 0.25,
        ..default()
    });
    let glow = materials.add(StandardMaterial {
        base_color: coat.glow,
        emissive: coat.glow.to_linear() * 6.0,
        ..default()
    });
    let membrane = materials.add(StandardMaterial {
        base_color: Color::srgba(0.75, 0.9, 1.0, 0.28),
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        perceptual_roughness: 0.1,
        ..default()
    });

    let body_mesh = meshes.add(Capsule3d::new(0.55, 3.6));
    let abdomen_mesh = meshes.add(Capsule3d::new(0.42, 2.4));
    let head_mesh = meshes.add(Sphere::new(0.5).mesh().ico(3).unwrap());
    let eye_mesh = meshes.add(Sphere::new(0.2).mesh().ico(2).unwrap());
    let proboscis_mesh = meshes.add(Cylinder::new(0.06, 2.6));
    let leg_mesh = meshes.add(Cylinder::new(0.045, 2.2));
    let wing_mesh = meshes.add(Cuboid::new(3.8, 0.02, 1.3));

    commands
        .spawn((
            Ephemera,
            crate::models::Swappable::new(crate::models::ModelKind::Ephemera),
            Flap {
                intensity: flap_intensity,
                phase: 0.0,
            },
            Name::new("Ephemera"),
            Transform::from_translation(at),
            Visibility::default(),
            extra,
        ))
        .with_children(|p| {
            // Thorax — the long axis points along -Z (forward).
            p.spawn((
                Body { rest_y: 0.0 },
                Mesh3d(body_mesh),
                MeshMaterial3d(chitin.clone()),
                Transform::from_xyz(0.0, 0.0, 0.3)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            ))
            .with_children(|b| {
                // Abdomen trailing behind and slightly down.
                b.spawn((
                    Mesh3d(abdomen_mesh),
                    MeshMaterial3d(chitin.clone()),
                    Transform::from_xyz(0.0, 2.7, 0.35).with_rotation(Quat::from_rotation_x(-0.18)),
                ));
                // Head in front.
                b.spawn((
                    Mesh3d(head_mesh),
                    MeshMaterial3d(chitin.clone()),
                    Transform::from_xyz(0.0, -2.1, 0.0),
                ))
                .with_children(|h| {
                    for side in [-1.0, 1.0] {
                        h.spawn((
                            Mesh3d(eye_mesh.clone()),
                            MeshMaterial3d(glow.clone()),
                            Transform::from_xyz(0.32 * side, 0.1, 0.2),
                        ));
                    }
                    h.spawn((
                        Mesh3d(proboscis_mesh),
                        MeshMaterial3d(chitin.clone()),
                        Transform::from_xyz(0.0, -1.5, 0.1)
                            .with_rotation(Quat::from_rotation_x(0.08)),
                    ));
                });
                // Six legs hanging from the thorax.
                for (i, z) in [-0.9_f32, 0.1, 1.1].into_iter().enumerate() {
                    for side in [-1.0_f32, 1.0] {
                        let rest = Quat::from_rotation_z(side * 1.05)
                            * Quat::from_rotation_x(0.2 * (i as f32 - 1.0));
                        b.spawn((
                            Leg {
                                offset: i as f32 * 1.3 + side * 0.4,
                                rest,
                            },
                            Mesh3d(leg_mesh.clone()),
                            MeshMaterial3d(chitin.clone()),
                            Transform::from_xyz(side * 0.55, z, -0.5).with_rotation(rest),
                        ));
                    }
                }
            });

            // Wings are children of the root so they stay level with the body.
            for side in [-1.0_f32, 1.0] {
                p.spawn((
                    Wing { side },
                    Mesh3d(wing_mesh.clone()),
                    MeshMaterial3d(membrane.clone()),
                    Transform::from_xyz(side * 2.0, 0.55, 0.2),
                ));
            }
        })
        .id()
}

fn animate(
    time: Res<Time>,
    mut flaps: Query<(&mut Flap, &Children)>,
    mut wings: Query<(&Wing, &mut Transform), (Without<Leg>, Without<Body>)>,
    mut bodies: Query<(&Body, &mut Transform, &Children), (Without<Wing>, Without<Leg>)>,
    mut legs: Query<(&Leg, &mut Transform), (Without<Wing>, Without<Body>)>,
) {
    let dt = time.delta_secs();
    for (mut flap, children) in &mut flaps {
        // Beat rate rises steeply with intensity: resting insects barely
        // tremble, racing ones become a blur.
        let hz = 2.0 + 26.0 * flap.intensity;
        flap.phase = (flap.phase + dt * hz * std::f32::consts::TAU) % std::f32::consts::TAU;
        let beat = flap.phase.sin();
        let amp = 0.08 + 0.75 * flap.intensity;

        for child in children.iter() {
            if let Ok((wing, mut t)) = wings.get_mut(child) {
                // Hinge at the wing root: shift the pivot by rotating about
                // the attachment point rather than the mesh centre.
                let angle = beat * amp * wing.side;
                let rot = Quat::from_rotation_z(angle);
                t.rotation = rot;
                t.translation =
                    rot * Vec3::new(wing.side * 1.9, 0.0, 0.0) + Vec3::new(0.0, 0.55, 0.2);
            }
            if let Ok((body, mut t, legs_children)) = bodies.get_mut(child) {
                let bob = (time.elapsed_secs() * (1.5 + 4.0 * flap.intensity)).sin();
                t.translation.y = body.rest_y + bob * (0.03 + 0.12 * flap.intensity);
                for leg_e in legs_children.iter() {
                    if let Ok((leg, mut lt)) = legs.get_mut(leg_e) {
                        // Legs tuck back in flight, dangle when perched.
                        let tuck = Quat::from_rotation_x(-0.9 * flap.intensity);
                        let twitch = ((time.elapsed_secs() * 3.0 + leg.offset).sin() * 0.08)
                            * (1.0 - flap.intensity);
                        lt.rotation = leg.rest * tuck * Quat::from_rotation_z(twitch);
                    }
                }
            }
        }
    }
}

pub struct CreaturePlugin;

impl Plugin for CreaturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate.in_set(crate::GameSet::Camera));
    }
}
