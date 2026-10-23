//! The shape of the endless world, drawn by alea.
//!
//! The world is a sea of 512 m *regions*. Each region is an edition of one of
//! the land works in `assets/alea/land/` (its seed drawn from the world seed
//! and its coordinates), and a climate map decides which work, so similar
//! lands cluster into biomes. A land work is read as:
//!
//! - its first `render image`: the **relief** — the picture is the land.
//!   Black is the lake bottom, 30 % grey the shoreline, white the peaks;
//!   the render's `range` says how high the picture goes (`[0, 1]` peaks
//!   at +42 m, `[0, 5]` at +282 m: mountains);
//! - its second `render image` (optional): the **ground painting**;
//! - its `render sound`: the region's **ambience** (see `audio.rs`).
//!
//! Relief and painting are blended between neighbouring regions by
//! distance, so editions meet without seams. Over the lands run two things
//! of the world's own: **rivers**, carved along the zero lines of a noise
//! that knows nothing of region borders, and **settlement sites**, where
//! the tribe has flattened the ground for a town or driven stilts into the
//! shallows. Everything here is a function of the seed and a position:
//! chunks built in any order, on any thread, agree exactly, and the same
//! seed always gives back the same world.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use alea::compile::Program;
use alea::eval::{self, Edition};
use alea::media::{self, Palette};
use alea::{library, library::Module};
use bevy::asset::io::file::FileAssetReader;
use bevy::math::{IVec2, Vec2};
use bevy::prelude::{info, warn};

/// Water surface height. Terrain below it is a lake.
pub const SEA_LEVEL: f32 = 0.0;
/// Size of a region: one alea edition.
pub const REGION: f32 = 512.0;
/// A region's canvas (0–1) spans twice its size around its centre, so
/// neighbours overlap and blend inside their canvases.
const CANVAS: f32 = REGION * 2.0;
/// Relief value → metres: `(value − SHORE) × RELIEF_M`.
pub const SHORE: f32 = 0.3;
pub const RELIEF_M: f32 = 60.0;
/// How sharply a region's influence falls off with distance (in regions).
const FALLOFF: f32 = 4.0;
/// Rivers: the zero lines of a noise this many metres across…
const RIVER_SCALE: f32 = 1100.0;
/// …carved this deep at most (a gorge where the land is high)…
const RIVER_DEPTH: f32 = 16.0;
/// …down to this bed where the land is low (so they hold water).
const RIVER_BED: f32 = -2.6;
/// Settlement sites: one per cell at most, this far apart.
pub const SITE_CELL: f32 = 448.0;
/// No settlement this close to the origin (camps and the race live there).
const SITE_EXCLUSION: f32 = 300.0;

/// The props a region grows, by land work. Works the game doesn't know get
/// terrain, painting and ambience, but no props.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Props {
    Marsh,
    Fungal,
    Bones,
    Crystal,
    Monoliths,
    Peaks,
    Grove,
    None,
}

impl Props {
    fn for_work(name: &str) -> Props {
        match name {
            "marsh" => Props::Marsh,
            "fungal" => Props::Fungal,
            "bones" => Props::Bones,
            "crystal" => Props::Crystal,
            "monoliths" => Props::Monoliths,
            "peaks" => Props::Peaks,
            "grove" => Props::Grove,
            _ => Props::None,
        }
    }
}

/// What the tribe built at a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SiteKind {
    /// Huts on stilts over the shallows, joined by walkways.
    Stilts,
    /// A walled town of stone houses on flattened ground.
    Town,
    /// A hold of towers on a mountain shoulder.
    Hold,
}

/// A settlement: where it stands, how big, what kind. The ground under a
/// town or a hold is flattened to `height`; stilts stand in the water.
#[derive(Clone, Debug)]
pub struct Site {
    pub centre: Vec2,
    pub radius: f32,
    pub height: f32,
    pub kind: SiteKind,
    pub props: Props,
    pub seed: u64,
}

/// A region forced to a given height (camp, nest): a disc that blends out.
#[derive(Clone, Copy, Debug)]
pub struct Flatten {
    pub centre: Vec2,
    pub radius: f32,
    pub height: f32,
}

/// One of the land works, loaded once.
pub struct LandWork {
    pub name: String,
    pub props: Props,
    /// The work's text, the key of its cached editions.
    pub src: Arc<str>,
    module: Arc<Module>,
    /// Where it sits in climate space; the nearest work wins a region.
    anchor: Vec2,
}

