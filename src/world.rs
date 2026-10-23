//! The endless wetlands, streamed in chunks around whoever the camera follows.
//!
//! `spawn_world` sets up a scene (sky, sun, fogged camera) and the world's
//! seed; `stream_chunks` then builds 128 m chunks as you approach and drops
//! them behind you. Each chunk is a pure function of the seed and its
//! coordinates — terrain from `terrain.rs`, props and landmarks from a
//! per-chunk RNG — so the world is endless, seamless and reproducible.
//!
//! Landmarks (monoliths, orbs, veils, sigils) wear editions of alea works,
//! painted at runtime by `art.rs`.

use std::collections::{HashMap, HashSet};

use bevy::ecs::system::RunSystemOnce;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::art::{Art, Landmark, Work};
use crate::components::{CameraTarget, PhaseScoped};
use crate::models::{ModelKind, Swappable};
use crate::terrain::{Biome, Flatten, SEA_LEVEL, Terrain, hash};

pub const CHUNK: f32 = 128.0;
const GRID: usize = 32;
/// Chunks kept around the focus (radius, in chunks).
const VIEW_RADIUS: i32 = 4;
/// Chunks built per frame once the area around the player exists.
const BUILD_BUDGET: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// Capture: deep dusk, long shadows, thick fog — stealth.
    Dusk,
    /// Taming: pale dawn at camp.
    Dawn,
    /// Racing: hard noon light, far fog so the horizon rushes at you.
    Noon,
}

/// What a phase asks of the world.
#[derive(Clone, Debug)]
pub struct WorldSetup {
    pub seed: u64,
    pub mood: Mood,
    /// Discs forced flat (camp, nest).
    pub flatten: Vec<Flatten>,
    /// A polyline (the race track) kept clear of solid props.
    pub keep_clear: Vec<Vec3>,
    pub keep_clear_radius: f32,
}

impl WorldSetup {
    pub fn new(seed: u64, mood: Mood) -> Self {
        Self {
            seed,
            mood,
            flatten: Vec::new(),
            keep_clear: Vec::new(),
            keep_clear_radius: 20.0,
        }
    }
}

/// The current world's shape, for any system that needs ground heights.
#[derive(Resource, Clone)]
pub struct WorldTerrain(pub Terrain);

/// Streaming state.
#[derive(Resource)]
pub struct WorldGen {
    pub seed: u64,
    loaded: HashMap<IVec2, Entity>,
    clear_cells: HashMap<IVec2, Vec<Vec2>>,
    clear_radius: f32,
}

impl WorldGen {
    #[cfg_attr(not(test), allow(dead_code))] // read by the streaming tests
    pub fn loaded(&self) -> impl Iterator<Item = &IVec2> {
        self.loaded.keys()
    }

