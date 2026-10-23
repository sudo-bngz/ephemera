//! alea in the game: the world's landmarks are painted at runtime by works
//! written in alea (https://github.com/sudo-bngz/alea), in `assets/alea/`.
//!
//! Each landmark asks for an *edition*: a work plus a seed derived from the
//! world seed and where the landmark stands. The same world always shows the
//! same editions; another world, other editions. Editions are rendered on
//! background threads, cached on disk, and animated by swapping frames on a
//! material shared by every landmark that shows that edition.
//!
//! If a work can't be loaded or drawn (a missing file, an edited work with an
//! error), the landmark keeps a dark placeholder and the error is logged once:
//! the game never stops for art.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::asset::io::file::FileAssetReader;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::terrain::hash;

/// The alea works the world uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Work {
    /// Carved, animated faces of floating obelisks.
    Monolith,
    /// Aurora curtains in the sky (additive).
    Veil,
    /// Glowing circles in the mud (additive).
    Sigil,
    /// The marbled skin of drifting spheres.
    Orb,
}

impl Work {
    #[cfg(test)]
    pub const ALL: [Work; 4] = [Work::Monolith, Work::Veil, Work::Sigil, Work::Orb];

    pub fn file(self) -> &'static str {
        match self {
            Work::Monolith => "monolith.alea",
            Work::Veil => "veil.alea",
            Work::Sigil => "sigil.alea",
            Work::Orb => "orb.alea",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Work::Monolith => "Monolith",
            Work::Veil => "Veil",
            Work::Sigil => "Sigil",
            Work::Orb => "Orb",
        }
    }

    /// Texture size in game: smaller than the work's own render size, plenty
    /// at the distances these are seen from, and quick to draw.
    fn size(self) -> (usize, usize) {
        match self {
            Work::Monolith => (128, 384),
            Work::Veil => (256, 128),
            Work::Sigil => (512, 512),
            Work::Orb => (256, 256),
        }
    }

    fn frames(self) -> usize {
        match self {
            Work::Sigil => 1,
            Work::Monolith => 12,
            Work::Veil => 16,
            Work::Orb => 12,
        }
    }

    /// Animation loop length in seconds.
    fn period(self) -> f32 {
        match self {
            Work::Monolith => 4.0,
            Work::Veil => 6.0,
            Work::Sigil => 1.0,
            Work::Orb => 5.0,
        }
    }

    fn material(self) -> StandardMaterial {
        match self {
            Work::Monolith => StandardMaterial {
                base_color: Color::srgb(0.25, 0.24, 0.28),
                emissive: LinearRgba::BLACK,
                perceptual_roughness: 0.35,
                metallic: 0.4,
                ..default()
            },
            Work::Orb => StandardMaterial {
                base_color: Color::srgb(0.5, 0.5, 0.5),
                emissive: LinearRgba::BLACK,
                perceptual_roughness: 0.25,
                metallic: 0.15,
                ..default()
            },
            // Additive: black is transparent, so a placeholder is invisible.
            Work::Veil | Work::Sigil => StandardMaterial {
                base_color: Color::BLACK,
                alpha_mode: AlphaMode::Add,
                unlit: true,
                fog_enabled: false,
                double_sided: true,
                cull_mode: None,
                ..default()
            },
        }
    }

    /// Wire finished frames into the material.
    fn apply(self, m: &mut StandardMaterial, frame: Handle<Image>) {
        match self {
            Work::Monolith => {
                m.base_color = Color::WHITE;
                m.base_color_texture = Some(frame.clone());
                m.emissive = LinearRgba::rgb(2.2, 2.2, 2.2);
                m.emissive_texture = Some(frame);
            }
            Work::Orb => {
                m.base_color = Color::WHITE;
                m.base_color_texture = Some(frame.clone());
                m.emissive = LinearRgba::rgb(1.1, 1.1, 1.1);
                m.emissive_texture = Some(frame);
            }
            Work::Veil => {
                m.base_color = Color::linear_rgb(0.9, 0.9, 0.9);
                m.base_color_texture = Some(frame);
            }
            Work::Sigil => {
                m.base_color = Color::linear_rgb(0.75, 0.75, 0.75);
                m.base_color_texture = Some(frame);
            }
        }
    }
}

/// How many distinct editions of each work one world shows. Landmarks share
/// them, which bounds the rendering work however far you travel.
pub const EDITIONS_PER_WORK: u64 = 12;

/// An edition's identity, as alea would print it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditionId {
    pub work: Work,
    pub seed: u64,
}

impl EditionId {
    pub fn label(&self) -> String {
        format!("{} — edition 0x{:x}", self.work.label(), self.seed)
    }
}

/// Marks a landmark entity with the edition it shows (for the HUD).
#[derive(Component, Clone, Copy, Debug)]
pub struct Landmark(pub EditionId);

type Frames = Result<Vec<Vec<u8>>, String>;