/// How the land looks and sounds here: `name — edition 0x…`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RegionId {
    pub work: String,
    pub seed: u64,
}

impl RegionId {
    pub fn label(&self) -> String {
        let mut c = self.work.chars();
        let name = match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        };
        format!("{name} — edition 0x{:x}", self.seed)
    }
}

/// A resolved region: compiled relief and painting, and the edition itself
/// (for its ambience).
pub struct Region {
    pub centre: Vec2,
    pub id: RegionId,
    pub props: Props,
    pub edition: Edition,
    relief: Program,
    relief_range: (f64, f64),
    /// Relief value → literal relief: `t × span + lo` (so a `range: [0, 5]`
    /// climbs five times as high as `[0, 1]`; a measured range maps to 0–1).
    relief_lit: (f32, f32),
    paint: Program,
    palette: Palette,
    paint_range: (f64, f64),
}

impl Region {
    /// Canvas coordinates (0–1) of a world point, clamped to the canvas.
    fn canvas(&self, p: Vec2) -> (f64, f64) {
        let u = ((p - self.centre) / CANVAS + 0.5).clamp(Vec2::ZERO, Vec2::ONE);
        (u.x as f64, u.y as f64)
    }

    /// Influence at a world point, 0–1.
    pub fn weight(&self, p: Vec2) -> f32 {
        let d = p.distance(self.centre) / REGION;
        (-d * d * FALLOFF).exp()
    }

    /// Index of the ambience render, if the work has one.
    pub fn ambience(&self) -> Option<usize> {
        self.edition.renders.iter().position(|r| r.kind == "sound")
    }
}

/// Fixed values measured the way alea measures an image's range: sampled on
/// a grid over the canvas, ignoring the extreme 0.5 % at each end.
fn measure(prog: &Program, slots: &mut [f64]) -> (f64, f64) {
    let n = 48;
    let mut v: Vec<f64> = (0..n * n)
        .map(|i| {
            prog.run(
                slots,
                ((i % n) as f64 + 0.5) / n as f64,
                ((i / n) as f64 + 0.5) / n as f64,
            )
        })
        .filter(|v| v.is_finite())
        .collect();
    if v.is_empty() {
        return (0.0, 1.0);
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let (lo, hi) = (v[v.len() / 200], v[v.len() - 1 - v.len() / 200]);
    if hi - lo < 1e-9 {
        (lo, lo + 1.0)
    } else {
        (lo, hi)
    }
}

/// The world's own weather: seeded noise for the climate map and the rivers.
/// Small enough to copy into every sampler.
#[derive(Clone)]
pub struct Climate {
    perm: [u8; 512],
}

impl Climate {
    fn noise(&self, x: f32, y: f32) -> f32 {
        let (xi, yi) = (x.floor(), y.floor());
        let (xf, yf) = (x - xi, y - yi);
        let (xi, yi) = ((xi as i64 & 255) as usize, (yi as i64 & 255) as usize);
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let grad = |h: u8, x: f32, y: f32| match h & 7 {
            0 => x + y,
            1 => -x + y,
            2 => x - y,
            3 => -x - y,
            4 => x,
            5 => -x,
            6 => y,
            _ => -y,
        };
        let p = &self.perm;
        let aa = p[p[xi] as usize + yi];
        let ab = p[p[xi] as usize + yi + 1];
        let ba = p[p[xi + 1] as usize + yi];
        let bb = p[p[xi + 1] as usize + yi + 1];
        let (u, v) = (fade(xf), fade(yf));
        let x1 = lerp(grad(aa, xf, yf), grad(ba, xf - 1.0, yf), u);
        let x2 = lerp(grad(ab, xf, yf - 1.0), grad(bb, xf - 1.0, yf - 1.0), u);
        lerp(x1, x2, v) * std::f32::consts::FRAC_1_SQRT_2 * 1.4
    }

    fn fbm(&self, x: f32, y: f32, octaves: u32) -> f32 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for o in 0..octaves {
            let off = o as f32 * 37.13;
            sum += amp * self.noise(x * freq + off, y * freq - off);
            norm += amp;
            amp *= 0.5;
            freq *= 2.03;
        }
        sum / norm
    }

    /// The rivers at a point: how deep the cut is here (0–1, 1 on the
    /// river's bed) and the river's direction (the zero line's tangent).
    /// Rivers follow the zero lines of one slow noise over the whole world,
    /// so they cross region borders without noticing them.
    pub fn river(&self, p: Vec2) -> (f32, Vec2) {
        let n = |q: Vec2| self.fbm(q.x / RIVER_SCALE + 41.0, q.y / RIVER_SCALE - 23.0, 2);
        let v = n(p);
        // The river's half-width wanders between 10 and 24 m.
        let width = 17.0 + 7.0 * self.noise(p.x / 260.0 + 7.0, p.y / 260.0 + 3.0);
        // Quick reject: the noise changes by at most ~2 per RIVER_SCALE.
        if v.abs() * RIVER_SCALE / 2.2 > width * 2.0 {
            return (0.0, Vec2::X);
        }
        let e = 3.0;
        let gx = (n(p + Vec2::new(e, 0.0)) - n(p - Vec2::new(e, 0.0))) / (2.0 * e);
        let gz = (n(p + Vec2::new(0.0, e)) - n(p - Vec2::new(0.0, e))) / (2.0 * e);
        let grad = Vec2::new(gx, gz);
        let slope = grad.length().max(1e-5);
        // Metres to the zero line, and the line's direction.
        let d = v.abs() / slope;
        let along = Vec2::new(-gz, gx) / slope;
        let carve = 1.0 - smoothstep(0.0, width, d);
        (carve, along)
    }
}