    fn keeps_clear(&self, p: Vec2) -> bool {
        let c = (p / 32.0).floor().as_ivec2();
        let r2 = self.clear_radius * self.clear_radius;
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(pts) = self.clear_cells.get(&(c + IVec2::new(dx, dz))) {
                    if pts.iter().any(|q| q.distance_squared(p) < r2) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// A chunk root, with its grid coordinates.
#[derive(Component)]
pub struct Chunk(#[cfg_attr(not(test), allow(dead_code))] pub IVec2);

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
    /// The clump's own turn about the vertical (the lean is applied on top).
    pub yaw: f32,
}

impl ReedClump {
    /// Push the clump over in a (world xz) direction; `strength` in rad/s.
    pub fn shove(&mut self, dir: Vec2, strength: f32) {
        self.tilt_vel += dir.normalize_or_zero() * strength;
    }
}

/// Something solid a flier can hit: a vertical cylinder around the entity,
/// from `below` under its origin to `above` over it.
#[derive(Component, Debug, Clone, Copy)]
pub struct Obstacle {
    pub radius: f32,
    pub below: f32,
    pub above: f32,
}

/// Bobs and turns slowly: floating landmarks.
#[derive(Component)]
struct Floater {
    base_y: f32,
    phase: f32,
    bob: f32,
    spin: f32,
}

#[derive(Component)]
pub struct Spore {
    base_y: f32,
    phase: f32,
}

/// Shared meshes & materials, created once.
#[derive(Resource)]
pub struct WetlandAssets {
    pub reed_meshes: Vec<Handle<Mesh>>,
    pub reed_mat: Handle<StandardMaterial>,
    pub reed_mat_tall: Handle<StandardMaterial>,
    pub water_mesh: Handle<Mesh>,
    pub water_mat: Handle<StandardMaterial>,
    pub terrain_mat: Handle<StandardMaterial>,
    pub spore_mesh: Handle<Mesh>,
    pub spore_mat: Handle<StandardMaterial>,
    pub pad_mesh: Handle<Mesh>,
    pub pad_mat: Handle<StandardMaterial>,
    stalk_mesh: Handle<Mesh>,
    cap_mesh: Handle<Mesh>,
    stalk_mat: Handle<StandardMaterial>,
    cap_mats: Vec<Handle<StandardMaterial>>,
    gill_mat: Handle<StandardMaterial>,
    rib_mesh: Handle<Mesh>,
    bone_mat: Handle<StandardMaterial>,
    shard_mesh: Handle<Mesh>,
    shard_mats: Vec<Handle<StandardMaterial>>,
    slab_mesh: Handle<Mesh>,
    orb_mesh: Handle<Mesh>,
    veil_mesh: Handle<Mesh>,
    sigil_mesh: Handle<Mesh>,
}

/// A clump of reeds as one mesh (unit radius, unit height), so a clump is a
/// single entity — and identical clumps batch together on the GPU.
fn reed_clump_mesh(seed: u64) -> Mesh {
    let mut rng = StdRng::seed_from_u64(seed);
    let stalk = Mesh::from(Cylinder::new(0.03, 1.0).mesh().resolution(5));
    let mut out: Option<Mesh> = None;
    for _ in 0..22 {
        let a = rng.gen_range(0.0..std::f32::consts::TAU);
        let r = rng.gen_range(0.0_f32..1.0).sqrt();
        let h = rng.gen_range(0.65..1.15);
        let lean = Vec2::new(rng.gen_range(-0.1..0.1), rng.gen_range(-0.1..0.1));
        let s = stalk.clone().transformed_by(
            Transform::from_xyz(a.cos() * r, h * 0.5, a.sin() * r)
                .with_rotation(Quat::from_rotation_x(lean.x) * Quat::from_rotation_z(lean.y))
                .with_scale(Vec3::new(1.0, h, 1.0)),
        );
        match &mut out {
            None => out = Some(s),
            Some(m) => m.merge(&s).expect("same attributes"),
        }
    }
    out.unwrap()
}

/// A bone rib: a half-ring in the XY plane, radius 1, feet at x = ±1.
fn rib_mesh() -> Mesh {
    let seg = Mesh::from(Cylinder::new(0.06, 1.0).mesh().resolution(6));
    let n = 10;
    let mut out: Option<Mesh> = None;
    for k in 0..n {
        let a0 = std::f32::consts::PI * k as f32 / n as f32;
        let a1 = std::f32::consts::PI * (k + 1) as f32 / n as f32;
        let p0 = Vec3::new(a0.cos(), a0.sin(), 0.0);
        let p1 = Vec3::new(a1.cos(), a1.sin(), 0.0);
        let mid = (p0 + p1) * 0.5;
        let dir = (p1 - p0).normalize();
        // Taper towards the tip of the arch.
        let thick = 1.0 - 0.45 * (mid.y);
        let s = seg.clone().transformed_by(
            Transform::from_translation(mid)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir))
                .with_scale(Vec3::new(thick, p0.distance(p1) * 1.08, thick)),
        );
        match &mut out {
            None => out = Some(s),
            Some(m) => m.merge(&s).expect("same attributes"),
        }
    }
    out.unwrap()
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let emissive = |m: &mut Assets<StandardMaterial>, c: Color, glow: f32| {
        m.add(StandardMaterial {
            base_color: c,
            emissive: c.to_linear() * glow,
            perceptual_roughness: 0.4,
            ..default()
        })
    };
    commands.insert_resource(WetlandAssets {
        reed_meshes: (0..8)
            .map(|i| meshes.add(reed_clump_mesh(1000 + i)))
            .collect(),
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
        water_mesh: meshes.add(Plane3d::default().mesh().size(CHUNK, CHUNK)),
        water_mat: materials.add(StandardMaterial {
            base_color: Color::srgba(0.10, 0.42, 0.48, 0.78),
            emissive: LinearRgba::new(0.02, 0.16, 0.18, 1.0),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.05,
            metallic: 0.1,
            ..default()
        }),
        terrain_mat: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        }),
        spore_mesh: meshes.add(Sphere::new(0.18).mesh().ico(1).unwrap()),
        spore_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 1.0, 0.85),
            emissive: LinearRgba::new(1.2, 4.0, 3.0, 1.0),
            unlit: true,
            ..default()
        }),
        pad_mesh: meshes.add(Circle::new(1.0)),
        pad_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.24, 0.55, 0.40),
            perceptual_roughness: 0.7,
            ..default()
        }),
        stalk_mesh: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(10)),
        cap_mesh: meshes.add(Sphere::new(1.0).mesh().uv(18, 9)),
        stalk_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.80, 0.72),
            perceptual_roughness: 0.8,
            ..default()
        }),
        cap_mats: vec![
            emissive(&mut materials, Color::srgb(0.55, 0.20, 0.65), 0.6),
            emissive(&mut materials, Color::srgb(0.95, 0.45, 0.25), 0.5),
            emissive(&mut materials, Color::srgb(0.25, 0.75, 0.70), 0.6),
        ],
        gill_mat: emissive(&mut materials, Color::srgb(0.6, 1.0, 0.9), 3.0),
        rib_mesh: meshes.add(rib_mesh()),
        bone_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.88, 0.84, 0.74),
            perceptual_roughness: 0.6,
            ..default()
        }),
        shard_mesh: meshes.add(Cone::new(1.0, 1.0).mesh().resolution(5)),
        shard_mats: vec![
            emissive(&mut materials, Color::srgb(0.45, 0.85, 1.0), 1.8),
            emissive(&mut materials, Color::srgb(0.75, 0.45, 1.0), 1.8),
        ],
        slab_mesh: meshes.add(Cuboid::new(5.0, 15.0, 1.2)),
        orb_mesh: meshes.add(Sphere::new(1.0).mesh().uv(32, 16)),
        veil_mesh: meshes.add(Rectangle::new(1.0, 1.0)),
        sigil_mesh: meshes.add(Circle::new(1.0).mesh().resolution(48)),
    });
}

