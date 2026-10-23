//! Forms: buildings, trees and vehicles sculpted by alea.
//!
//! Every work in `assets/alea/forms/<kind>/` is a shape, read from its first
//! `render sculpture`: the field is evaluated straight into a mesh, sized in
//! metres by the render's `side` and `relief` (alea's millimetres become the
//! game's metres). Its first `render image` paints it, its second (if any)
//! lights it. Each world draws a few editions of every work, so no two
//! towers, trees or craft need be alike, and none of it costs anything.
//!
//! Two readings:
//!
//! - **lathe** (towers, trees): the shape turns round a vertical axis.
//!   `x` goes round (0–1), `y` runs from the top (0) to the foot (1), and the
//!   field is the radius, 0–1 of `side / 2`; the height is `relief × side`.
//! - **relief** (arcologies, vehicles): seen from above, `x`, `y` across a
//!   `side × side` footprint, the field the height, 0–1 of `relief × side`.
//!   Where it is zero there is nothing. Vehicles are mirrored underneath.
//!
//! Meshes are built on the compute pool when a world starts; anything that
//! wants a form spawns with a `FormSlot` and is dressed when it is ready.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alea::compile::Program;
use alea::eval;
use alea::library::{self, Module};
use alea::media;
use bevy::asset::RenderAssetUsages;
use bevy::asset::io::file::FileAssetReader;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::terrain::hash;
use crate::world::Obstacle;

/// Editions of every work drawn per world.
pub const EDITIONS: u32 = 4;
const PAINT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormKind {
    Tower,
    Tree,
    Arcology,
    Vehicle,
}

impl FormKind {
    pub const ALL: [FormKind; 4] = [
        FormKind::Tower,
        FormKind::Tree,
        FormKind::Arcology,
        FormKind::Vehicle,
    ];

    fn dir(self) -> &'static str {
        match self {
            FormKind::Tower => "tower",
            FormKind::Tree => "tree",
            FormKind::Arcology => "arcology",
            FormKind::Vehicle => "vehicle",
        }
    }

    fn lathe(self) -> bool {
        matches!(self, FormKind::Tower | FormKind::Tree)
    }
}

pub struct FormWork {
    pub kind: FormKind,
    pub name: String,
    module: Arc<Module>,
}

/// A form as built: its mesh, its paint and light, and its size in metres
/// (width, height, depth).
pub struct FormData {
    pub mesh: Mesh,
    pub paint: Option<Image>,
    pub glow: Option<Image>,
    pub size: Vec3,
}

struct Built {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    size: Vec3,
}

#[derive(Resource, Default)]
pub struct Forms {
    pub works: Vec<FormWork>,
    built: HashMap<(usize, u32), Built>,
    pending: HashMap<(usize, u32), Task<Result<FormData, String>>>,
    failed: Vec<String>,
}

