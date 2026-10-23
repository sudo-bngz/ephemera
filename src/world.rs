//! The alien wetlands. Everything is procedural — mud, pools, reed beds,
//! bioluminescent spores — so each phase can build its own scene with a
//! different mood from the same seed.

use bevy::ecs::system::RunSystemOnce;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::components::PhaseScoped;

/// Shared meshes & materials for the wetlands, created once.
#[derive(Resource)]
pub struct WetlandAssets {
    pub reed_mesh: Handle<Mesh>,
    pub reed_mat: Handle<StandardMaterial>,
    pub reed_mat_tall: Handle<StandardMaterial>,
    pub pool_mesh: Handle<Mesh>,
    pub pool_mat: Handle<StandardMaterial>,
    pub spore_mesh: Handle<Mesh>,
    pub spore_mat: Handle<StandardMaterial>,
    pub mud_mat: Handle<StandardMaterial>,
    pub pad_mesh: Handle<Mesh>,
    pub pad_mat: Handle<StandardMaterial>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// Capture: deep dusk, long shadows, thick fog — stealth.
    Dusk,
    /// Taming: pale dawn at camp.
    Dawn,
    /// Racing: hard noon light, far fog so the horizon rushes at you.
    Noon,
}

#[derive(Clone, Copy, Debug)]
pub struct WetlandsConfig {
    pub seed: u64,
    pub half_size: f32,
    pub reed_clumps: usize,
    pub pools: usize,
    pub spores: usize,
    pub mood: Mood,
    /// Keep a corridor of this radius clear around the origin (camp / nest).
    pub clearing: f32,
}

/// A reed clump. Gives cover on foot. In flight, short clumps are soft
/// (you plough through them and they bend), tall ones are `solid` obstacles
/// you slide along. Every clump sways in the wind and bends away from
/// anything that flies past.
#[derive(Component, Debug, Clone, Copy)]
pub struct ReedClump {
    pub radius: f32,
    pub height: f32,
    pub solid: bool,
    /// Current lean (x, z) in radians, and its angular velocity.
    pub tilt: Vec2,
    pub tilt_vel: Vec2,
    /// Wind phase offset.
    pub phase: f32,
}

impl ReedClump {
    /// Push the clump over in a (world xz) direction; `strength` in rad/s.
    pub fn shove(&mut self, dir: Vec2, strength: f32) {
        self.tilt_vel += dir.normalize_or_zero() * strength;
    }
}

/// A pool of standing water — Judith splashes (noise!) if she crosses it.
#[derive(Component, Debug, Clone, Copy)]
pub struct Pool {
    pub radius: f32,
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(WetlandAssets {
        reed_mesh: meshes.add(Cylinder::new(0.09, 1.0)),
        reed_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.52, 0.34),
            perceptual_roughness: 0.95,
            ..default()
        }),
        reed_mat_tall: materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.34, 0.56),
            perceptual_roughness: 0.95,
            ..default()
        }),
        pool_mesh: meshes.add(Circle::new(1.0)),
        pool_mat: materials.add(StandardMaterial {
            base_color: Color::srgba(0.10, 0.45, 0.50, 0.75),
            emissive: LinearRgba::new(0.02, 0.18, 0.20, 1.0),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.05,
            metallic: 0.1,
            ..default()
        }),
        spore_mesh: meshes.add(Sphere::new(0.18).mesh().ico(1).unwrap()),
        spore_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 1.0, 0.85),
            emissive: LinearRgba::new(1.2, 4.0, 3.0, 1.0),
            unlit: true,
            ..default()
        }),
        mud_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.17, 0.14, 0.15),
            perceptual_roughness: 1.0,
            ..default()
        }),
        pad_mesh: meshes.add(Circle::new(1.0)),
        pad_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.24, 0.55, 0.40),
            perceptual_roughness: 0.7,
            ..default()
        }),
    });
}