enum State {
    Rendering(Task<Frames>),
    Ready,
    Failed,
}

struct Edition {
    id: EditionId,
    material: Handle<StandardMaterial>,
    frames: Vec<Handle<Image>>,
    shown: usize,
    state: State,
}

#[derive(Resource)]
pub struct Art {
    world_seed: u64,
    editions: HashMap<(Work, u64), Edition>,
    dir: PathBuf,
    cache: Option<PathBuf>,
}

impl Art {
    fn new() -> Self {
        Self {
            world_seed: 0,
            editions: HashMap::new(),
            dir: FileAssetReader::get_base_path().join("assets").join("alea"),
            cache: cache_dir(),
        }
    }

    /// A new world: forget the old editions (their materials die with the
    /// entities that used them).
    pub fn reset(&mut self, world_seed: u64) {
        self.world_seed = world_seed;
        self.editions.clear();
    }

    /// The material for edition `slot` of `work`, starting its rendering if
    /// it's the first time it is asked for.
    pub fn edition(
        &mut self,
        work: Work,
        slot: u64,
        materials: &mut Assets<StandardMaterial>,
    ) -> (Handle<StandardMaterial>, EditionId) {
        let slot = slot % EDITIONS_PER_WORK;
        let seed = hash(&[self.world_seed as i64, work as i64, slot as i64]) & 0xffff_ffff;
        let id = EditionId { work, seed };
        let (dir, cache) = (self.dir.clone(), self.cache.clone());
        let e = self.editions.entry((work, slot)).or_insert_with(|| {
            let path = dir.join(work.file());
            let task = AsyncComputeTaskPool::get().spawn(async move {
                let (w, h) = work.size();
                cached_render(&path, cache.as_deref(), seed, w, h, work.frames())
            });
            Edition {
                id,
                material: materials.add(work.material()),
                frames: Vec::new(),
                shown: usize::MAX,
                state: State::Rendering(task),
            }
        });
        (e.material.clone(), e.id)
    }

    /// Editions still rendering (for tests and a loading hint).
    pub fn pending(&self) -> usize {
        self.editions
            .values()
            .filter(|e| matches!(e.state, State::Rendering(_)))
            .count()
    }

    pub fn ready(&self) -> usize {
        self.editions
            .values()
            .filter(|e| matches!(e.state, State::Ready))
            .count()
    }

    pub fn failed(&self) -> usize {
        self.editions
            .values()
            .filter(|e| matches!(e.state, State::Failed))
            .count()
    }
}

/// Draws `frames` frames of the work's first render, as RGBA, through the
/// alea library. A video is sampled evenly over its loop (alea works built
/// on `t` loop seamlessly); an image gives one frame.
pub fn render_frames(path: &Path, seed: u64, w: usize, h: usize, frames: usize) -> Frames {
    use alea::{eval, library, media};
    let file = path.to_string_lossy().into_owned();
    let module = library::load(&file).map_err(|e| e.display("", &file))?;
    let show = |e: alea::error::Error| e.display(&module.src, &module.file);
    let ed = eval::resolve(&module, seed).map_err(show)?;
    media::validate(&ed).map_err(show)?;
    let r = ed.renders.first().ok_or("the work has no render")?;
    let mut out = Vec::with_capacity(frames);
    match r.kind.as_str() {
        "image" => {
            let s = media::image_spec(r).map_err(show)?;
            let rgb = r
                .visual("the field to paint")
                .map_err(show)?
                .rgb(w, h, s.at, ed.seed, &s.stops, s.range);
            out.push(rgba(&rgb));
        }
        "video" => {
            let s = media::video_spec(r).map_err(show)?;
            let visual = r.visual("the field to animate").map_err(show)?;
            // Colour bounds once for the whole loop, so frames don't flicker.
            // (Works should give `range:` — measuring is the slow part.)
            let bounds = match (&visual, s.range) {
                (media::Visual::Paint(_), _) => None,
                (_, Some(b)) => Some(b),
                (_, None) => {
                    let f = r.required_field(0, "the field").map_err(show)?;
                    Some(media::animation_bounds(f, 64, 64, ed.seed))
                }
            };
            for k in 0..frames {
                let t = k as f64 / frames as f64;
                let rgb = match &visual {
                    media::Visual::Paint(p) => alea::paint::frame_rgb(p, w, h, t),
                    _ => {
                        let f = r.required_field(0, "the field").map_err(show)?;
                        media::frame_rgb(f, w, h, t, ed.seed, &s.stops, bounds)
                    }
                };
                out.push(rgba(&rgb));
            }
        }
        k => return Err(format!("a `{k}` render has no picture")),
    }
    Ok(out)
}

fn rgba(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for px in rgb.chunks_exact(3) {
        out.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    out
}

fn cache_dir() -> Option<PathBuf> {
    if std::env::var_os("EPHEMERA_NO_ART_CACHE").is_some() {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".cache")))
    }?;
    Some(base.join("ephemera").join("alea"))
}

