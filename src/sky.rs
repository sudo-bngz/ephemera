//! Sky dome, sun, clouds, distant mesas — and the fog that ties them to the
//! ground.
//!
//! The trick that makes it read as one world: the fog colour *is* the dome's
//! horizon colour. Distant terrain fogs into exactly the colour of the sky
//! behind it, so there is no visible seam and the world feels endless.
//!
//! The dome and its children ignore fog (they would otherwise be 100 %
//! fogged) and follow the camera, so you can never fly to the edge of the sky.
//! The mesas live in world space, far away, so they still give parallax.

use bevy::light::NotShadowCaster;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::components::PhaseScoped;
use crate::world::Mood;

pub const DOME_RADIUS: f32 = 880.0;
pub const CAMERA_FAR: f32 = 1000.0;

/// Every colour and distance that defines a time of day.
#[derive(Clone, Copy, Debug)]
pub struct SkyPalette {
    pub zenith: Color,
    pub horizon: Color,
    pub sun: Color,
    /// Direction the sunlight travels (from the sun towards the ground).
    pub sun_dir: Vec3,
    pub sun_lux: f32,
    pub ambient: Color,
    pub ambient_brightness: f32,
    /// Distance at which fog hides everything.
    pub visibility: f32,
    pub cloud: Color,
    pub clouds: usize,
}

impl SkyPalette {
    pub fn for_mood(mood: Mood) -> Self {
        match mood {
            Mood::Dusk => Self {
                zenith: Color::srgb(0.05, 0.04, 0.16),
                horizon: Color::srgb(0.42, 0.22, 0.36),
                sun: Color::srgb(1.0, 0.50, 0.38),
                sun_dir: Vec3::new(-0.75, -0.12, 0.55),
                sun_lux: 1400.0,
                ambient: Color::srgb(0.38, 0.30, 0.58),
                ambient_brightness: 180.0,
                visibility: 140.0,
                cloud: Color::srgb(0.55, 0.30, 0.42),
                clouds: 14,
            },
            Mood::Dawn => Self {
                zenith: Color::srgb(0.30, 0.38, 0.62),
                horizon: Color::srgb(0.92, 0.72, 0.66),
                sun: Color::srgb(1.0, 0.82, 0.62),
                sun_dir: Vec3::new(0.55, -0.30, -0.45),
                sun_lux: 6500.0,
                ambient: Color::srgb(0.60, 0.58, 0.72),
                ambient_brightness: 320.0,
                visibility: 260.0,
                cloud: Color::srgb(1.0, 0.86, 0.82),
                clouds: 18,
            },
            Mood::Noon => Self {
                zenith: Color::srgb(0.16, 0.42, 0.62),
                horizon: Color::srgb(0.62, 0.82, 0.78),
                sun: Color::srgb(1.0, 0.96, 0.86),
                sun_dir: Vec3::new(0.35, -0.80, 0.30),
                sun_lux: 14000.0,
                ambient: Color::srgb(0.62, 0.74, 0.78),
                ambient_brightness: 520.0,
                visibility: 420.0,
                cloud: Color::srgb(0.97, 0.99, 0.98),
                clouds: 22,
            },
        }
    }

    pub fn fog(&self) -> DistanceFog {
        DistanceFog {
            color: self.horizon,
            // Looking towards the sun, the fog glows in the sun's colour.
            directional_light_color: self.sun.with_alpha(0.55),
            directional_light_exponent: 18.0,
            falloff: FogFalloff::from_visibility_squared(self.visibility),
        }
    }
}

/// Root of the sky; kept centred on the camera every frame.
#[derive(Component)]
pub struct SkyDome;

/// Cloud layer — rotates slowly around the zenith.
#[derive(Component)]
struct CloudLayer;