/// Builds the scene for a phase: sky, sun, fogged camera, and the world
/// that streams around the camera's target. Returns the camera entity and
/// the terrain (to place things on the ground during setup).
pub fn spawn_world(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    art: &mut Art,
    setup: WorldSetup,
) -> (Entity, Terrain) {
    let palette = crate::sky::SkyPalette::for_mood(setup.mood);
    let mut terrain = Terrain::new(setup.seed);
    terrain.flatten = setup.flatten.clone();

    let mut clear_cells: HashMap<IVec2, Vec<Vec2>> = HashMap::new();
    for p in &setup.keep_clear {
        let q = p.xz();
        clear_cells
            .entry((q / 32.0).floor().as_ivec2())
            .or_default()
            .push(q);
    }
    commands.insert_resource(WorldGen {
        seed: setup.seed,
        loaded: HashMap::new(),
        clear_cells,
        clear_radius: setup.keep_clear_radius.min(32.0),
    });
    commands.insert_resource(WorldTerrain(terrain.clone()));
    art.reset(setup.seed);

    crate::sky::spawn_sky(commands, meshes, materials, &palette, setup.seed);
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
    let cam = commands
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
        .id();
    (cam, terrain)
}

fn terrain_mesh(t: &Terrain, coord: IVec2) -> (Mesh, f32) {
    let n = GRID + 1;
    let step = CHUNK / GRID as f32;
    let origin = coord.as_vec2() * CHUNK;
    // Heights on the grid plus a one-cell border, sampled once: normals come
    // from neighbouring samples (central differences), and the border makes
    // them match across chunk edges exactly.
    let m = n + 2;
    let mut hs = vec![0.0f32; m * m];
    for j in 0..m {
        for i in 0..m {
            let wx = origin.x + (i as f32 - 1.0) * step;
            let wz = origin.y + (j as f32 - 1.0) * step;
            hs[j * m + i] = t.height(wx, wz);
        }
    }
    let at = |i: usize, j: usize| hs[(j + 1) * m + (i + 1)];
    let mut positions = Vec::with_capacity(n * n);
    let mut normals = Vec::with_capacity(n * n);
    let mut colors = Vec::with_capacity(n * n);
    let mut uvs = Vec::with_capacity(n * n);
    let mut min_h = f32::MAX;
    for j in 0..n {
        for i in 0..n {
            let (lx, lz) = (i as f32 * step, j as f32 * step);
            let h = at(i, j);
            min_h = min_h.min(h);
            let dx = hs[(j + 1) * m + (i + 2)] - hs[(j + 1) * m + i];
            let dz = hs[(j + 2) * m + (i + 1)] - hs[j * m + (i + 1)];
            positions.push([lx, h, lz]);
            normals.push(Vec3::new(-dx, 2.0 * step, -dz).normalize().to_array());
            colors.push(t.color(origin.x + lx, origin.y + lz, h));
            uvs.push([i as f32 / GRID as f32, j as f32 / GRID as f32]);
        }
    }
    let mut indices = Vec::with_capacity(GRID * GRID * 6);
    for j in 0..GRID {
        for i in 0..GRID {
            let a = (j * n + i) as u32;
            let (b, c, d) = (a + n as u32, a + 1, a + n as u32 + 1);
            indices.extend_from_slice(&[a, b, c, c, b, d]);
        }
    }
    let mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(bevy::mesh::Indices::U32(indices));
    (mesh, min_h)
}