/// `render_frames`, with a disk cache keyed by the work's source text, the
/// seed and the size: editing a work invalidates its cached editions.
fn cached_render(
    path: &Path,
    cache: Option<&Path>,
    seed: u64,
    w: usize,
    h: usize,
    frames: usize,
) -> Frames {
    let src = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut key = 0xcbf2_9ce4_8422_2325u64;
    for b in &src {
        key = (key ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    let key = hash(&[key as i64, seed as i64, w as i64, h as i64, frames as i64]);
    let file = cache.map(|c| c.join(format!("{key:016x}.rgba")));
    let frame_len = w * h * 4;

    if let Some(f) = &file {
        if let Ok(bytes) = std::fs::read(f) {
            if bytes.len() == frame_len * frames {
                return Ok(bytes.chunks(frame_len).map(<[u8]>::to_vec).collect());
            }
        }
    }
    let out = render_frames(path, seed, w, h, frames)?;
    if let (Some(f), Some(dir)) = (&file, cache) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(f, out.concat());
    }
    Ok(out)
}

/// Collects finished renders into textures.
fn collect(
    mut art: ResMut<Art>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for e in art.editions.values_mut() {
        let State::Rendering(task) = &mut e.state else {
            continue;
        };
        let Some(result) = block_on(future::poll_once(task)) else {
            continue;
        };
        match result {
            Ok(frames) => {
                let (w, h) = e.id.work.size();
                e.frames = frames
                    .into_iter()
                    .map(|data| {
                        images.add(Image::new(
                            Extent3d {
                                width: w as u32,
                                height: h as u32,
                                depth_or_array_layers: 1,
                            },
                            TextureDimension::D2,
                            data,
                            TextureFormat::Rgba8UnormSrgb,
                            RenderAssetUsages::RENDER_WORLD,
                        ))
                    })
                    .collect();
                if let (Some(m), Some(first)) = (materials.get_mut(&e.material), e.frames.first()) {
                    e.id.work.apply(m, first.clone());
                    e.shown = 0;
                }
                e.state = State::Ready;
            }
            Err(msg) => {
                warn!(
                    "alea: {} 0x{:x} could not be drawn:\n{msg}",
                    e.id.work.file(),
                    e.id.seed
                );
                e.state = State::Failed;
            }
        }
    }
}

/// Steps animated editions through their frames.
fn animate(time: Res<Time>, mut art: ResMut<Art>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let now = time.elapsed_secs();
    for e in art.editions.values_mut() {
        if e.frames.len() < 2 {
            continue;
        }
        let n = e.frames.len();
        // Offset each edition so they don't all pulse in step.
        let phase = (e.id.seed % 997) as f32 / 997.0;
        let k = (((now / e.id.work.period() + phase) % 1.0) * n as f32) as usize % n;
        if k != e.shown {
            if let Some(m) = materials.get_mut(&e.material) {
                e.id.work.apply(m, e.frames[k].clone());
                e.shown = k;
            }
        }
    }
}

pub struct ArtPlugin;

impl Plugin for ArtPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Art::new()).add_systems(
            Update,
            (collect, animate).chain().in_set(crate::GameSet::Hud),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_path(w: Work) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/alea")
            .join(w.file())
    }

    /// Every shipped work draws, at its in-game size, with real content
    /// (not a blank canvas), and the same seed gives the same pixels.
    #[test]
    fn every_work_draws_reproducibly() {
        for work in Work::ALL {
            let (w, h) = work.size();
            let frames = 2.min(work.frames());
            let a = render_frames(&work_path(work), 0xa1ea, w, h, frames)
                .unwrap_or_else(|e| panic!("{}: {e}", work.file()));
            let b = render_frames(&work_path(work), 0xa1ea, w, h, frames).unwrap();
            assert_eq!(a, b, "{} is not reproducible", work.file());
            assert_eq!(a.len(), frames);
            assert_eq!(a[0].len(), w * h * 4);
            let lit = a[0]
                .chunks(4)
                .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60)
                .count();
            let frac = lit as f32 / (w * h) as f32;
            assert!(frac > 0.02, "{} is nearly blank ({frac})", work.file());
            if work.frames() > 1 {
                assert_ne!(a[0], a[1], "{} doesn't move", work.file());
            }
        }
    }

    #[test]
    fn editions_differ_by_seed() {
        let p = work_path(Work::Sigil);
        let a = render_frames(&p, 1, 64, 64, 1).unwrap();
        let b = render_frames(&p, 2, 64, 64, 1).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn broken_work_is_an_error_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("ephemera-alea-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("broken.alea");
        std::fs::write(&p, "work Broken { render image(circle(").unwrap();
        assert!(render_frames(&p, 1, 16, 16, 1).is_err());
        assert!(render_frames(&dir.join("missing.alea"), 1, 16, 16, 1).is_err());
    }
}
