//! The endless wetlands, streamed in chunks around whoever the camera follows.
//!
//! `spawn_world` sets up a scene (sky, sun, fogged camera) and the world's
//! seed; `stream_chunks` then builds 128 m chunks as you approach and drops
//! them behind you. Each chunk is a pure function of the seed and its
//! coordinates — terrain from `terrain.rs`, props and landmarks from a
//! per-chunk RNG — so the world is endless, seamless and reproducible.
//!
//! Landmarks (monoliths, orbs, veils, sigils) wear editions of alea works,
//! painted at runtime by `art.rs`. Settlements (`settlement.rs`) and animals
//! (`fauna.rs`) are spawned from here too, chunk by chunk.

use std::collections::{HashMap, HashSet};

use bevy::ecs::system::RunSystemOnce;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::art::{Art, Landmark, Work};
use crate::components::{CameraTarget, PhaseScoped};
use crate::models::{ModelKind, Swappable};
use crate::terrain::{Flatten, Props, SEA_LEVEL, Sampler, Terrain, hash};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

pub const CHUNK: f32 = 128.0;
const GRID: usize = 32;
/// Ground painting texels per chunk side (1.33 m each).
const TEXELS: usize = 96;
/// Chunk builds in flight at once.
const IN_FLIGHT: usize = 6;
/// Water mesh vertices per side.
const WATER_GRID: usize = 24;
/// Chunks kept around the focus (radius, in chunks).
const VIEW_RADIUS: i32 = 4;

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
    /// Distance from the origin reality (`reality.rs`); 0 for the home world.
    pub depth: u32,
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
            depth: 0,
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
    pending: HashMap<IVec2, Task<ChunkData>>,
    clear_cells: HashMap<IVec2, Vec<Vec2>>,
    clear_radius: f32,
    /// Streaming pauses while a reality shifts into another.
    pub paused: bool,
    /// Props of chunks spawned before this time grow in from nothing (the
    /// world resolving after a shift).
    pub grow_until: f32,
}

impl WorldGen {
    /// The chunks in the world, with their roots.
    pub fn chunks(&self) -> Vec<(IVec2, Entity)> {
        self.loaded.iter().map(|(c, e)| (*c, *e)).collect()
    }

    /// Forgets every chunk (the caller despawns them) and takes a new seed:
    /// the next frames stream the new reality in.
    pub fn restart(&mut self, seed: u64) {
        self.seed = seed;
        self.loaded.clear();
        self.pending.clear();
    }

    /// Chunk builds in flight.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

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

/// A chunk's ground as built: its heights (to morph from, when the reality
/// shifts), its mesh and its material.
#[derive(Component)]
pub struct ChunkGround {
    pub heights: Vec<f32>,
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

/// A prop of a chunk (anything but its ground and water): what dissolves
/// when the reality shifts, and grows back in the next one.
#[derive(Component, Debug, Clone, Copy)]
pub struct Prop {
    pub base_scale: Vec3,
}

/// Growing in from nothing: `t` runs 0–1.
#[derive(Component)]
pub struct Grow(pub f32);

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
    base: Vec3,
    phase: f32,
}

/// Swells and settles: mushroom caps and their gills.
#[derive(Component)]
struct Breathe {
    base: Vec3,
    phase: f32,
    amount: f32,
}

/// Trees lean in the wind, pivoting at the root.
#[derive(Component)]
pub struct Sway {
    pub phase: f32,
    pub amount: f32,
    pub yaw: f32,
}

/// A swarm of glowing moths circling a point.
#[derive(Component)]
struct Moth {
    phase: f32,
    radius: f32,
    speed: f32,
    height: f32,
    centre: Vec3,
}

/// Shared meshes & materials, created once.
#[derive(Resource)]
pub struct WetlandAssets {
    pub reed_meshes: Vec<Handle<Mesh>>,
    pub reed_mat: Handle<StandardMaterial>,
    pub reed_mat_tall: Handle<StandardMaterial>,
    pub water_mesh: Handle<Mesh>,
    pub water_mat: Handle<StandardMaterial>,
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
    moth_mesh: Handle<Mesh>,
    moth_mat: Handle<StandardMaterial>,
    pub trunk_mesh: Handle<Mesh>,
    pub canopy_mesh: Handle<Mesh>,
    pub trunk_mat: Handle<StandardMaterial>,
    pub leaf_mats: Vec<Handle<StandardMaterial>>,
    pub rock_mesh: Handle<Mesh>,
    pub rock_mat: Handle<StandardMaterial>,
    pub spire_mesh: Handle<Mesh>,
    pub grass_mat: Handle<StandardMaterial>,
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

/// A moth: two triangular wings meeting at the body line (x = 0), so
/// scaling x folds them.
fn moth_mesh() -> Mesh {
    let positions: Vec<[f32; 3]> = vec![
        [0.0, 0.0, -0.12],
        [0.0, 0.0, 0.1],
        [0.22, 0.02, 0.04],
        [0.0, 0.0, -0.12],
        [0.0, 0.0, 0.1],
        [-0.22, 0.02, 0.04],
    ];
    let normals = vec![[0.0, 1.0, 0.0]; 6];
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(bevy::mesh::Indices::U16(vec![0, 1, 2, 3, 5, 4]))
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
        // Subdivided: `animate_water` moves its vertices in waves that
        // repeat every chunk, so every water chunk shares this one mesh.
        water_mesh: meshes.add(
            Plane3d::default()
                .mesh()
                .size(CHUNK, CHUNK)
                .subdivisions(WATER_GRID as u32 - 1),
        ),
        water_mat: materials.add(StandardMaterial {
            base_color: Color::srgba(0.10, 0.42, 0.48, 0.78),
            emissive: LinearRgba::new(0.02, 0.16, 0.18, 1.0),
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
        moth_mesh: meshes.add(moth_mesh()),
        moth_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.9, 0.7),
            emissive: LinearRgba::rgb(4.0, 3.2, 1.6),
            double_sided: true,
            cull_mode: None,
            unlit: true,
            ..default()
        }),
        // Unit trunk (height 1, radius 1 at the foot) and unit canopy.
        trunk_mesh: meshes.add(Cone::new(1.0, 1.0).mesh().resolution(7)),
        canopy_mesh: meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
        trunk_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.22, 0.18),
            perceptual_roughness: 0.95,
            ..default()
        }),
        leaf_mats: vec![
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.22, 0.48, 0.26),
                perceptual_roughness: 0.9,
                ..default()
            }),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.46, 0.52, 0.18),
                perceptual_roughness: 0.9,
                ..default()
            }),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.58, 0.30, 0.42),
                emissive: LinearRgba::rgb(0.25, 0.05, 0.15),
                perceptual_roughness: 0.9,
                ..default()
            }),
        ],
        rock_mesh: meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap()),
        rock_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.40, 0.38, 0.40),
            perceptual_roughness: 0.98,
            ..default()
        }),
        spire_mesh: meshes.add(Cone::new(1.0, 1.0).mesh().resolution(5)),
        grass_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.52, 0.60, 0.26),
            perceptual_roughness: 0.95,
            ..default()
        }),
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
    let mut terrain = Terrain::at_depth(setup.seed, setup.depth);
    terrain.flatten = setup.flatten.clone();
    // Pay for the regions around the start now, not during the first frames.
    let _ = terrain.sampler(Vec2::ZERO, CHUNK * (VIEW_RADIUS as f32 + 1.0));

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
        pending: HashMap::new(),
        clear_cells,
        clear_radius: setup.keep_clear_radius.min(32.0),
        paused: false,
        grow_until: 0.0,
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
            // Where the world is heard from.
            bevy::audio::SpatialListener::new(0.6),
        ))
        .id();
    (cam, terrain)
}