impl Forms {
    /// Works of a kind, by index.
    fn of(&self, kind: FormKind) -> Vec<usize> {
        (0..self.works.len())
            .filter(|i| self.works[*i].kind == kind)
            .collect()
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn ready(&self) -> usize {
        self.built.len()
    }

    /// A new world: drop the old editions and start drawing this one's.
    pub fn reset(&mut self, seed: u64, depth: u32) {
        self.built.clear();
        self.pending.clear();
        let pool = AsyncComputeTaskPool::get();
        for (i, w) in self.works.iter().enumerate() {
            for k in 0..EDITIONS {
                let module = w.module.clone();
                let kind = w.kind;
                let s = hash(&[seed as i64, i as i64 * 101, k as i64]) & 0xffff_ffff;
                self.pending.insert(
                    (i, k),
                    pool.spawn(async move { build(&module, kind, s, depth) }),
                );
            }
        }
    }
}

pub fn forms_dir() -> PathBuf {
    FileAssetReader::get_base_path()
        .join("assets")
        .join("alea")
        .join("forms")
}

/// Loads every form work under `dir`.
pub fn load_works(dir: &Path) -> Vec<FormWork> {
    let mut works = Vec::new();
    for kind in FormKind::ALL {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(kind.dir()))
            .map(|d| {
                d.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "alea"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        for path in files {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            match library::load(&path.to_string_lossy()) {
                Ok(module) => works.push(FormWork { kind, name, module }),
                Err(e) => warn!("alea form {}: {}", path.display(), e.display("", "")),
            }
        }
    }
    works
}

/// Draws one edition of a form work and builds its mesh and paintings.
pub fn build(module: &Module, kind: FormKind, seed: u64, depth: u32) -> Result<FormData, String> {
    let show = |e: alea::error::Error| e.display(&module.src, &module.file);
    let mut given = HashMap::new();
    given.insert(
        "depth".to_string(),
        eval::Val::Field(eval::Field::Const(depth as f64)),
    );
    let edition = eval::resolve_with(module, seed, &given).map_err(show)?;
    media::validate(&edition).map_err(show)?;
    let shape_r = edition
        .renders
        .iter()
        .find(|r| r.kind == "sculpture")
        .ok_or("a form needs a `render sculpture` for its shape")?;
    let spec = media::sculpture_spec(shape_r).map_err(show)?;
    let field = shape_r.required_field(0, "the shape").map_err(show)?;
    let prog = Program::new(field, edition.seed);
    let mut slots = prog.slots(spec.at);
    let mut f = |u: f64, v: f64| -> f32 {
        let x = prog.run(&mut slots, u, v);
        if x.is_finite() {
            (x as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let side = spec.side as f32;
    let height = (spec.relief * spec.side) as f32;
    let (mesh, size) = if kind.lathe() {
        lathe(&mut f, side, height)
    } else {
        relief(
            &mut f,
            side,
            height,
            spec.n.min(96),
            kind == FormKind::Vehicle,
        )
    };

    let images: Vec<&eval::Render> = edition
        .renders
        .iter()
        .filter(|r| r.kind == "image")
        .collect();
    let paint = |r: &eval::Render| -> Result<Image, String> {
        let spec = media::image_spec(r).map_err(show)?;
        let field = r.required_field(0, "the painting").map_err(show)?;
        let prog = Program::new(field, edition.seed);
        let mut slots = prog.slots(spec.at);
        let (lo, hi) = spec.range.unwrap_or((0.0, 1.0));
        let mut px = Vec::with_capacity(PAINT * PAINT * 4);
        for j in 0..PAINT {
            for i in 0..PAINT {
                let v = prog.run(
                    &mut slots,
                    (i as f64 + 0.5) / PAINT as f64,
                    (j as f64 + 0.5) / PAINT as f64,
                );
                let t = ((v - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0);
                let t = if t.is_finite() { t } else { 0.0 };
                let c = spec.stops.color(t);
                px.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let mut img = Image::new(
            Extent3d {
                width: PAINT as u32,
                height: PAINT as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            px,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        Ok(img)
    };
    let paint_img = images.first().map(|r| paint(r)).transpose()?;
    let glow_img = images.get(1).map(|r| paint(r)).transpose()?;
    Ok(FormData {
        mesh,
        paint: paint_img,
        glow: glow_img,
        size,
    })
}

const AROUND: usize = 40;
const UP: usize = 64;

/// A shape turned round the vertical axis: radius from the field.
fn lathe(f: &mut impl FnMut(f64, f64) -> f32, side: f32, height: f32) -> (Mesh, Vec3) {
    let radius = side * 0.5;
    let mut positions = Vec::with_capacity((AROUND + 1) * (UP + 1) + 1);
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut widest: f32 = 0.0;
    let mut rings: Vec<Vec<f32>> = Vec::with_capacity(UP + 1);
    for j in 0..=UP {
        let v = j as f64 / UP as f64; // 0 at the foot
        let mut ring = Vec::with_capacity(AROUND);
        for i in 0..AROUND {
            ring.push(f(i as f64 / AROUND as f64, 1.0 - v));
        }
        rings.push(ring);
    }
    for (j, ring) in rings.iter().enumerate() {
        let v = j as f32 / UP as f32;
        for i in 0..=AROUND {
            let r = ring[i % AROUND] * radius;
            widest = widest.max(r);
            let a = i as f32 / AROUND as f32 * std::f32::consts::TAU;
            positions.push([r * a.cos(), v * height, r * a.sin()]);
            uvs.push([i as f32 / AROUND as f32, 1.0 - v]);
        }
    }
    let mut indices = Vec::with_capacity(AROUND * UP * 6 + AROUND * 3);
    let row = AROUND as u32 + 1;
    for j in 0..UP as u32 {
        for i in 0..AROUND as u32 {
            let a = j * row + i;
            let (b, c, d) = (a + 1, a + row, a + row + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    // Close the top.
    let tip = positions.len() as u32;
    positions.push([0.0, height, 0.0]);
    uvs.push([0.5, 0.0]);
    let top = UP as u32 * row;
    for i in 0..AROUND as u32 {
        indices.extend_from_slice(&[top + i, tip, top + i + 1]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices));
    mesh.compute_smooth_normals();
    (mesh, Vec3::new(widest * 2.0, height, widest * 2.0))
}

/// A height map seen from above, sharp-edged; nothing where it is zero.
fn relief(
    f: &mut impl FnMut(f64, f64) -> f32,
    side: f32,
    height: f32,
    n: usize,
    mirrored: bool,
) -> (Mesh, Vec3) {
    let n = n.max(8);
    let mut h = vec![0.0f32; (n + 1) * (n + 1)];
    for j in 0..=n {
        for i in 0..=n {
            h[j * (n + 1) + i] = f(i as f64 / n as f64, j as f64 / n as f64);
        }
    }
    let eps = 0.01;
    let at = |i: usize, j: usize| h[j * (n + 1) + i];
    let point = |i: usize, j: usize, sign: f32| -> ([f32; 3], [f32; 2]) {
        let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
        let y = at(i, j) * height * sign;
        ([(u - 0.5) * side, y, (v - 0.5) * side], [u, v])
    };
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    let mut tri = |ps: [([f32; 3], [f32; 2]); 3]| {
        for (p, uv) in ps {
            lo = lo.min(Vec3::from(p));
            hi = hi.max(Vec3::from(p));
            positions.push(p);
            uvs.push(uv);
        }
    };
    for j in 0..n {
        for i in 0..n {
            let corners = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            if corners.iter().all(|c| *c < eps) {
                continue;
            }
            for (sign, flip) in [(1.0, false), (-0.35, true)] {
                if flip && !mirrored {
                    continue;
                }
                let (a, b, c, d) = (
                    point(i, j, sign),
                    point(i + 1, j, sign),
                    point(i + 1, j + 1, sign),
                    point(i, j + 1, sign),
                );
                if flip {
                    tri([a, b, c]);
                    tri([a, c, d]);
                } else {
                    tri([a, c, b]);
                    tri([a, d, c]);
                }
            }
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.compute_flat_normals();
    let size = if lo.x <= hi.x { hi - lo } else { Vec3::ONE };
    (mesh, size)
}

/// Wants a form: dressed with an edition's mesh when one is ready. `pick`
/// chooses the work and the edition; `scale` sizes it; `solid` makes it an
/// obstacle the size of its footprint.
#[derive(Component, Debug, Clone, Copy)]
pub struct FormSlot {
    pub kind: FormKind,
    pub pick: u64,
    pub scale: f32,
    pub solid: bool,
}

/// Dressed: the form's size in metres, scaled (read by tests and tools).
#[derive(Component, Debug, Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct Dressed(pub Vec3);

fn setup(mut commands: Commands) {
    let works = load_works(&forms_dir());
    if !works.is_empty() {
        info!(
            "alea forms: {}",
            works
                .iter()
                .map(|w| format!("{}/{}", w.kind.dir(), w.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    commands.insert_resource(Forms { works, ..default() });
}

/// Lands finished builds as meshes and materials.
fn land(
    mut forms: ResMut<Forms>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut done = Vec::new();
    for (key, task) in forms.pending.iter_mut() {
        if let Some(r) = block_on(future::poll_once(task)) {
            done.push((*key, r));
        }
    }
    for (key, r) in done {
        forms.pending.remove(&key);
        match r {
            Ok(data) => {
                let glow = data.glow.map(|g| images.add(g));
                let material = materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: data.paint.map(|p| images.add(p)),
                    emissive: if glow.is_some() {
                        LinearRgba::rgb(2.5, 2.5, 2.5)
                    } else {
                        LinearRgba::BLACK
                    },
                    emissive_texture: glow,
                    perceptual_roughness: 0.35,
                    metallic: 0.2,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
                forms.built.insert(
                    key,
                    Built {
                        mesh: meshes.add(data.mesh),
                        material,
                        size: data.size,
                    },
                );
            }
            Err(msg) => {
                let name = forms.works[key.0].name.clone();
                if !forms.failed.contains(&name) {
                    warn!("alea form `{name}` could not be drawn:\n{msg}");
                    forms.failed.push(name);
                }
            }
        }
    }
}

/// Puts a built edition on every slot waiting for one.
fn dress(
    mut commands: Commands,
    forms: Res<Forms>,
    slots: Query<(Entity, &FormSlot), Without<Dressed>>,
) {
    if forms.built.is_empty() {
        return;
    }
    for (e, slot) in &slots {
        let works = forms.of(slot.kind);
        if works.is_empty() {
            // No works of this kind: leave the slot as it is.
            commands.entity(e).insert(Dressed(Vec3::ZERO));
            continue;
        }
        let w = works[(slot.pick % works.len() as u64) as usize];
        let k = ((slot.pick / works.len() as u64) % EDITIONS as u64) as u32;
        // The edition asked for, or any built edition of that work.
        let built = forms
            .built
            .get(&(w, k))
            .or_else(|| (0..EDITIONS).find_map(|k| forms.built.get(&(w, k))));
        let Some(b) = built else {
            continue;
        };
        let size = b.size * slot.scale;
        commands.entity(e).insert((
            Mesh3d(b.mesh.clone()),
            MeshMaterial3d(b.material.clone()),
            Dressed(size),
        ));
        if slot.solid {
            commands.entity(e).insert(Obstacle {
                radius: size.x.max(size.z) * 0.42,
                below: 0.0,
                above: size.y,
            });
        }
    }
}

/// Each world (and each reality) draws its own editions.
fn follow_world(
    wg: Option<Res<crate::world::WorldGen>>,
    terrain: Option<Res<crate::world::WorldTerrain>>,
    mut forms: ResMut<Forms>,
    mut drawn: Local<Option<u64>>,
) {
    let (Some(wg), Some(terrain)) = (wg, terrain) else {
        return;
    };
    if *drawn != Some(wg.seed) {
        *drawn = Some(wg.seed);
        forms.reset(wg.seed, terrain.0.land.depth);
    }
}

pub struct FormsPlugin;

impl Plugin for FormsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            (follow_world, land, dress)
                .chain()
                .run_if(resource_exists::<Forms>)
                .in_set(crate::GameSet::Simulation),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/alea/forms")
    }

    /// Every shipped form draws, in several editions, into a sane mesh of
    /// the size its kind promises.
    #[test]
    fn every_form_draws() {
        let works = load_works(&dir());
        for kind in FormKind::ALL {
            assert!(
                works.iter().filter(|w| w.kind == kind).count() >= 2,
                "fewer than two {kind:?} works"
            );
        }
        for w in &works {
            let mut sizes = Vec::new();
            for seed in [1u64, 2, 3] {
                let d =
                    build(&w.module, w.kind, seed, 0).unwrap_or_else(|e| panic!("{}: {e}", w.name));
                let n = d.mesh.count_vertices();
                assert!(n > 30, "{}: only {n} vertices", w.name);
                assert!(d.paint.is_some(), "{}: no painting", w.name);
                assert!(
                    d.size.is_finite() && d.size.min_element() > 0.1,
                    "{}: size {:?}",
                    w.name,
                    d.size
                );
                sizes.push(d.size);
            }
            let (lo, hi) = match w.kind {
                FormKind::Tower => (40.0, 260.0),
                FormKind::Tree => (4.0, 30.0),
                FormKind::Arcology => (25.0, 260.0),
                FormKind::Vehicle => (0.3, 5.0),
            };
            for s in &sizes {
                assert!(
                    (lo..hi).contains(&s.y),
                    "{} {:?}: height {:.1} m",
                    w.name,
                    w.kind,
                    s.y
                );
            }
            assert!(
                sizes.windows(2).any(|p| p[0] != p[1]),
                "{}: editions are all alike",
                w.name
            );
        }
    }
}