/// All the land works and the regions resolved so far, shared by every
/// thread that samples the terrain.
pub struct Land {
    pub seed: u64,
    pub works: Vec<LandWork>,
    climate: Climate,
    regions: Mutex<HashMap<IVec2, Arc<Region>>>,
    sites: Mutex<HashMap<IVec2, Option<Arc<Site>>>>,
    /// Works that failed to draw, reported once.
    failed: Mutex<Vec<String>>,
}

impl Land {
    pub fn load(seed: u64) -> Self {
        Self::load_from(seed, &land_dir())
    }

    pub fn load_from(seed: u64, dir: &Path) -> Self {
        let mut works = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
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
                Ok(module) => {
                    let anchor = match name.as_str() {
                        "marsh" => Vec2::new(-0.35, -0.25),
                        "fungal" => Vec2::new(0.35, -0.3),
                        "bones" => Vec2::new(0.4, 0.35),
                        "crystal" => Vec2::new(-0.4, 0.4),
                        "monoliths" => Vec2::new(0.0, 0.62),
                        "peaks" => Vec2::new(0.05, -0.66),
                        "grove" => Vec2::new(-0.7, 0.02),
                        other => {
                            // Extra works get a spot of their own on the circle.
                            let a = (hash(&[other.len() as i64, fnv(other.as_bytes()) as i64])
                                % 360) as f32;
                            Vec2::new(a.to_radians().cos(), a.to_radians().sin()) * 0.5
                        }
                    };
                    works.push(LandWork {
                        props: Props::for_work(&name),
                        name,
                        src: module.src.clone(),
                        module,
                        anchor,
                    });
                }
                Err(e) => warn!("alea land work {}: {}", path.display(), e.display("", "")),
            }
        }
        if works.is_empty() {
            warn!(
                "no land works in {}: the world will be a flat plain",
                dir.display()
            );
        } else {
            info!(
                "land works: {}",
                works
                    .iter()
                    .map(|w| w.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        // Seeded permutation for the climate noise.
        let mut p: Vec<u8> = (0..=255).collect();
        let mut s = seed ^ 0x9E37_79B9_7F4A_7C15;
        for i in (1..256).rev() {
            s = splitmix(s);
            let j = (s % (i as u64 + 1)) as usize;
            p.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Self {
            seed,
            works,
            climate: Climate { perm },
            regions: Mutex::new(HashMap::new()),
            sites: Mutex::new(HashMap::new()),
            failed: Mutex::new(Vec::new()),
        }
    }

    /// Which land work a region gets: the nearest anchor in climate space.
    fn choose_work(&self, centre: Vec2) -> usize {
        let (x, z) = (centre.x, centre.y);
        let warp = self.climate.fbm(x / 1800.0 + 11.0, z / 1800.0 - 7.0, 2) * 0.35;
        let c = Vec2::new(
            self.climate.fbm(x / 2800.0 + 3.1 + warp, z / 2800.0, 3),
            self.climate
                .fbm(x / 2800.0 - 5.7, z / 2800.0 + 9.2 + warp, 3),
        ) * 1.6;
        let mut best = 0;
        let mut best_d = f32::MAX;
        for (i, w) in self.works.iter().enumerate() {
            let d = c.distance(w.anchor);
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        best
    }

    /// The region at grid coordinates, resolving it on first use.
    pub fn region(&self, coord: IVec2) -> Option<Arc<Region>> {
        if self.works.is_empty() {
            return None;
        }
        if let Some(r) = self.regions.lock().unwrap().get(&coord) {
            return Some(r.clone());
        }
        let r = Arc::new(self.resolve(coord));
        self.regions
            .lock()
            .unwrap()
            .entry(coord)
            .or_insert(r)
            .clone()
            .into()
    }

    fn resolve(&self, coord: IVec2) -> Region {
        let h = hash(&[self.seed as i64, coord.x as i64, coord.y as i64]);
        let jitter = Vec2::new(
            ((h & 0xffff) as f32 / 65535.0 - 0.5) * 0.4,
            (((h >> 16) & 0xffff) as f32 / 65535.0 - 0.5) * 0.4,
        );
        let centre = (coord.as_vec2() + 0.5 + jitter) * REGION;
        let wi = self.choose_work(centre);
        let work = &self.works[wi];
        let seed = (h >> 32) & 0xffff_ffff;
        let id = RegionId {
            work: work.name.clone(),
            seed,
        };

        match self.draw(work, seed) {
            Ok((edition, relief, relief_range, relief_lit, paint, palette, paint_range)) => {
                Region {
                    centre,
                    id,
                    props: work.props,
                    edition,
                    relief,
                    relief_range,
                    relief_lit,
                    paint,
                    palette,
                    paint_range,
                }
            }
            Err(msg) => {
                let mut failed = self.failed.lock().unwrap();
                if !failed.contains(&work.name) {
                    warn!("alea: land work `{}` could not be drawn:\n{msg}", work.name);
                    failed.push(work.name.clone());
                }
                self.flat(centre, id, work.props)
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn draw(
        &self,
        work: &LandWork,
        seed: u64,
    ) -> Result<
        (
            Edition,
            Program,
            (f64, f64),
            (f32, f32),
            Program,
            Palette,
            (f64, f64),
        ),
        String,
    > {
        let m = &work.module;
        let show = |e: alea::error::Error| e.display(&m.src, &m.file);
        let edition = eval::resolve(m, seed).map_err(show)?;
        media::validate(&edition).map_err(show)?;
        let images: Vec<&eval::Render> = edition
            .renders
            .iter()
            .filter(|r| r.kind == "image")
            .collect();
        let relief_r = images
            .first()
            .ok_or("a land work needs a `render image` for its relief")?;
        let paint_r = images.get(1).copied().unwrap_or(relief_r);
        let compile = |r: &eval::Render| -> Result<(Program, Palette, (f64, f64), bool), String> {
            let spec = media::image_spec(r).map_err(show)?;
            let field = r.required_field(0, "the field").map_err(show)?;
            let prog = Program::new(field, edition.seed);
            let (range, explicit) = match spec.range {
                Some(r) => (r, true),
                None => (measure(&prog, &mut prog.slots(spec.at)), false),
            };
            Ok((prog, spec.stops, range, explicit))
        };
        let (relief, _, relief_range, explicit) = compile(relief_r)?;
        // An explicit range is the picture's own scale of heights.
        let relief_lit = if explicit {
            (
                (relief_range.1 - relief_range.0) as f32,
                relief_range.0 as f32,
            )
        } else {
            (1.0, 0.0)
        };
        let (paint, palette, paint_range, _) = compile(paint_r)?;
        Ok((
            edition,
            relief,
            relief_range,
            relief_lit,
            paint,
            palette,
            paint_range,
        ))
    }

    /// A fallback region: a flat plain, for a work that failed to draw.
    fn flat(&self, centre: Vec2, id: RegionId, props: Props) -> Region {
        let field = eval::Field::Const(0.35);
        let edition = Edition {
            name: id.work.clone(),
            seed: id.seed,
            attempts: 1,
            renders: Vec::new(),
            inputs: Vec::new(),
        };
        Region {
            centre,
            id,
            props,
            edition,
            relief: Program::new(&field, 0),
            relief_range: (0.0, 1.0),
            relief_lit: (1.0, 0.0),
            paint: Program::new(&field, 0),
            palette: Palette {
                colors: vec![[40, 36, 44], [90, 80, 96]],
                positions: None,
                bands: 0,
            },
            paint_range: (0.0, 1.0),
        }
    }

    /// The settlement in a site cell, if the tribe built one there.
    pub fn site(&self, cell: IVec2) -> Option<Arc<Site>> {
        if self.works.is_empty() {
            return None;
        }
        if let Some(s) = self.sites.lock().unwrap().get(&cell) {
            return s.clone();
        }
        let s = self.resolve_site(cell).map(Arc::new);
        self.sites.lock().unwrap().entry(cell).or_insert(s).clone()
    }

    fn resolve_site(&self, cell: IVec2) -> Option<Site> {
        let h = hash(&[self.seed as i64, 7_777, cell.x as i64, cell.y as i64]);
        // Four settlements in ten cells.
        if (h & 0xff) >= 102 {
            return None;
        }
        let jitter = Vec2::new(
            ((h >> 8) & 0xffff) as f32 / 65535.0 - 0.5,
            ((h >> 24) & 0xffff) as f32 / 65535.0 - 0.5,
        ) * (SITE_CELL - 180.0);
        let centre = (cell.as_vec2() + 0.5) * SITE_CELL + jitter;
        if centre.length() < SITE_EXCLUSION {
            return None;
        }
        let seed = h >> 40;
        let props = self
            .region_at(centre)
            .map(|r| r.props)
            .unwrap_or(Props::None);
        // The wild ground here: a sampler of the regions around, no sites.
        let mut s = self.wild_sampler(centre, 1.0);
        let ground = s.wild_height(centre);
        // Rolling ground within the radius: average the height over a ring,
        // so the flattened disc sits at the land's mean, not on one bump.
        let mut mean = ground;
        for k in 0..6 {
            let a = k as f32 * std::f32::consts::TAU / 6.0;
            mean += s.wild_height(centre + Vec2::new(a.cos(), a.sin()) * 24.0);
        }
        let mean = mean / 7.0;
        let (kind, radius, height) = if mean < SEA_LEVEL + 1.5 && mean > SEA_LEVEL - 4.0 {
            (SiteKind::Stilts, 34.0, SEA_LEVEL)
        } else if mean < SEA_LEVEL - 4.0 {
            return None;
        } else if props == Props::Peaks || mean > 60.0 {
            (SiteKind::Hold, 30.0, mean.max(SEA_LEVEL + 3.0))
        } else {
            (SiteKind::Town, 40.0, mean.max(SEA_LEVEL + 2.0))
        };
        Some(Site {
            centre,
            radius,
            height,
            kind,
            props,
            seed,
        })
    }

    /// The settlements whose ground reaches within `radius` of a point.
    pub fn sites_near(&self, centre: Vec2, radius: f32) -> Vec<Arc<Site>> {
        let reach = radius + SITE_CELL;
        let lo = ((centre - reach) / SITE_CELL).floor().as_ivec2();
        let hi = ((centre + reach) / SITE_CELL).floor().as_ivec2();
        let mut out = Vec::new();
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                if let Some(s) = self.site(IVec2::new(x, z)) {
                    if s.centre.distance(centre) < radius + s.radius * 1.6 {
                        out.push(s);
                    }
                }
            }
        }
        out
    }

    /// A sampler of the regions around a point, without sites or flattens.
    fn wild_sampler(&self, centre: Vec2, radius: f32) -> Sampler {
        let lo = ((centre - radius) / REGION).floor().as_ivec2() - IVec2::ONE;
        let hi = ((centre + radius) / REGION).floor().as_ivec2() + IVec2::ONE;
        let mut regions = Vec::new();
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                if let Some(r) = self.region(IVec2::new(x, z)) {
                    let (rs, ps) = (r.relief.slots(0.0), r.paint.slots(0.0));
                    regions.push((r, rs, ps));
                }
            }
        }
        Sampler {
            climate: self.climate.clone(),
            regions,
            sites: Vec::new(),
            flatten: Vec::new(),
        }
    }

    /// The region whose influence is strongest at a point.
    pub fn region_at(&self, p: Vec2) -> Option<Arc<Region>> {
        let cell = (p / REGION).floor().as_ivec2();
        let mut best: Option<(f32, Arc<Region>)> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(r) = self.region(cell + IVec2::new(dx, dz)) {
                    let w = r.weight(p);
                    if best.as_ref().is_none_or(|(bw, _)| w > *bw) {
                        best = Some((w, r));
                    }
                }
            }
        }
        best.map(|(_, r)| r)
    }
}

