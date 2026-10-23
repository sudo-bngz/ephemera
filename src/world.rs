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

/// A reed clump: solid enough to give cover on foot, and a visible
/// obstacle in flight. Radius is used for both.
#[derive(Component, Debug, Clone, Copy)]
pub struct ReedClump {
    pub radius: f32,
    pub height: f32,
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

/// Spawns a complete scene (ground, props, light, fog camera config) tagged
/// `PhaseScoped`. Returns the camera entity.
pub fn spawn_wetlands(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    assets: &WetlandAssets,
    cfg: WetlandsConfig,
) -> Entity {
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let s = cfg.half_size;

    // Ground
    commands.spawn((
        PhaseScoped,
        Name::new("Mud"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(s * 2.4, s * 2.4))),
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
        let tall = rng.gen_bool(0.25);
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

    // Lighting by mood.
    let (sun_color, sun_lux, sun_dir, ambient, fog_color, fog_start, fog_end, clear) =
        match cfg.mood {
            Mood::Dusk => (
                Color::srgb(0.95, 0.55, 0.50),
                1200.0,
                Vec3::new(-0.7, -0.25, 0.4),
                (Color::srgb(0.35, 0.30, 0.55), 160.0),
                Color::srgba(0.12, 0.08, 0.20, 1.0),
                18.0,
                95.0,
                Color::srgb(0.10, 0.07, 0.18),
            ),
            Mood::Dawn => (
                Color::srgb(1.0, 0.85, 0.75),
                6000.0,
                Vec3::new(0.5, -0.5, -0.4),
                (Color::srgb(0.55, 0.55, 0.70), 300.0),
                Color::srgba(0.70, 0.62, 0.72, 1.0),
                40.0,
                180.0,
                Color::srgb(0.62, 0.55, 0.68),
            ),
            Mood::Noon => (
                Color::srgb(1.0, 0.97, 0.90),
                14000.0,
                Vec3::new(0.3, -0.9, 0.2),
                (Color::srgb(0.60, 0.70, 0.75), 500.0),
                Color::srgba(0.55, 0.75, 0.70, 1.0),
                120.0,
                520.0,
                Color::srgb(0.52, 0.74, 0.70),
            ),
        };

    commands.insert_resource(ClearColor(clear));
    commands.insert_resource(AmbientLight {
        color: ambient.0,
        brightness: ambient.1,
        ..default()
    });

    commands.spawn((
        PhaseScoped,
        Name::new("Sun"),
        DirectionalLight {
            color: sun_color,
            illuminance: sun_lux,
            shadows_enabled: true,
            ..default()
        },
        Transform::default().looking_to(sun_dir.normalize(), Vec3::Y),
    ));

    commands
        .spawn((
            PhaseScoped,
            Name::new("Camera"),
            Camera3d::default(),
            Projection::from(PerspectiveProjection {
                fov: 62f32.to_radians(),
                near: 0.1,
                far: 900.0,
                ..default()
            }),
            Transform::from_xyz(0.0, 8.0, 16.0).looking_at(Vec3::ZERO, Vec3::Y),
            DistanceFog {
                color: fog_color,
                falloff: FogFalloff::Linear {
                    start: fog_start,
                    end: fog_end,
                },
                ..default()
            },
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
            ReedClump { radius, height },
            Name::new("ReedClump"),
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

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        // The initial `OnEnter(GamePhase::Title)` runs *before* `PreStartup`,
        // so the shared assets have to exist by the end of plugin setup.
        app.world_mut()
            .run_system_once(setup_assets)
            .expect("wetland assets need Assets<Mesh> and Assets<StandardMaterial>");
        app.add_systems(Update, drift_spores.in_set(crate::GameSet::Simulation));
    }
}