/// Spawns a complete scene (ground, props, sky, light, fogged camera) tagged
/// `PhaseScoped`. Returns the camera entity.
///
/// `corridor` is a polyline (e.g. the race track) along which no *tall* reed
/// clump is placed at random — the race places its own, deliberate obstacles.
pub fn spawn_wetlands(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    assets: &WetlandAssets,
    cfg: WetlandsConfig,
    corridor: &[Vec3],
) -> Entity {
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let s = cfg.half_size;
    let palette = crate::sky::SkyPalette::for_mood(cfg.mood);

    // Ground — much bigger than the play area so it runs into the fog
    // instead of ending at a visible edge.
    let ground = (s * 2.4).max(1500.0);
    commands.spawn((
        PhaseScoped,
        Name::new("Mud"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(ground, ground))),
        MeshMaterial3d(assets.mud_mat.clone()),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // Pools
    for _ in 0..cfg.pools {
        let p = random_point(&mut rng, s, cfg.clearing);
        let radius = rng.gen_range(4.0..14.0);
        commands.spawn((
            PhaseScoped,
            Pool { radius },
            Mesh3d(assets.pool_mesh.clone()),
            MeshMaterial3d(assets.pool_mat.clone()),
            Transform::from_xyz(p.x, 0.02, p.z)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(radius)),
            NotShadowCaster,
        ));
    }

    // Reed clumps — a parent entity with ~10–25 stalks as children.
    for _ in 0..cfg.reed_clumps {
        let p = random_point(&mut rng, s, cfg.clearing);
        let near_corridor = corridor
            .iter()
            .any(|c| (c.xz() - p.xz()).length_squared() < 20.0 * 20.0);
        let tall = rng.gen_bool(0.25) && !near_corridor;
        let radius = rng.gen_range(1.8..4.5);
        let height = if tall {
            rng.gen_range(7.0..12.0)
        } else {
            rng.gen_range(2.6..4.2)
        };
        spawn_reed_clump(commands, assets, &mut rng, p, radius, height, tall);
    }

    // Bioluminescent spores drifting above the marsh.
    for _ in 0..cfg.spores {
        let p = random_point(&mut rng, s, 0.0);
        commands.spawn((
            PhaseScoped,
            Spore {
                base_y: rng.gen_range(1.0..6.0),
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
            },
            Mesh3d(assets.spore_mesh.clone()),
            MeshMaterial3d(assets.spore_mat.clone()),
            Transform::from_xyz(p.x, 2.0, p.z),
            NotShadowCaster,
        ));
    }

    crate::sky::spawn_sky(commands, meshes, materials, &palette, cfg.seed);

    commands.insert_resource(ClearColor(palette.horizon));
    commands.insert_resource(AmbientLight {
        color: palette.ambient,
        brightness: palette.ambient_brightness,
        ..default()
    });

    commands.spawn((
        PhaseScoped,
        Name::new("Sun"),
        DirectionalLight {
            color: palette.sun,
            illuminance: palette.sun_lux,
            shadows_enabled: true,
            ..default()
        },
        Transform::default().looking_to(palette.sun_dir.normalize(), Vec3::Y),
    ));

    commands
        .spawn((
            PhaseScoped,
            Name::new("Camera"),
            Camera3d::default(),
            Projection::from(PerspectiveProjection {
                fov: 62f32.to_radians(),
                near: 0.1,
                far: crate::sky::CAMERA_FAR,
                ..default()
            }),
            Transform::from_xyz(0.0, 8.0, 16.0).looking_at(Vec3::ZERO, Vec3::Y),
            palette.fog(),
            crate::camera::ChaseCamera::default(),
        ))
        .id()
}

#[derive(Component)]
pub struct Spore {
    base_y: f32,
    phase: f32,
}