/// A set of regions with scratch space, for sampling without locks: made
/// on the main thread, used on any.
pub struct Sampler {
    climate: Climate,
    regions: Vec<(Arc<Region>, Vec<f64>, Vec<f64>)>,
    sites: Vec<Arc<Site>>,
    flatten: Vec<Flatten>,
}

impl Sampler {
    fn raw_height(&mut self, p: Vec2) -> f32 {
        let (mut h, mut total) = (0.0, 0.0);
        for (r, relief_slots, _) in &mut self.regions {
            let w = r.weight(p);
            if w < 0.003 {
                continue;
            }
            let (u, v) = r.canvas(p);
            let val = r.relief.run(relief_slots, u, v);
            let (lo, hi) = r.relief_range;
            let t = (((val - lo) / (hi - lo)) as f32).clamp(0.0, 1.0);
            let (span, base) = r.relief_lit;
            h += w * (t * span + base - SHORE) * RELIEF_M;
            total += w;
        }
        if total <= 0.0 { 2.0 } else { h / total }
    }

    /// The land's own shape: the regions' reliefs with the rivers cut in,
    /// before anything built on it.
    pub fn wild_height(&mut self, p: Vec2) -> f32 {
        let h = self.raw_height(p);
        let (carve, _) = self.climate.river(p);
        if carve <= 0.0 {
            return h;
        }
        let bed = RIVER_BED.max(h - RIVER_DEPTH);
        lerp(h, bed, carve)
    }