/// Everything about one chunk, as a pure function of the seed and coords.
fn spawn_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    art: &mut Art,
    a: &WetlandAssets,
    t: &Terrain,
    wg: &WorldGen,
    coord: IVec2,
) -> Entity {
    let origin = coord.as_vec2() * CHUNK;
    let (mesh, min_h) = terrain_mesh(t, coord);
    let mut rng = StdRng::seed_from_u64(hash(&[wg.seed as i64, coord.x as i64, coord.y as i64]));
    let centre_biome = t.biome(origin.x + CHUNK * 0.5, origin.y + CHUNK * 0.5);

    let root = commands
        .spawn((
            PhaseScoped,
            Chunk(coord),
            Name::new(format!("Chunk {},{}", coord.x, coord.y)),
            Transform::from_xyz(origin.x, 0.0, origin.y),
            Visibility::default(),
        ))
        .id();

    let mut kids: Vec<Entity> = Vec::new();
    kids.push(
        commands
            .spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(a.terrain_mat.clone()),
            ))
            .id(),
    );
    if min_h < SEA_LEVEL {
        kids.push(
            commands
                .spawn((
                    Mesh3d(a.water_mesh.clone()),
                    MeshMaterial3d(a.water_mat.clone()),
                    Transform::from_xyz(CHUNK * 0.5, SEA_LEVEL, CHUNK * 0.5),
                    NotShadowCaster,
                ))
                .id(),
        );
    }

    // A random spot in the chunk: local position, world xz, ground height.
    let spot = |rng: &mut StdRng| {
        let l = Vec2::new(
            rng.gen_range(2.0..CHUNK - 2.0),
            rng.gen_range(2.0..CHUNK - 2.0),
        );
        let w = origin + l;
        (l, w, t.height(w.x, w.y))
    };
    let flat_ok = |w: Vec2| {
        !t.flatten
            .iter()
            .any(|f| f.centre.distance(w) < f.radius * 1.4)
    };
    let solid_ok = |w: Vec2| !wg.keeps_clear(w) && flat_ok(w);
    let local = |l: Vec2, y: f32| Vec3::new(l.x, y, l.y);

    // --- reeds: everywhere wet, densest in the marsh -----------------------
    let reed_count = match centre_biome {
        Biome::Marsh => 24,
        Biome::Fungal => 8,
        Biome::Monoliths => 6,
        Biome::Bones => 2,
        Biome::Crystal => 0,
    };
    for _ in 0..reed_count {
        let (l, w, h) = spot(&mut rng);
        if h < SEA_LEVEL - 1.2 || !flat_ok(w) {
            continue;
        }
        let tall = rng.gen_bool(0.22) && solid_ok(w);
        let radius = rng.gen_range(1.8..4.2);
        let height = if tall {
            rng.gen_range(7.0..12.0)
        } else {
            rng.gen_range(2.6..4.2)
        };
        let variant = rng.gen_range(0..a.reed_meshes.len());
        kids.push(reed_entity(
            commands,
            a,
            &mut rng,
            local(l, h.max(SEA_LEVEL - 0.3)),
            radius,
            height,
            tall,
            variant,
        ));
    }
    for _ in 0..(reed_count / 3) {
        let (l, _, h) = spot(&mut rng);
        kids.push(
            commands
                .spawn((
                    Spore {
                        base_y: h.max(SEA_LEVEL) + rng.gen_range(1.0..6.0),
                        phase: rng.gen_range(0.0..std::f32::consts::TAU),
                    },
                    Mesh3d(a.spore_mesh.clone()),
                    MeshMaterial3d(a.spore_mat.clone()),
                    Transform::from_translation(local(l, h + 2.0)),
                    NotShadowCaster,
                ))
                .id(),
        );
    }

    // --- biome features ------------------------------------------------------
    match centre_biome {
        Biome::Fungal => {
            for _ in 0..6 {
                let (l, w, h) = spot(&mut rng);
                if h < SEA_LEVEL || !solid_ok(w) {
                    continue;
                }
                let stalk_h = rng.gen_range(6.0..17.0);
                let stalk_r = rng.gen_range(0.5..1.2);
                let cap_r = rng.gen_range(3.0..8.0);
                let cap = a.cap_mats[rng.gen_range(0..a.cap_mats.len())].clone();
                let e = commands
                    .spawn((
                        Transform::from_translation(local(l, h))
                            .with_rotation(Quat::from_rotation_z(rng.gen_range(-0.12..0.12))),
                        Visibility::default(),
                        Obstacle {
                            radius: stalk_r + 0.8,
                            below: 0.0,
                            above: stalk_h,
                        },
                    ))
                    .with_children(|p| {
                        p.spawn((
                            Mesh3d(a.stalk_mesh.clone()),
                            MeshMaterial3d(a.stalk_mat.clone()),
                            Transform::from_xyz(0.0, stalk_h * 0.5, 0.0)
                                .with_scale(Vec3::new(stalk_r, stalk_h, stalk_r)),
                        ));
                        p.spawn((
                            Mesh3d(a.cap_mesh.clone()),
                            MeshMaterial3d(cap),
                            Transform::from_xyz(0.0, stalk_h, 0.0).with_scale(Vec3::new(
                                cap_r,
                                cap_r * 0.38,
                                cap_r,
                            )),
                            Obstacle {
                                radius: cap_r,
                                below: cap_r * 0.2,
                                above: cap_r * 0.38,
                            },
                        ));
                        // Glowing gills under the cap.
                        p.spawn((
                            Mesh3d(a.cap_mesh.clone()),
                            MeshMaterial3d(a.gill_mat.clone()),
                            Transform::from_xyz(0.0, stalk_h - cap_r * 0.06, 0.0)
                                .with_scale(Vec3::new(cap_r * 0.92, cap_r * 0.12, cap_r * 0.92)),
                            NotShadowCaster,
                        ));
                    })
                    .id();
                kids.push(e);
            }
        }
        Biome::Bones => {
            for _ in 0..rng.gen_range(1..3) {
                let (l, w, h) = spot(&mut rng);
                if h < SEA_LEVEL || !solid_ok(w) {
                    continue;
                }
                // A spine of ribs: arches to fly through.
                let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
                let spine = Vec2::new(yaw.cos(), yaw.sin());
                let ribs = rng.gen_range(5..9);
                let size = rng.gen_range(10.0..18.0);
                for k in 0..ribs {
                    let along = (k as f32 - ribs as f32 * 0.5) * size * 0.42;
                    let lp = l + spine * along;
                    let wp = origin + lp;
                    if !solid_ok(wp) {
                        continue;
                    }
                    let taper = 1.0 - (k as f32 / ribs as f32 - 0.5).abs();
                    let r = size * (0.6 + 0.4 * taper);
                    let gh = t.height(wp.x, wp.y);
                    // The rib's plane is perpendicular to the spine.
                    let e = commands
                        .spawn((
                            Mesh3d(a.rib_mesh.clone()),
                            MeshMaterial3d(a.bone_mat.clone()),
                            Transform::from_translation(local(lp, gh - 0.5))
                                .with_rotation(Quat::from_rotation_y(
                                    -yaw + std::f32::consts::FRAC_PI_2,
                                ))
                                .with_scale(Vec3::splat(r)),
                        ))
                        .with_children(|p| {
                            // Only the feet are solid: fly between them.
                            for side in [-1.0, 1.0] {
                                p.spawn((
                                    Transform::from_xyz(side, 0.0, 0.0),
                                    Obstacle {
                                        radius: 1.4,
                                        below: 0.0,
                                        above: r * 0.55,
                                    },
                                ));
                            }
                        })
                        .id();
                    kids.push(e);
                }
            }
        }
        Biome::Crystal => {
            for _ in 0..9 {
                let (l, w, h) = spot(&mut rng);
                if !solid_ok(w) {
                    continue;
                }
                let mat = a.shard_mats[rng.gen_range(0..a.shard_mats.len())].clone();
                let tallest = rng.gen_range(5.0..15.0);
                let n = rng.gen_range(3..7);
                let shards: Vec<(Vec3, Quat, Vec3)> = (0..n)
                    .map(|_| {
                        let hh = tallest * rng.gen_range(0.4..1.0);
                        let rr = hh * rng.gen_range(0.12..0.2);
                        let off =
                            Vec3::new(rng.gen_range(-1.5..1.5), 0.0, rng.gen_range(-1.5..1.5));
                        let tilt = Quat::from_rotation_x(rng.gen_range(-0.4..0.4))
                            * Quat::from_rotation_z(rng.gen_range(-0.4..0.4));
                        (off + tilt * Vec3::Y * hh * 0.5, tilt, Vec3::new(rr, hh, rr))
                    })
                    .collect();
                let e = commands
                    .spawn((
                        Transform::from_translation(local(l, h.max(SEA_LEVEL))),
                        Visibility::default(),
                        Obstacle {
                            radius: 2.6,
                            below: 0.0,
                            above: tallest,
                        },
                    ))
                    .with_children(|p| {
                        for (pos, rot, scale) in shards {
                            p.spawn((
                                Mesh3d(a.shard_mesh.clone()),
                                MeshMaterial3d(mat.clone()),
                                Transform::from_translation(pos)
                                    .with_rotation(rot)
                                    .with_scale(scale),
                            ));
                        }
                    })
                    .id();
                kids.push(e);
            }
        }
        Biome::Monoliths => {
            for _ in 0..3 {
                let (l, w, h) = spot(&mut rng);
                if !solid_ok(w) {
                    continue;
                }
                let slot = rng.r#gen::<u64>();
                kids.push(monolith(
                    commands,
                    materials,
                    art,
                    a,
                    &mut rng,
                    local(l, h.max(SEA_LEVEL)),
                    slot,
                ));
                kids.push(sigil(
                    commands,
                    materials,
                    art,
                    a,
                    &mut rng,
                    local(l, h.max(SEA_LEVEL)),
                    slot,
                ));
            }
        }
        Biome::Marsh => {}
    }

    // --- landmarks anywhere (rarer outside the monolith field) ---------------
    let roll = |rng: &mut StdRng, p: f64| rng.gen_bool(p);
    if roll(&mut rng, 0.07) {
        let (l, w, h) = spot(&mut rng);
        if solid_ok(w) {
            let slot = rng.r#gen::<u64>();
            kids.push(monolith(
                commands,
                materials,
                art,
                a,
                &mut rng,
                local(l, h.max(SEA_LEVEL)),
                slot,
            ));
        }
    }
    if roll(&mut rng, 0.14) {
        let (l, w, h) = spot(&mut rng);
        if solid_ok(w) {
            let slot = rng.r#gen::<u64>();
            kids.push(orb(
                commands,
                materials,
                art,
                a,
                &mut rng,
                local(l, h.max(SEA_LEVEL)),
                slot,
            ));
        }
    }
    if roll(&mut rng, 0.12) {
        let (l, _, h) = spot(&mut rng);
        let slot = rng.r#gen::<u64>();
        kids.push(veil(
            commands,
            materials,
            art,
            a,
            &mut rng,
            local(l, h.max(SEA_LEVEL)),
            slot,
        ));
    }
    if roll(&mut rng, 0.10) {
        let (l, w, h) = spot(&mut rng);
        if h > SEA_LEVEL + 0.3 && t.normal(w.x, w.y).y > 0.97 {
            let slot = rng.r#gen::<u64>();
            kids.push(sigil(
                commands,
                materials,
                art,
                a,
                &mut rng,
                local(l, h),
                slot,
            ));
        }
    }

    commands.entity(root).add_children(&kids);
    root
}