/// What a chunk build produces: ready to turn into a mesh and a texture.
pub struct ChunkData {
    coord: IVec2,
    /// Heights on the grid plus a one-cell border, row-major, `(GRID + 3)²`.
    heights: Vec<f32>,
    texture: Vec<u8>,
    min_h: f32,
}

impl ChunkData {
    /// The lowest point sampled inside the chunk.
    pub(crate) fn min_height(&self) -> f32 {
        self.min_h
    }

    /// Height at a chunk-local position, from the sampled grid.
    pub(crate) fn height_at(&self, lx: f32, lz: f32) -> f32 {
        let m = GRID + 3;
        let step = CHUNK / GRID as f32;
        let (fx, fz) = (
            (lx / step).clamp(0.0, GRID as f32),
            (lz / step).clamp(0.0, GRID as f32),
        );
        let (i, j) = (fx.floor() as usize, fz.floor() as usize);
        let (tx, tz) = (fx - i as f32, fz - j as f32);
        let at = |i: usize, j: usize| self.heights[(j + 1) * m + (i + 1)];
        let a = at(i, j) + (at(i + 1, j) - at(i, j)) * tx;
        let b = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * tx;
        a + (b - a) * tz
    }

    /// Ground mesh positions and normals from a height grid (`(GRID+3)²`,
    /// border included), as `mesh()` builds them.
    pub(crate) fn ground_vertices(hs: &[f32]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let n = GRID + 1;
        let m = GRID + 3;
        let step = CHUNK / GRID as f32;
        let at = |i: usize, j: usize| hs[(j + 1) * m + (i + 1)];
        let mut positions = Vec::with_capacity(n * n);
        let mut normals = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let (lx, lz) = (i as f32 * step, j as f32 * step);
                let dx = hs[(j + 1) * m + (i + 2)] - hs[(j + 1) * m + i];
                let dz = hs[(j + 2) * m + (i + 1)] - hs[j * m + (i + 1)];
                positions.push([lx, at(i, j), lz]);
                normals.push(Vec3::new(-dx, 2.0 * step, -dz).normalize().to_array());
            }
        }
        (positions, normals)
    }

    fn mesh(&self) -> Mesh {
        let n = GRID + 1;
        let m = GRID + 3;
        let step = CHUNK / GRID as f32;
        let hs = &self.heights;
        let at = |i: usize, j: usize| hs[(j + 1) * m + (i + 1)];
        let mut positions = Vec::with_capacity(n * n);
        let mut normals = Vec::with_capacity(n * n);
        let mut uvs = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let (lx, lz) = (i as f32 * step, j as f32 * step);
                // Central differences on the border-padded grid: normals
                // match exactly across chunk edges.
                let dx = hs[(j + 1) * m + (i + 2)] - hs[(j + 1) * m + i];
                let dz = hs[(j + 2) * m + (i + 1)] - hs[j * m + (i + 1)];
                positions.push([lx, at(i, j), lz]);
                normals.push(Vec3::new(-dx, 2.0 * step, -dz).normalize().to_array());
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
        Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(bevy::mesh::Indices::U32(indices))
    }

    pub(crate) fn heights(&self) -> &[f32] {
        &self.heights
    }

    pub(crate) fn image(&self) -> Image {
        let mut img = Image::new(
            Extent3d {
                width: TEXELS as u32,
                height: TEXELS as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.texture.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        img
    }
}

/// Samples the terrain and paints the ground for one chunk. Pure: runs on
/// the compute pool with a sampler made on the main thread.
pub fn build_chunk(mut s: Sampler, coord: IVec2) -> ChunkData {
    let m = GRID + 3;
    let step = CHUNK / GRID as f32;
    let origin = coord.as_vec2() * CHUNK;
    let mut heights = vec![0.0f32; m * m];
    let mut min_h = f32::MAX;
    for j in 0..m {
        for i in 0..m {
            let h = s.height(
                origin.x + (i as f32 - 1.0) * step,
                origin.y + (j as f32 - 1.0) * step,
            );
            heights[j * m + i] = h;
            if (1..m - 1).contains(&i) && (1..m - 1).contains(&j) {
                min_h = min_h.min(h);
            }
        }
    }
    let mut texture = Vec::with_capacity(TEXELS * TEXELS * 4);
    let texel = CHUNK / TEXELS as f32;
    for j in 0..TEXELS {
        for i in 0..TEXELS {
            let c = s.color(
                origin.x + (i as f32 + 0.5) * texel,
                origin.y + (j as f32 + 0.5) * texel,
            );
            texture.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    ChunkData {
        coord,
        heights,
        texture,
        min_h,
    }
}

/// Turns a built chunk into entities: ground, water and props. Props are a
/// pure function of the seed and coords too.
fn spawn_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    art: &mut Art,
    a: &WetlandAssets,
    sa: &crate::settlement::SettlementAssets,
    fa: &crate::fauna::FaunaAssets,
    t: &Terrain,
    wg: &WorldGen,
    data: ChunkData,
) -> Entity {
    let coord = data.coord;
    let origin = coord.as_vec2() * CHUNK;
    let mesh = meshes.add(data.mesh());
    let min_h = data.min_h;
    let ground_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(data.image())),
        perceptual_roughness: 0.95,
        ..default()
    });
    let ground = ChunkGround {
        heights: data.heights.clone(),
        mesh: mesh.clone(),
        material: ground_mat.clone(),
    };
    let mut rng = StdRng::seed_from_u64(hash(&[wg.seed as i64, coord.x as i64, coord.y as i64]));
    let centre_biome = t.props_at(origin.x + CHUNK * 0.5, origin.y + CHUNK * 0.5);

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
    let ground_e = commands
        .spawn((Mesh3d(mesh), MeshMaterial3d(ground_mat)))
        .id();
    commands.entity(root).insert(ground);
    // Everything spawned after the ground and the water is a prop.
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
    let first_prop = kids.len();

    // A random spot in the chunk: local position, world xz, ground height.
    let spot = |rng: &mut StdRng| {
        let l = Vec2::new(
            rng.gen_range(2.0..CHUNK - 2.0),
            rng.gen_range(2.0..CHUNK - 2.0),
        );
        let w = origin + l;
        (l, w, data.height_at(l.x, l.y))
    };
    let sites = t.sites_near(origin + CHUNK * 0.5, CHUNK);
    let flat_ok = |w: Vec2| {
        !t.flatten
            .iter()
            .any(|f| f.centre.distance(w) < f.radius * 1.4)
            && !sites.iter().any(|s| s.centre.distance(w) < s.radius * 1.3)
            && t.river(w.x, w.y).0 < 0.5
    };
    let solid_ok = |w: Vec2| !wg.keeps_clear(w) && flat_ok(w);
    let local = |l: Vec2, y: f32| Vec3::new(l.x, y, l.y);

    // --- reeds: everywhere wet, densest in the marsh -----------------------
    let reed_count = match centre_biome {
        Props::Marsh => 24,
        Props::Fungal => 8,
        Props::Monoliths => 6,
        Props::Bones => 2,
        Props::Crystal => 0,
        Props::Peaks => 0,
        Props::Grove => 3,
        Props::None => 4,
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
    // Reeds and grass line the rivers' banks, whatever the land.
    for _ in 0..10 {
        let (l, w, h) = spot(&mut rng);
        let (carve, _) = t.river(w.x, w.y);
        if !(0.08..0.55).contains(&carve) || h < SEA_LEVEL - 1.0 || !flat_ok(w) {
            continue;
        }
        let variant = rng.gen_range(0..a.reed_meshes.len());
        let (radius, height) = (rng.gen_range(1.4..3.0), rng.gen_range(1.8..3.4));
        kids.push(reed_entity(
            commands,
            a,
            &mut rng,
            local(l, h.max(SEA_LEVEL - 0.3)),
            radius,
            height,
            false,
            variant,
        ));
    }
    for _ in 0..(reed_count / 3) {
        let (l, _, h) = spot(&mut rng);
        kids.push(
            commands
                .spawn((
                    Spore {
                        base: local(l, h.max(SEA_LEVEL) + rng.gen_range(1.0..6.0)),
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

    // Moths over the wet and the strange lands, at dusk most of all.
    let swarms = match centre_biome {
        Props::Marsh | Props::Fungal | Props::Monoliths => rng.gen_range(0..3),
        _ => 0,
    };
    for _ in 0..swarms {
        let (l, _, h) = spot(&mut rng);
        let centre = local(l, h.max(SEA_LEVEL) + rng.gen_range(2.0..6.0));
        for _ in 0..rng.gen_range(6..14) {
            kids.push(
                commands
                    .spawn((
                        Moth {
                            phase: rng.gen_range(0.0..std::f32::consts::TAU),
                            radius: rng.gen_range(1.0..4.5),
                            speed: rng.gen_range(0.6..1.6)
                                * if rng.gen_bool(0.5) { 1.0 } else { -1.0 },
                            height: rng.gen_range(-1.0..1.5),
                            centre,
                        },
                        Mesh3d(a.moth_mesh.clone()),
                        MeshMaterial3d(a.moth_mat.clone()),
                        Transform::from_translation(centre).with_scale(Vec3::splat(1.6)),
                        NotShadowCaster,
                    ))
                    .id(),
            );
        }
    }

    // --- biome features ------------------------------------------------------
    match centre_biome {
        Props::Fungal => {
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
                        let breath = rng.gen_range(0.0..std::f32::consts::TAU);
                        let cap_scale = Vec3::new(cap_r, cap_r * 0.38, cap_r);
                        p.spawn((
                            Mesh3d(a.cap_mesh.clone()),
                            MeshMaterial3d(cap),
                            Transform::from_xyz(0.0, stalk_h, 0.0).with_scale(cap_scale),
                            Breathe {
                                base: cap_scale,
                                phase: breath,
                                amount: 0.06,
                            },
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
                            Breathe {
                                base: Vec3::new(cap_r * 0.92, cap_r * 0.12, cap_r * 0.92),
                                phase: breath,
                                amount: 0.08,
                            },
                            NotShadowCaster,
                        ));
                    })
                    .id();
                kids.push(e);
            }
        }
        Props::Bones => {
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
                    let gh = data.height_at(lp.x, lp.y);
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
        Props::Crystal => {
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
        Props::Monoliths => {
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
        Props::Grove => {
            // Bulb trees in loose stands, long grass between them, the odd
            // boulder.
            for _ in 0..11 {
                let (l, w, h) = spot(&mut rng);
                if h < SEA_LEVEL + 0.3 || !solid_ok(w) {
                    continue;
                }
                if rng.gen_bool(0.6) {
                    // An alea tree (forms.rs): every world grows its own.
                    kids.push(form_tree(commands, &mut rng, local(l, h)));
                } else {
                    let height = rng.gen_range(7.0..16.0);
                    kids.push(tree(commands, a, &mut rng, local(l, h), height));
                }
            }
            for _ in 0..14 {
                let (l, w, h) = spot(&mut rng);
                if h < SEA_LEVEL || !flat_ok(w) {
                    continue;
                }
                kids.push(grass(commands, a, &mut rng, local(l, h)));
            }
            for _ in 0..rng.gen_range(0..3) {
                let (l, w, h) = spot(&mut rng);
                if solid_ok(w) {
                    let size = rng.gen_range(1.2..3.0);
                    kids.push(boulder(commands, a, &mut rng, local(l, h), size));
                }
            }
        }
        Props::Peaks => {
            // Scree and spires; stunted trees on the lower slopes.
            for _ in 0..7 {
                let (l, w, h) = spot(&mut rng);
                if solid_ok(w) {
                    let size = rng.gen_range(1.5..5.0);
                    kids.push(boulder(commands, a, &mut rng, local(l, h), size));
                }
            }
            for _ in 0..rng.gen_range(1..4) {
                let (l, w, h) = spot(&mut rng);
                if solid_ok(w) && h > SEA_LEVEL + 5.0 {
                    let height = rng.gen_range(9.0..26.0);
                    kids.push(spire(commands, a, &mut rng, local(l, h), height));
                }
            }
            for _ in 0..3 {
                let (l, w, h) = spot(&mut rng);
                if h > SEA_LEVEL + 0.3 && h < 70.0 && solid_ok(w) {
                    let height = rng.gen_range(4.0..8.0);
                    kids.push(tree(commands, a, &mut rng, local(l, h), height));
                }
            }
        }
        Props::Marsh | Props::None => {
            // Jelly and glass trees on the dry patches of the marsh.
            for _ in 0..rng.gen_range(0..4) {
                let (l, w, h) = spot(&mut rng);
                if h > SEA_LEVEL + 0.4 && solid_ok(w) {
                    kids.push(form_tree(commands, &mut rng, local(l, h)));
                }
            }
        }
    }

    // --- the tribe and the beasts -------------------------------------------
    for site in sites
        .iter()
        .filter(|s| ((s.centre / CHUNK).floor().as_ivec2()) == coord)
    {
        crate::settlement::spawn(
            commands, meshes, materials, a, sa, t, site, origin, &mut kids,
        );
    }
    crate::fauna::spawn_in_chunk(
        commands,
        fa,
        t,
        &data,
        coord,
        centre_biome,
        &mut rng,
        &mut kids,
    );

    // --- a lone spire now and then, visible from far off ------------------------
    if rng.gen_bool(0.035) {
        let (l, w, h) = spot(&mut rng);
        if h > SEA_LEVEL + 0.5 && solid_ok(w) {
            kids.push(form_spire(commands, &mut rng, local(l, h)));
        }
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
        let slope = (data.height_at(l.x + 2.0, l.y) - data.height_at(l.x - 2.0, l.y)).abs()
            + (data.height_at(l.x, l.y + 2.0) - data.height_at(l.x, l.y - 2.0)).abs();
        if h > SEA_LEVEL + 0.3 && slope < 1.0 && solid_ok(w) {
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

    let growing = wg.grow_until > 0.0;
    for k in &kids[first_prop..] {
        // The prop's own scale is its size (`record_props` reads it once
        // it exists); growing in starts from nothing.
        commands.entity(*k).insert(Prop {
            base_scale: Vec3::NAN,
        });
        if growing {
            commands.entity(*k).insert(Grow(0.0));
        }
    }
    kids.insert(0, ground_e);
    commands.entity(root).add_children(&kids);
    root
}

/// A new prop's scale is its size: remember it before anything scales it.
fn record_props(mut q: Query<(&mut Prop, &Transform), Added<Prop>>) {
    for (mut p, tf) in &mut q {
        if p.base_scale.is_nan() {
            p.base_scale = tf.scale;
        }
    }
}

/// Props grow in from nothing, then are plain props again.
fn grow_props(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut Grow, &mut Transform, &Prop)>,
) {
    let dt = time.delta_secs();
    for (e, mut g, mut tf, p) in &mut q {
        if p.base_scale.is_nan() {
            continue;
        }
        g.0 = (g.0 + dt / 1.4).min(1.0);
        let s = 1.0 - (1.0 - g.0).powi(3);
        tf.scale = p.base_scale * s.max(0.001);
        if g.0 >= 1.0 {
            tf.scale = p.base_scale;
            commands.entity(e).remove::<Grow>();
        }
    }
}

/// A tree sculpted by alea, swaying like the others.
fn form_tree(commands: &mut Commands, rng: &mut StdRng, at: Vec3) -> Entity {
    let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
    let s = rng.gen_range(0.8..1.4);
    commands
        .spawn((
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::splat(s)),
            Visibility::default(),
            Sway {
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                amount: 0.02,
                yaw,
            },
            crate::forms::FormSlot {
                kind: crate::forms::FormKind::Tree,
                pick: rng.r#gen(),
                scale: s,
                solid: true,
            },
        ))
        .id()
}

/// A lone spire out in the wilds: a beacon on the skyline.
fn form_spire(commands: &mut Commands, rng: &mut StdRng, at: Vec3) -> Entity {
    let s = rng.gen_range(0.7..1.3);
    commands
        .spawn((
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(rng.gen_range(0.0..6.0)))
                .with_scale(Vec3::splat(s)),
            Visibility::default(),
            crate::forms::FormSlot {
                kind: crate::forms::FormKind::Tower,
                pick: rng.r#gen(),
                scale: s,
                solid: true,
            },
        ))
        .id()
}

/// A bulb tree: a tapering trunk and a cluster of round canopies, leaning
/// in the wind from the root.
fn tree(
    commands: &mut Commands,
    a: &WetlandAssets,
    rng: &mut StdRng,
    at: Vec3,
    height: f32,
) -> Entity {
    let leaf = a.leaf_mats[rng.gen_range(0..a.leaf_mats.len())].clone();
    let trunk_r = height * rng.gen_range(0.035..0.055);
    let yaw = rng.gen_range(0.0..std::f32::consts::TAU);

    commands
        .spawn((
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
            Swappable::sized(
                ModelKind::Tree,
                Vec3::new(height * 0.6, height, height * 0.6),
            )
            .variant(rng.r#gen()),
            Sway {
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                amount: 0.025,
                yaw,
            },
            Obstacle {
                radius: trunk_r + 0.6,
                below: 0.0,
                above: height * 0.75,
            },
        ))
        .with_children(|p| {
            p.spawn((
                Mesh3d(a.trunk_mesh.clone()),
                MeshMaterial3d(a.trunk_mat.clone()),
                Transform::from_xyz(0.0, height * 0.45, 0.0).with_scale(Vec3::new(
                    trunk_r,
                    height * 0.9,
                    trunk_r,
                )),
            ));
            let n = rng.gen_range(3..6);
            for k in 0..n {
                let r = height * rng.gen_range(0.16..0.28);
                let a0 = k as f32 / n as f32 * std::f32::consts::TAU + rng.gen_range(-0.4..0.4);
                let off = Vec3::new(a0.cos(), 0.0, a0.sin()) * r * rng.gen_range(0.3..0.9)
                    + Vec3::Y * (height * rng.gen_range(0.72..1.0));
                p.spawn((
                    Mesh3d(a.canopy_mesh.clone()),
                    MeshMaterial3d(leaf.clone()),
                    Transform::from_translation(off).with_scale(Vec3::new(
                        r,
                        r * rng.gen_range(0.7..1.0),
                        r,
                    )),
                    Breathe {
                        base: Vec3::new(r, r * 0.85, r),
                        phase: rng.gen_range(0.0..std::f32::consts::TAU),
                        amount: 0.02,
                    },
                ));
            }
        })
        .id()
}

fn boulder(
    commands: &mut Commands,
    a: &WetlandAssets,
    rng: &mut StdRng,
    at: Vec3,
    size: f32,
) -> Entity {
    let scale = Vec3::new(
        size * rng.gen_range(0.8..1.3),
        size * rng.gen_range(0.55..0.9),
        size * rng.gen_range(0.8..1.3),
    );
    let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
    let tilt = Quat::from_euler(
        EulerRot::YXZ,
        0.0,
        rng.gen_range(-0.3..0.3),
        rng.gen_range(-0.3..0.3),
    );
    commands
        .spawn((
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
            Swappable::sized(
                ModelKind::Boulder,
                Vec3::new(scale.x * 2.0, scale.y * 1.6, scale.z * 2.0),
            )
            .variant(rng.r#gen()),
            Obstacle {
                radius: scale.x.max(scale.z) * 0.9,
                below: 0.0,
                above: scale.y * 1.6,
            },
        ))
        .with_children(|p| {
            p.spawn((
                Mesh3d(a.rock_mesh.clone()),
                MeshMaterial3d(a.rock_mat.clone()),
                Transform::from_translation(Vec3::Y * scale.y * 0.55)
                    .with_rotation(tilt)
                    .with_scale(scale),
            ));
        })
        .id()
}

/// A rock spire: a needle of stone leaning a little.
fn spire(
    commands: &mut Commands,
    a: &WetlandAssets,
    rng: &mut StdRng,
    at: Vec3,
    height: f32,
) -> Entity {
    let r = height * rng.gen_range(0.1..0.16);
    let tilt = Quat::from_rotation_x(rng.gen_range(-0.12..0.12))
        * Quat::from_rotation_z(rng.gen_range(-0.12..0.12));
    let yaw = rng.gen_range(0.0..6.0);
    commands
        .spawn((
            Transform::from_translation(at).with_rotation(tilt * Quat::from_rotation_y(yaw)),
            Visibility::default(),
            Swappable::sized(ModelKind::Spire, Vec3::new(r * 2.0, height, r * 2.0))
                .variant(rng.r#gen()),
            Obstacle {
                radius: r + 0.5,
                below: 0.0,
                above: height * 0.95,
            },
        ))
        .with_children(|p| {
            p.spawn((
                Mesh3d(a.spire_mesh.clone()),
                MeshMaterial3d(a.rock_mat.clone()),
                Transform::from_translation(Vec3::Y * height * 0.5)
                    .with_scale(Vec3::new(r, height, r)),
            ));
        })
        .id()
}

/// A tuft of long grass: a short, wide reed clump in grass green.
fn grass(commands: &mut Commands, a: &WetlandAssets, rng: &mut StdRng, at: Vec3) -> Entity {
    let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
    let variant = rng.gen_range(0..a.reed_meshes.len());
    let (radius, height) = (rng.gen_range(1.2..2.6), rng.gen_range(0.8..1.5));
    commands
        .spawn((
            ReedClump {
                radius,
                height,
                solid: false,
                tilt: Vec2::ZERO,
                tilt_vel: Vec2::ZERO,
                phase: rng.gen_range(0.0..std::f32::consts::TAU),
                yaw,
            },
            Mesh3d(a.reed_meshes[variant].clone()),
            MeshMaterial3d(a.grass_mat.clone()),
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::new(radius, height, radius)),
            NotShadowCaster,
        ))
        .id()
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
    // Low enough to fly through: the bottom edge hangs 8–25 m up.
    let y = ground.y + rng.gen_range(38.0..52.0);
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

/// Builds chunks around the focus, nearest first, and drops far ones. The
/// sampling and painting run on the compute pool; entities are spawned when
/// a build lands.
fn stream_chunks(
    mut commands: Commands,
    wg: Option<ResMut<WorldGen>>,
    terrain: Option<Res<WorldTerrain>>,
    assets: Res<WetlandAssets>,
    tribe: Res<crate::settlement::SettlementAssets>,
    beasts: Res<crate::fauna::FaunaAssets>,
    mut art: ResMut<Art>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    focus: Query<&Transform, With<CameraTarget>>,
    cams: Query<&Transform, With<Camera3d>>,
) {
    let (Some(mut wg), Some(terrain)) = (wg, terrain) else {
        return;
    };
    if wg.paused {
        return;
    }
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
    wg.pending.retain(|c, _| keep.contains(c));

    // Land finished builds (polling a finished task takes its result).
    let mut done = Vec::new();
    for (c, task) in wg.pending.iter_mut() {
        if let Some(data) = block_on(future::poll_once(task)) {
            done.push((*c, data));
        }
    }
    for (c, data) in done {
        wg.pending.remove(&c);
        let e = spawn_chunk(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut images,
            &mut art,
            &assets,
            &tribe,
            &beasts,
            &terrain.0,
            &wg,
            data,
        );
        wg.loaded.insert(c, e);
    }

    // Start new builds, nearest first.
    for c in wanted {
        if wg.loaded.contains_key(&c) || wg.pending.contains_key(&c) {
            continue;
        }
        if wg.pending.len() >= IN_FLIGHT {
            break;
        }
        let chunk_centre = (c.as_vec2() + 0.5) * CHUNK;
        let sampler = terrain.0.sampler(chunk_centre, CHUNK);
        let task = AsyncComputeTaskPool::get().spawn(async move { build_chunk(sampler, c) });
        wg.pending.insert(c, task);
    }
}

/// Spores wander on slow loops and twinkle.
fn drift_spores(time: Res<Time>, mut q: Query<(&Spore, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (s, mut tf) in &mut q {
        let a = t * 0.25 + s.phase;
        tf.translation = s.base
            + Vec3::new(
                a.cos() * 1.6 + (a * 2.3).sin() * 0.4,
                (t * 0.7 + s.phase).sin() * 0.6,
                a.sin() * 1.2,
            );
        tf.scale = Vec3::splat(0.75 + 0.45 * ((t * 3.0 + s.phase * 5.0).sin() * 0.5 + 0.5));
    }
}

/// Moths circle their swarm's centre on wobbling loops, wings beating.
fn fly_moths(time: Res<Time>, mut q: Query<(&Moth, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (m, mut tf) in &mut q {
        let a = t * m.speed + m.phase;
        let r = m.radius * (1.0 + 0.25 * (t * 0.9 + m.phase).sin());
        let p = m.centre
            + Vec3::new(
                a.cos() * r,
                m.height + 0.6 * (t * 1.7 + m.phase).sin(),
                a.sin() * r,
            );
        let ahead = Vec3::new(-a.sin(), 0.0, a.cos()) * m.speed.signum();
        tf.translation = p;
        tf.look_to(ahead, Vec3::Y);
        let beat = (t * 22.0 + m.phase * 7.0).sin().abs();
        tf.scale = Vec3::new(1.6 * (0.25 + 0.75 * beat), 1.6, 1.6);
    }
}

fn breathe(time: Res<Time>, mut q: Query<(&Breathe, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (b, mut tf) in &mut q {
        let s = (t * 0.6 + b.phase).sin() * b.amount;
        tf.scale = b.base * Vec3::new(1.0 + s, 1.0 - s * 0.8, 1.0 + s);
    }
}

/// Shared glowing materials pulse: gills, crystals, and the water's sheen.
fn pulse_materials(
    time: Res<Time>,
    a: Res<WetlandAssets>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let t = time.elapsed_secs();
    if let Some(m) = mats.get_mut(&a.gill_mat) {
        let k = 2.2 + 1.4 * (t * 0.6).sin();
        m.emissive = Color::srgb(0.6, 1.0, 0.9).to_linear() * k;
    }
    for (i, h) in a.shard_mats.iter().enumerate() {
        if let Some(m) = mats.get_mut(h) {
            let c = m.base_color.to_linear();
            // Slow throb with a sharp glint now and then.
            let throb = 1.4 + 0.8 * (t * (0.7 + 0.3 * i as f32)).sin();
            let glint = ((t * 0.37 + i as f32 * 1.7).sin().max(0.0)).powi(24) * 4.0;
            m.emissive = LinearRgba::rgb(c.red, c.green, c.blue) * (throb + glint);
        }
    }
    if let Some(m) = mats.get_mut(&a.water_mat) {
        let k = 0.18 + 0.06 * (t * 0.8).sin();
        m.emissive = LinearRgba::new(0.02 * k * 5.0, k, k * 1.1, 1.0);
    }
}

/// Wave height and slope at a point in the water mesh (chunk-local x, z,
/// centred). Every wave repeats exactly over a chunk, so neighbouring
/// chunks' water meets seamlessly.
pub fn water_wave(x: f32, z: f32, t: f32) -> (f32, f32, f32) {
    let k = std::f32::consts::TAU / CHUNK;
    let waves = [
        (3.0, 2.0, 0.07, 0.9),
        (-2.0, 5.0, 0.05, 1.3),
        (7.0, -4.0, 0.025, 2.1),
        (-9.0, -11.0, 0.015, 2.9),
    ];
    let (mut h, mut dx, mut dz) = (0.0, 0.0, 0.0);
    for (nx, nz, amp, w) in waves {
        let ph = k * (nx * x + nz * z) - w * t;
        h += amp * ph.sin();
        dx += amp * k * nx * ph.cos();
        dz += amp * k * nz * ph.cos();
    }
    (h, dx, dz)
}

fn animate_water(time: Res<Time>, a: Res<WetlandAssets>, mut meshes: ResMut<Assets<Mesh>>) {
    let t = time.elapsed_secs();
    let Some(mesh) = meshes.get_mut(&a.water_mesh) else {
        return;
    };
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(ps)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).cloned()
    else {
        return;
    };
    let mut positions = Vec::with_capacity(ps.len());
    let mut normals = Vec::with_capacity(ps.len());
    for p in ps {
        let (h, dx, dz) = water_wave(p[0], p[2], t);
        positions.push([p[0], h, p[2]]);
        normals.push(Vec3::new(-dx, 1.0, -dz).normalize().to_array());
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
}

/// Trees lean with the gusts, the same wind the reeds feel.
fn sway_trees(time: Res<Time>, mut q: Query<(&Sway, &mut Transform, &GlobalTransform)>) {
    let t = time.elapsed_secs();
    let wind_dir = Vec2::new(0.8, 0.6);
    for (s, mut tf, gt) in &mut q {
        let w = gt.translation().xz();
        let gust = ((w.dot(wind_dir) / 38.0 - t * 1.1).sin() * 0.5 + 0.5).powi(4);
        let lean = wind_dir * (s.amount * (0.4 + 1.6 * gust))
            + Vec2::new((t * 0.8 + s.phase).sin(), (t * 0.6 + s.phase).cos()) * s.amount * 0.5;
        tf.rotation = Quat::from_rotation_z(-lean.x)
            * Quat::from_rotation_x(lean.y)
            * Quat::from_rotation_y(s.yaw);
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
fn sway_reeds(time: Res<Time>, mut q: Query<(&mut ReedClump, &mut Transform, &GlobalTransform)>) {
    let dt = time.delta_secs().min(1.0 / 20.0);
    let t = time.elapsed_secs();
    // Gusts: crests of wind rolling across the marsh, visible as waves
    // running through the reed beds.
    let wind_dir = Vec2::new(0.8, 0.6);
    for (mut reed, mut tf, gt) in &mut q {
        let w = gt.translation().xz();
        let along = w.dot(wind_dir);
        let gust = ((along / 38.0 - t * 1.1).sin() * 0.5 + 0.5).powi(4)
            * (0.6 + 0.4 * (w.dot(Vec2::new(-0.6, 0.8)) / 90.0 + t * 0.2).sin());
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
        ) + wind_dir * gust * if reed.solid { 0.14 } else { 0.26 };
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
            (
                record_props,
                grow_props.after(record_props),
                drift_spores,
                sway_reeds,
                sway_trees,
                float_landmarks,
                fly_moths,
                breathe,
                pulse_materials,
                animate_water,
            )
                .in_set(crate::GameSet::Simulation),
        )
        .add_systems(Update, stream_chunks.in_set(crate::GameSet::Camera));
    }
}

#[cfg(test)]
mod water_tests {
    use super::*;

    #[test]
    fn waves_meet_across_chunks() {
        let h = CHUNK * 0.5;
        for i in 0..20 {
            let (z, t) = (i as f32 * 5.3 - 50.0, i as f32 * 0.37);
            let (a, _, _) = water_wave(-h, z, t);
            let (b, _, _) = water_wave(h, z, t);
            assert!((a - b).abs() < 1e-4, "seam along x at z={z}: {a} vs {b}");
            let (c, _, _) = water_wave(z, -h, t);
            let (d, _, _) = water_wave(z, h, t);
            assert!((c - d).abs() < 1e-4, "seam along z at x={z}");
        }
        assert_ne!(
            water_wave(3.0, 4.0, 0.0).0,
            water_wave(3.0, 4.0, 1.0).0,
            "waves move"
        );
    }
}

#[cfg(test)]
mod bench {
    /// `cargo test chunk_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn chunk_cost() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/alea/land");
        let t = crate::terrain::Terrain::from_dir(2026, &dir);
        let start = std::time::Instant::now();
        let n = 12;
        for i in 0..n {
            let c = bevy::math::IVec2::new(i * 7 - 40, i * 3);
            let s = t.sampler((c.as_vec2() + 0.5) * super::CHUNK, super::CHUNK);
            let _ = super::build_chunk(s, c);
        }
        eprintln!(
            "chunk build (heights + {0}x{0} painting): {1:?} per chunk",
            super::TEXELS,
            start.elapsed() / n as u32
        );
        // Split: heights only vs. one texel row of colour.
        let c = bevy::math::IVec2::new(3, -2);
        let mut sm = t.sampler((c.as_vec2() + 0.5) * super::CHUNK, super::CHUNK);
        let t0 = std::time::Instant::now();
        let mut acc = 0.0;
        for i in 0..4096 {
            acc += sm.height(
                c.x as f32 * super::CHUNK + (i % 64) as f32 * 2.0,
                c.y as f32 * super::CHUNK + (i / 64) as f32 * 2.0,
            );
        }
        let th = t0.elapsed();
        let t0 = std::time::Instant::now();
        let mut acc2 = 0u32;
        for i in 0..4096 {
            acc2 += sm.color(
                c.x as f32 * super::CHUNK + (i % 64) as f32 * 2.0,
                c.y as f32 * super::CHUNK + (i / 64) as f32 * 2.0,
            )[0] as u32;
        }
        let tc = t0.elapsed();
        eprintln!(
            "4096 heights {th:?} ({:.2} us each), 4096 colours {tc:?} ({:.2} us each) [{acc:.0} {acc2}]",
            th.as_secs_f64() * 1e6 / 4096.0,
            tc.as_secs_f64() * 1e6 / 4096.0
        );
    }
}