    /// Terrain height at a point.
    pub fn height(&mut self, x: f32, z: f32) -> f32 {
        let p = Vec2::new(x, z);
        let mut h = self.wild_height(p);
        for s in &self.sites {
            if s.kind == SiteKind::Stilts {
                continue;
            }
            let d = p.distance(s.centre);
            if d < s.radius * 1.6 {
                let k = 1.0 - smoothstep(s.radius, s.radius * 1.6, d);
                h = lerp(h, s.height, k);
            }
        }
        for f in &self.flatten {
            let d = p.distance(f.centre);
            if d < f.radius * 1.8 {
                let k = 1.0 - smoothstep(f.radius, f.radius * 1.8, d);
                h = lerp(h, f.height, k);
            }
        }
        h
    }

    /// Ground colour at a point (sRGB bytes): the regions' paintings
    /// blended, darkened to wet mud along the rivers.
    pub fn color(&mut self, x: f32, z: f32) -> [u8; 3] {
        let p = Vec2::new(x, z);
        let (carve, _) = self.climate.river(p);
        let (mut c, mut total) = ([0.0f32; 3], 0.0f32);
        for (r, _, paint_slots) in &mut self.regions {
            let w = r.weight(p);
            if w < 0.003 {
                continue;
            }
            let (u, v) = r.canvas(p);
            let val = r.paint.run(paint_slots, u, v);
            let (lo, hi) = r.paint_range;
            let t = ((val - lo) / (hi - lo)).clamp(0.0, 1.0);
            let rgb = r.palette.color(t);
            for k in 0..3 {
                c[k] += w * rgb[k] as f32;
            }
            total += w;
        }
        if total <= 0.0 {
            [60, 56, 64]
        } else {
            let wet = 1.0 - 0.45 * carve;
            [
                (c[0] / total * wet).round() as u8,
                (c[1] / total * wet).round() as u8,
                (c[2] / total * wet * (1.0 + 0.1 * carve)).round() as u8,
            ]
        }
    }
}