/// Spawns dome, sun disc, clouds and the distant mesa ring.
pub fn spawn_sky(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    palette: &SkyPalette,
    seed: u64,
) {
    let mut rng = StdRng::seed_from_u64(seed ^ 0x5eed);
    let to_sun = -palette.sun_dir.normalize();

    // --- dome: an inside-out sphere with a vertex-colour gradient ----------
    let mut dome = Sphere::new(1.0).mesh().uv(64, 32);
    let zenith = palette.zenith.to_linear();
    let horizon = palette.horizon.to_linear();
    let sun = palette.sun.to_linear();
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        dome.attribute(Mesh::ATTRIBUTE_POSITION).cloned()
    {
        let colors: Vec<[f32; 4]> = positions
            .iter()
            .map(|p| {
                let n = Vec3::from(*p).normalize();
                // Below the horizon is pure horizon colour (it is hidden by
                // fogged ground anyway); above, ease towards the zenith.
                let t = n.y.max(0.0).powf(0.55);
                let mut c = horizon.mix(&zenith, t);
                // Soft halo around the sun.
                let glow = n.dot(to_sun).max(0.0).powf(10.0) * 0.55;
                c = c.mix(&sun, glow);
                [c.red, c.green, c.blue, 1.0]
            })
            .collect();
        dome.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    let dome_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    });
    let sun_mat = materials.add(StandardMaterial {
        base_color: palette.sun,
        emissive: palette.sun.to_linear() * 8.0,
        unlit: true,
        fog_enabled: false,
        ..default()
    });
    let cloud_mat = materials.add(StandardMaterial {
        base_color: palette.cloud.with_alpha(0.82),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        fog_enabled: false,
        ..default()
    });
    let cloud_mesh = meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap());

    commands
        .spawn((
            PhaseScoped,
            SkyDome,
            Name::new("Sky"),
            Transform::default(),
            Visibility::default(),
        ))
        .with_children(|sky| {
            sky.spawn((
                Mesh3d(meshes.add(dome)),
                MeshMaterial3d(dome_mat),
                Transform::from_scale(Vec3::splat(DOME_RADIUS)),
                NotShadowCaster,
            ));
            // Sun disc, just inside the dome, facing the centre.
            let sun_pos = to_sun * (DOME_RADIUS * 0.94);
            sky.spawn((
                Mesh3d(meshes.add(Circle::new(1.0))),
                MeshMaterial3d(sun_mat),
                Transform::from_translation(sun_pos)
                    .looking_at(Vec3::ZERO, Vec3::Y)
                    .with_scale(Vec3::splat(26.0)),
                NotShadowCaster,
            ));
            // Clouds: clusters of flattened spheres on a high ring.
            sky.spawn((CloudLayer, Transform::default(), Visibility::default()))
                .with_children(|layer| {
                    for _ in 0..palette.clouds {
                        let a = rng.gen_range(0.0..std::f32::consts::TAU);
                        let elev: f32 = rng.gen_range(0.08..0.45);
                        let r = DOME_RADIUS * 0.82;
                        let centre = Vec3::new(
                            a.cos() * elev.cos() * r,
                            elev.sin() * r,
                            a.sin() * elev.cos() * r,
                        );
                        let puffs = rng.gen_range(3..7);
                        for _ in 0..puffs {
                            let off = Vec3::new(
                                rng.gen_range(-60.0..60.0),
                                rng.gen_range(-6.0..10.0),
                                rng.gen_range(-60.0..60.0),
                            );
                            let size = rng.gen_range(28.0..60.0);
                            layer.spawn((
                                Mesh3d(cloud_mesh.clone()),
                                MeshMaterial3d(cloud_mat.clone()),
                                Transform::from_translation(centre + off).with_scale(Vec3::new(
                                    size * 1.8,
                                    size * 0.35,
                                    size,
                                )),
                                NotShadowCaster,
                            ));
                        }
                    }
                });
        });

    // --- distant mesas: world-space silhouettes for parallax ----------------
    // Pre-blended towards the horizon so they read as far away without the
    // fog swallowing them completely.
    let rock = palette.horizon.mix(&Color::srgb(0.20, 0.16, 0.22), 0.45);
    let rock_mat = materials.add(StandardMaterial {
        base_color: rock,
        perceptual_roughness: 1.0,
        fog_enabled: false,
        unlit: true,
        ..default()
    });
    let rock_far = palette.horizon.mix(&Color::srgb(0.20, 0.16, 0.22), 0.25);
    let rock_far_mat = materials.add(StandardMaterial {
        base_color: rock_far,
        fog_enabled: false,
        unlit: true,
        ..default()
    });
    // The ring follows the camera across the plane (not up and down), so the
    // horizon never runs out however far the endless world is travelled.
    let ring = commands
        .spawn((
            PhaseScoped,
            MesaRing,
            Name::new("Mesa ring"),
            Transform::default(),
            Visibility::default(),
        ))
        .id();
    let mesa_mesh = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(7));
    let spire_mesh = meshes.add(Cone::new(1.0, 1.0).mesh().resolution(6));
    for i in 0..46 {
        let far = i % 2 == 1;
        let a = rng.gen_range(0.0..std::f32::consts::TAU);
        // Inside the dome (both are centred on the camera).
        let dist = if far {
            rng.gen_range(560.0..610.0)
        } else {
            rng.gen_range(420.0..500.0)
        };
        let h = rng.gen_range(30.0..110.0);
        let w = rng.gen_range(30.0..90.0);
        let spire = rng.gen_bool(0.3);
        let mesa = commands
            .spawn((
                Mesh3d(if spire {
                    spire_mesh.clone()
                } else {
                    mesa_mesh.clone()
                }),
                MeshMaterial3d(if far {
                    rock_far_mat.clone()
                } else {
                    rock_mat.clone()
                }),
                crate::models::Swappable::new(crate::models::ModelKind::Mesa),
                Transform::from_xyz(a.cos() * dist, h * 0.5 - 2.0, a.sin() * dist)
                    .with_rotation(Quat::from_rotation_y(rng.gen_range(0.0..3.0)))
                    .with_scale(if spire {
                        Vec3::new(w * 0.4, h * 1.6, w * 0.4)
                    } else {
                        Vec3::new(w, h, w * rng.gen_range(0.6..1.0))
                    }),
                NotShadowCaster,
            ))
            .id();
        commands.entity(ring).add_child(mesa);
    }
}

/// Distant mesas, kept centred on the camera horizontally.
#[derive(Component)]
struct MesaRing;

fn follow_camera_xz(
    cam: Query<&Transform, (With<Camera3d>, Without<MesaRing>)>,
    mut ring: Query<&mut Transform, With<MesaRing>>,
) {
    let Ok(cam) = cam.single() else {
        return;
    };
    for mut tf in &mut ring {
        tf.translation.x = cam.translation.x;
        tf.translation.z = cam.translation.z;
    }
}

fn follow_camera(
    // The local Transform, not GlobalTransform: the chase camera has just
    // moved this frame and propagation hasn't run yet.
    cam: Query<&Transform, (With<Camera3d>, Without<SkyDome>)>,
    mut sky: Query<&mut Transform, With<SkyDome>>,
) {
    let Ok(cam) = cam.single() else {
        return;
    };
    for mut tf in &mut sky {
        tf.translation = cam.translation;
    }
}

fn drift_clouds(time: Res<Time>, mut q: Query<&mut Transform, With<CloudLayer>>) {
    for mut tf in &mut q {
        tf.rotation = Quat::from_rotation_y(time.elapsed_secs() * 0.004);
    }
}

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                follow_camera.after(crate::camera::follow),
                follow_camera_xz.after(crate::camera::follow),
                drift_clouds,
            )
                .in_set(crate::GameSet::Camera),
        );
    }
}