fn reed_entity(
    commands: &mut Commands,
    a: &WetlandAssets,
    rng: &mut StdRng,
    at: Vec3,
    radius: f32,
    height: f32,
    tall: bool,
    variant: usize,
) -> Entity {
    let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
    commands
        .spawn((
            ReedClump {
                radius,
                height,
                solid: tall,
                tilt: Vec2::ZERO,
                tilt_vel: Vec2::ZERO,
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                yaw,
            },
            // The clump's own scale already sizes it: a swapped model is
            // authored at unit size and inherits that scale.
            Swappable::new(if tall {
                ModelKind::ReedTall
            } else {
                ModelKind::Reed
            }),
            Mesh3d(a.reed_meshes[variant % a.reed_meshes.len()].clone()),
            MeshMaterial3d(if tall {
                a.reed_mat_tall.clone()
            } else {
                a.reed_mat.clone()
            }),
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::new(radius, height, radius)),
            NotShadowCaster,
        ))
        .id()
}

/// A deliberate reed clump at a world position (the race's obstacles).
pub fn spawn_reed_clump(
    commands: &mut Commands,
    assets: &WetlandAssets,
    rng: &mut StdRng,
    at: Vec3,
    radius: f32,
    height: f32,
    tall: bool,
) -> Entity {
    let variant = rng.gen_range(0..assets.reed_meshes.len());
    let e = reed_entity(commands, assets, rng, at, radius, height, tall, variant);
    commands.entity(e).insert(PhaseScoped);
    e
}