/// The world's shape for one seed: the land plus the discs kept flat.
#[derive(Clone)]
pub struct Terrain {
    pub land: Arc<Land>,
    pub flatten: Vec<Flatten>,
}

impl Terrain {
    pub fn new(seed: u64) -> Self {
        Self {
            land: Arc::new(Land::load(seed)),
            flatten: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn from_dir(seed: u64, dir: &Path) -> Self {
        Self {
            land: Arc::new(Land::load_from(seed, dir)),
            flatten: Vec::new(),
        }
    }

    /// A sampler for everything within `radius` of `centre` (world xz).
    /// Resolves the regions it needs, so call it on the main thread.
    pub fn sampler(&self, centre: Vec2, radius: f32) -> Sampler {
        let mut s = self.land.wild_sampler(centre, radius);
        s.sites = self.land.sites_near(centre, radius);
        s.flatten = self.flatten.clone();
        s
    }

    /// The settlements whose ground reaches within `radius` of a point.
    pub fn sites_near(&self, centre: Vec2, radius: f32) -> Vec<Arc<Site>> {
        self.land.sites_near(centre, radius)
    }

    /// The river at a point: depth of the cut (0–1) and its direction.
    pub fn river(&self, x: f32, z: f32) -> (f32, Vec2) {
        self.land.climate.river(Vec2::new(x, z))
    }

    /// Terrain height at a point.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        self.sampler(Vec2::new(x, z), 0.0).height(x, z)
    }

    /// What a flier or walker rests on: the ground, or the water surface.
    pub fn surface(&self, x: f32, z: f32) -> f32 {
        self.height(x, z).max(SEA_LEVEL)
    }

    pub fn is_water(&self, x: f32, z: f32) -> bool {
        self.height(x, z) < SEA_LEVEL
    }

    /// The props the land grows at a point.
    pub fn props_at(&self, x: f32, z: f32) -> Props {
        self.land
            .region_at(Vec2::new(x, z))
            .map(|r| r.props)
            .unwrap_or(Props::None)
    }

    /// The land here, by name and edition.
    pub fn region_at(&self, x: f32, z: f32) -> Option<Arc<Region>> {
        self.land.region_at(Vec2::new(x, z))
    }
}

pub fn land_dir() -> PathBuf {
    FileAssetReader::get_base_path()
        .join("assets")
        .join("alea")
        .join("land")
}

/// A deterministic 64-bit hash of a few integers, for per-chunk seeds.
pub fn hash(parts: &[i64]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        h = splitmix(h ^ (*p as u64));
    }
    h
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

pub fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/alea/land")
    }

    #[test]
    fn deterministic_per_seed() {
        let (a, b, c) = (
            Terrain::from_dir(7, &dir()),
            Terrain::from_dir(7, &dir()),
            Terrain::from_dir(8, &dir()),
        );
        for i in 0..40 {
            let (x, z) = (i as f32 * 137.3 - 2000.0, i as f32 * -91.7 + 400.0);
            assert_eq!(a.height(x, z), b.height(x, z));
        }
        let differs = (0..40).any(|i| {
            let (x, z) = (i as f32 * 137.3, i as f32 * 41.0);
            a.height(x, z) != c.height(x, z)
        });
        assert!(differs, "different seeds must give different worlds");
    }

    #[test]
    fn continuous_across_regions() {
        // No seams between regions or chunks: on a 5 km walk crossing many
        // region borders, neighbouring 1 m samples never jump more than a
        // terrace step (the crystal flats step by ~6 m); mountainsides and
        // river banks are steep but continuous.
        let t = Terrain::from_dir(42, &dir());
        let mut s = t.sampler(Vec2::new(0.0, 0.0), 6000.0);
        let mut prev = s.height(-2500.0, 300.0);
        let mut worst: f32 = 0.0;
        for i in 1..5000 {
            let x = -2500.0 + i as f32;
            let h = s.height(x, 300.0 + (i as f32 * 0.0025).sin() * 400.0);
            worst = worst.max((h - prev).abs());
            prev = h;
        }
        assert!(worst < 7.0, "height jumped by {worst} m between 1 m steps");
    }

    #[test]
    fn mountains_rise_and_rivers_run() {
        let t = Terrain::from_dir(2026, &dir());
        // Somewhere in a 12 km square a peak stands well above the old
        // 42 m ceiling…
        let mut s = t.sampler(Vec2::ZERO, 7000.0);
        let mut highest = f32::MIN;
        for i in 0..48 {
            for j in 0..48 {
                let p = Vec2::new(i as f32 - 24.0, j as f32 - 24.0) * 250.0;
                highest = highest.max(s.height(p.x, p.y));
            }
        }
        assert!(highest > 100.0, "no mountain: highest point {highest} m");

        // …and rivers cut the land: following the zero line of the river
        // noise from a carved point, the cut stays and the bed holds water
        // where the land is low.
        let (mut wet, mut carved, mut n) = (0, 0, 0);
        for i in 0..40 {
            for j in 0..40 {
                let p = Vec2::new(i as f32 - 20.0, j as f32 - 20.0) * 90.0 + 2000.0;
                let (carve, _) = t.river(p.x, p.y);
                n += 1;
                if carve > 0.9 {
                    carved += 1;
                    let wild = s.wild_height(p);
                    let raw = s.raw_height(p);
                    assert!(
                        wild < raw - 1.0,
                        "river bed not cut at {p}: {wild} vs {raw}"
                    );
                    if raw < 12.0 {
                        wet += (wild < SEA_LEVEL) as usize;
                    }
                }
            }
        }
        let frac = carved as f32 / n as f32;
        assert!((0.004..0.25).contains(&frac), "river bed fraction {frac}");
        assert!(wet > 0, "no river holds water in the lowlands");
    }

    #[test]
    fn settlements_stand_on_flat_ground() {
        let t = Terrain::from_dir(2026, &dir());
        let sites = t.sites_near(Vec2::ZERO, 6000.0);
        assert!(
            sites.len() >= 8,
            "only {} settlements in 12 km",
            sites.len()
        );
        assert!(
            sites.iter().all(|s| s.centre.length() >= SITE_EXCLUSION),
            "a settlement on the camp"
        );
        let kinds: std::collections::HashSet<SiteKind> = sites.iter().map(|s| s.kind).collect();
        assert!(kinds.len() >= 2, "all settlements alike: {kinds:?}");
        for s in &sites {
            if s.kind == SiteKind::Stilts {
                continue;
            }
            for (dx, dz) in [(0.0, 0.0), (0.6, 0.0), (0.0, -0.7), (-0.5, 0.5)] {
                let p = s.centre + Vec2::new(dx, dz) * s.radius;
                let h = t.height(p.x, p.y);
                assert!(
                    (h - s.height).abs() < 0.05,
                    "{:?} site at {} not flat: {h} vs {}",
                    s.kind,
                    s.centre,
                    s.height
                );
            }
        }
        // The same world gives the same towns.
        let again = Terrain::from_dir(2026, &dir()).sites_near(Vec2::ZERO, 6000.0);
        assert_eq!(again.len(), sites.len());
        assert!(again.iter().zip(&sites).all(|(a, b)| a.centre == b.centre));
    }

    #[test]
    fn every_land_work_is_used_and_there_is_water() {
        let t = Terrain::from_dir(2026, &dir());
        assert!(t.land.works.len() >= 7, "shipped land works missing");
        let mut seen = std::collections::HashSet::new();
        let (mut water, mut n) = (0, 0);
        for i in 0..24 {
            for j in 0..24 {
                let p = Vec2::new(i as f32 - 12.0, j as f32 - 12.0) * REGION + REGION * 0.5;
                let r = t.land.region_at(p).unwrap();
                seen.insert(r.id.work.clone());
                water += t.is_water(p.x, p.y) as usize;
                n += 1;
            }
        }
        for w in &t.land.works {
            assert!(
                seen.contains(&w.name),
                "land work `{}` never appears",
                w.name
            );
        }
        let frac = water as f32 / n as f32;
        assert!((0.03..0.6).contains(&frac), "water fraction {frac}");
    }

    #[test]
    fn editions_differ_between_regions() {
        let t = Terrain::from_dir(3, &dir());
        let a = t.land.region(IVec2::new(0, 0)).unwrap();
        let b = t.land.region(IVec2::new(5, -3)).unwrap();
        assert_ne!(a.id.seed, b.id.seed);
        assert!(a.ambience().is_some(), "shipped works carry an ambience");
    }

    #[test]
    fn colours_come_from_the_painting() {
        let t = Terrain::from_dir(11, &dir());
        let mut s = t.sampler(Vec2::ZERO, 300.0);
        let cols: std::collections::HashSet<[u8; 3]> = (0..200)
            .map(|i| s.color(i as f32 * 3.1, (i * 7) as f32 % 250.0))
            .collect();
        assert!(
            cols.len() > 20,
            "ground painting is flat: {} colours",
            cols.len()
        );
    }

    #[test]
    fn flatten_holds_its_height() {
        let mut t = Terrain::from_dir(3, &dir());
        t.flatten.push(Flatten {
            centre: Vec2::new(100.0, -40.0),
            radius: 20.0,
            height: 2.5,
        });
        for (dx, dz) in [(0.0, 0.0), (10.0, 5.0), (-14.0, 0.0)] {
            assert!((t.height(100.0 + dx, -40.0 + dz) - 2.5).abs() < 1e-4);
        }
    }

    #[test]
    fn broken_or_missing_works_fall_back_to_a_plain() {
        let tmp = std::env::temp_dir().join(format!("ephemera-land-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("broken.alea"),
            "work Broken { render image(circle( }",
        )
        .unwrap();
        std::fs::write(
            tmp.join("nopic.alea"),
            "work NoPic { render sound(drone(clouds()), duration: 1) }",
        )
        .unwrap();
        let t = Terrain::from_dir(1, &tmp);
        // `broken` fails to load; `nopic` loads but can't be drawn as land.
        assert_eq!(t.land.works.len(), 1);
        let h = t.height(10.0, 10.0);
        assert!(h.is_finite());
        assert_eq!(t.props_at(10.0, 10.0), Props::None);
        let empty = Terrain::from_dir(1, &tmp.join("nothing"));
        assert!(empty.height(0.0, 0.0).is_finite());
    }
}