pub fn spawn_reed_clump(
    commands: &mut Commands,
    assets: &WetlandAssets,
    rng: &mut StdRng,
    at: Vec3,
    radius: f32,
    height: f32,
    tall: bool,
) -> Entity {
    let mat = if tall {
        assets.reed_mat_tall.clone()
    } else {
        assets.reed_mat.clone()
    };
    let count = (radius * radius * 1.4) as usize + 8;
    commands
        .spawn((
            PhaseScoped,
            ReedClump {
                radius,
                height,
                solid: tall,
                tilt: Vec2::ZERO,
                tilt_vel: Vec2::ZERO,
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
            },
            Name::new("ReedClump"),
            crate::models::Swappable::sized(
                if tall {
                    crate::models::ModelKind::ReedTall
                } else {
                    crate::models::ModelKind::Reed
                },
                Vec3::new(radius, height, radius),
            ),
            Transform::from_translation(at),
            Visibility::default(),
        ))
        .with_children(|p| {
            for _ in 0..count {
                let a = rng.gen_range(0.0..std::f32::consts::TAU);
                let r = radius * rng.gen_range(0.0_f32..1.0).sqrt();
                let h = height * rng.gen_range(0.7..1.2);
                let lean = rng.gen_range(-0.12..0.12);
                p.spawn((
                    Mesh3d(assets.reed_mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(a.cos() * r, h * 0.5, a.sin() * r)
                        .with_rotation(Quat::from_rotation_x(lean) * Quat::from_rotation_z(lean))
                        .with_scale(Vec3::new(1.0 + h * 0.08, h, 1.0 + h * 0.08)),
                    NotShadowCaster,
                ));
            }
        })
        .id()
}

fn random_point(rng: &mut StdRng, half: f32, clearing: f32) -> Vec3 {
    loop {
        let p = Vec3::new(rng.gen_range(-half..half), 0.0, rng.gen_range(-half..half));
        if p.length() >= clearing {
            return p;
        }
    }
}

fn drift_spores(time: Res<Time>, mut q: Query<(&Spore, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (s, mut tf) in &mut q {
        tf.translation.y = s.base_y + (t * 0.7 + s.phase).sin() * 0.6;
        tf.translation.x += (t * 0.4 + s.phase).cos() * 0.004;
    }
}

/// Wind plus a damped spring per clump: shoves from passing riders make the
/// whole bed whip and settle. The clump's origin is at ground level, so
/// rotating it pivots every stalk around its base.
fn sway_reeds(time: Res<Time>, mut q: Query<(&mut ReedClump, &mut Transform)>) {
    let dt = time.delta_secs().min(1.0 / 20.0);
    let t = time.elapsed_secs();
    for (mut reed, mut tf) in &mut q {
        // Tall reeds are stiffer and swing slower.
        let k = if reed.solid { 22.0 } else { 38.0 };
        let c = if reed.solid { 3.0 } else { 4.5 };
        let acc = -k * reed.tilt - c * reed.tilt_vel;
        let vel = reed.tilt_vel + acc * dt;
        reed.tilt_vel = vel;
        reed.tilt = (reed.tilt + vel * dt).clamp_length_max(0.9);
        let wind = Vec2::new(
            (t * 0.9 + reed.phase).sin() * 0.04,
            (t * 0.7 + reed.phase * 1.3).cos() * 0.025,
        );
        let lean = reed.tilt + wind;
        // Leaning towards +x means rotating about -z; towards +z, about +x.
        tf.rotation = Quat::from_rotation_z(-lean.x) * Quat::from_rotation_x(lean.y);
    }
}

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        // The initial `OnEnter(GamePhase::Title)` runs *before* `PreStartup`,
        // so the shared assets have to exist by the end of plugin setup.
        app.world_mut()
            .run_system_once(setup_assets)
            .expect("wetland assets need Assets<Mesh> and Assets<StandardMaterial>");
        app.add_systems(
            Update,
            (drift_spores, sway_reeds).in_set(crate::GameSet::Simulation),
        );
    }
}