pub(crate) fn monolith(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    art: &mut Art,
    a: &WetlandAssets,
    rng: &mut StdRng,
    ground: Vec3,
    slot: u64,
) -> Entity {
    let (mat, id) = art.edition(Work::Monolith, slot, materials);
    let y = ground.y + rng.gen_range(14.0..30.0);
    let scale = rng.gen_range(0.8..1.6);
    commands
        .spawn((
            Landmark(id),
            Mesh3d(a.slab_mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_xyz(ground.x, y, ground.z)
                .with_rotation(Quat::from_rotation_y(
                    rng.gen_range(0.0..std::f32::consts::TAU),
                ))
                .with_scale(Vec3::splat(scale)),
            Floater {
                base_y: y,
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                bob: 1.5,
                spin: rng.gen_range(-0.08..0.08),
            },
            Obstacle {
                radius: 3.2 * scale,
                below: 7.5 * scale,
                above: 7.5 * scale,
            },
        ))
        .id()
}

pub(crate) fn orb(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    art: &mut Art,
    a: &WetlandAssets,
    rng: &mut StdRng,
    ground: Vec3,
    slot: u64,
) -> Entity {
    let (mat, id) = art.edition(Work::Orb, slot, materials);
    let r = rng.gen_range(4.0..10.0);
    let y = ground.y + r + rng.gen_range(12.0..45.0);
    commands
        .spawn((
            Landmark(id),
            Mesh3d(a.orb_mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_xyz(ground.x, y, ground.z).with_scale(Vec3::splat(r)),
            Floater {
                base_y: y,
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                bob: 3.0,
                spin: rng.gen_range(-0.25..0.25),
            },
            Obstacle {
                radius: r,
                below: r,
                above: r,
            },
        ))
        .id()
}

pub(crate) fn veil(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    art: &mut Art,
    a: &WetlandAssets,
    rng: &mut StdRng,
    ground: Vec3,
    slot: u64,
) -> Entity {
    let (mat, id) = art.edition(Work::Veil, slot, materials);
    let y = ground.y + rng.gen_range(70.0..120.0);
    commands
        .spawn((
            Landmark(id),
            Mesh3d(a.veil_mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_xyz(ground.x, y, ground.z)
                .with_rotation(Quat::from_rotation_y(
                    rng.gen_range(0.0..std::f32::consts::TAU),
                ))
                .with_scale(Vec3::new(
                    rng.gen_range(150.0..240.0),
                    rng.gen_range(60.0..90.0),
                    1.0,
                )),
            Floater {
                base_y: y,
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                bob: 4.0,
                spin: rng.gen_range(-0.02..0.02),
            },
            NotShadowCaster,
        ))
        .id()
}

pub(crate) fn sigil(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    art: &mut Art,
    a: &WetlandAssets,
    rng: &mut StdRng,
    ground: Vec3,
    slot: u64,
) -> Entity {
    let (mat, id) = art.edition(Work::Sigil, slot, materials);
    commands
        .spawn((
            Landmark(id),
            Mesh3d(a.sigil_mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_xyz(ground.x, ground.y + 0.15, ground.z)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(rng.gen_range(9.0..22.0))),
            Floater {
                base_y: ground.y + 0.15,
                phase: 0.0,
                bob: 0.0,
                spin: rng.gen_range(-0.05..0.05),
            },
            NotShadowCaster,
        ))
        .id()
}

/// Builds chunks around the focus, nearest first, and drops far ones.
fn stream_chunks(
    mut commands: Commands,
    wg: Option<ResMut<WorldGen>>,
    terrain: Option<Res<WorldTerrain>>,
    assets: Res<WetlandAssets>,
    mut art: ResMut<Art>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    focus: Query<&Transform, With<CameraTarget>>,
    cams: Query<&Transform, With<Camera3d>>,
) {
    let (Some(mut wg), Some(terrain)) = (wg, terrain) else {
        return;
    };
    let Some(p) = focus
        .iter()
        .next()
        .or(cams.iter().next())
        .map(|t| t.translation)
    else {
        return;
    };
    let centre = (p.xz() / CHUNK).floor().as_ivec2();

    let mut wanted: Vec<IVec2> = Vec::new();
    for dz in -VIEW_RADIUS..=VIEW_RADIUS {
        for dx in -VIEW_RADIUS..=VIEW_RADIUS {
            if dx * dx + dz * dz <= VIEW_RADIUS * VIEW_RADIUS + 1 {
                wanted.push(centre + IVec2::new(dx, dz));
            }
        }
    }
    wanted.sort_by_key(|c| (*c - centre).length_squared());

    // Drop chunks well outside the view (hysteresis: one ring of slack).
    let keep: HashSet<IVec2> = wanted.iter().copied().collect();
    let limit = (VIEW_RADIUS + 1) * (VIEW_RADIUS + 1) + 1;
    let stale: Vec<IVec2> = wg
        .loaded
        .keys()
        .filter(|c| !keep.contains(c) && (**c - centre).length_squared() > limit)
        .copied()
        .collect();
    for c in stale {
        if let Some(e) = wg.loaded.remove(&c) {
            commands.entity(e).despawn();
        }
    }

    // The ground under the player must exist right away; the rest streams.
    let mut budget = BUILD_BUDGET;
    for c in wanted {
        if wg.loaded.contains_key(&c) {
            continue;
        }
        let near = (c - centre).length_squared() <= 2;
        if budget == 0 && !near {
            break;
        }
        let e = spawn_chunk(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut art,
            &assets,
            &terrain.0,
            &wg,
            c,
        );
        wg.loaded.insert(c, e);
        budget = budget.saturating_sub(1);
    }
}

fn drift_spores(time: Res<Time>, mut q: Query<(&Spore, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (s, mut tf) in &mut q {
        tf.translation.y = s.base_y + (t * 0.7 + s.phase).sin() * 0.6;
    }
}

fn float_landmarks(time: Res<Time>, mut q: Query<(&Floater, &mut Transform)>) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for (f, mut tf) in &mut q {
        tf.translation.y = f.base_y + (t * 0.4 + f.phase).sin() * f.bob;
        // Sigils lie flat: spin them about the world vertical.
        tf.rotate_axis(Dir3::Y, f.spin * dt);
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
        // The clump's own yaw first, then the lean in world space.
        tf.rotation = Quat::from_rotation_z(-lean.x)
            * Quat::from_rotation_x(lean.y)
            * Quat::from_rotation_y(reed.yaw);
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
            (drift_spores, sway_reeds, float_landmarks).in_set(crate::GameSet::Simulation),
        )
        .add_systems(Update, stream_chunks.in_set(crate::GameSet::Camera));
    }
}

#[cfg(test)]
mod bench {
    /// `cargo test chunk_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn chunk_cost() {
        let t = crate::terrain::Terrain::new(2026);
        let start = std::time::Instant::now();
        let n = 40;
        for i in 0..n {
            let _ = super::terrain_mesh(&t, bevy::math::IVec2::new(i * 7 - 100, i * 3));
        }
        eprintln!("terrain mesh: {:?} per chunk", start.elapsed() / n as u32);
    }
}
