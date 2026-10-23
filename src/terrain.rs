//! The shape of the endless world: one seeded height function, biomes, water.
//!
//! Everything here is a pure function of the seed and a position, so chunks
//! generated in any order, at any time, meet seamlessly, and the same seed
//! always gives back the same world. No Bevy types beyond `Vec2`/`Vec3`, so
//! it is cheap to unit-test.

use bevy::math::{Vec2, Vec3};

/// Water surface height. Terrain below it is a lake.
pub const SEA_LEVEL: f32 = 0.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    /// Reed beds and countless pools: the tribe's home ground.
    Marsh,
    /// Rolling hills of giant glowing fungi.
    Fungal,
    /// Pale dunes ribbed with the bones of something enormous.
    Bones,
    /// A high, dry plateau bristling with crystal shards.
    Crystal,
    /// A shallow basin where the monoliths float. Rare.
    Monoliths,
}

impl Biome {
    pub const ALL: [Biome; 5] = [
        Biome::Marsh,
        Biome::Fungal,
        Biome::Bones,
        Biome::Crystal,
        Biome::Monoliths,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Biome::Marsh => "the reed marsh",
            Biome::Fungal => "the fungal bloom",
            Biome::Bones => "the bone dunes",
            Biome::Crystal => "the crystal flats",
            Biome::Monoliths => "the field of monoliths",
        }
    }

    /// Anchor in (temperature, strangeness) space; the nearest anchors win.
    fn anchor(self) -> Vec2 {
        match self {
            Biome::Marsh => Vec2::new(-0.35, -0.25),
            Biome::Fungal => Vec2::new(0.35, -0.3),
            Biome::Bones => Vec2::new(0.4, 0.35),
            Biome::Crystal => Vec2::new(-0.4, 0.4),
            Biome::Monoliths => Vec2::new(0.0, 0.62),
        }
    }

    /// Ground colour (linear-ish sRGB components).
    fn ground(self) -> [f32; 3] {
        match self {
            Biome::Marsh => [0.17, 0.14, 0.15],
            Biome::Fungal => [0.20, 0.12, 0.22],
            Biome::Bones => [0.55, 0.47, 0.40],
            Biome::Crystal => [0.30, 0.34, 0.44],
            Biome::Monoliths => [0.07, 0.06, 0.10],
        }
    }

    fn highlight(self) -> [f32; 3] {
        match self {
            Biome::Marsh => [0.26, 0.36, 0.25],
            Biome::Fungal => [0.36, 0.20, 0.40],
            Biome::Bones => [0.80, 0.72, 0.60],
            Biome::Crystal => [0.55, 0.66, 0.80],
            Biome::Monoliths => [0.18, 0.12, 0.30],
        }
    }
}

/// A region forced to a given height (camp, nest): a disc that blends out.
#[derive(Clone, Copy, Debug)]
pub struct Flatten {
    pub centre: Vec2,
    pub radius: f32,
    pub height: f32,
}

/// The world's shape for one seed.
#[derive(Clone, Debug)]
pub struct Terrain {
    perm: [u8; 512],
    pub flatten: Vec<Flatten>,
}

impl Terrain {
    pub fn new(seed: u64) -> Self {
        // Seeded permutation (splitmix64 drives a Fisher–Yates shuffle).
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
            perm,
            flatten: Vec::new(),
        }
    }

    /// Classic 2D gradient noise, in −1..1.
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

    /// Fractal sum of `octaves` noise layers, roughly −1..1.
    pub fn fbm(&self, x: f32, y: f32, octaves: u32) -> f32 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for o in 0..octaves {
            // Offset each octave so they don't line up at the origin.
            let off = o as f32 * 37.13;
            sum += amp * self.noise(x * freq + off, y * freq - off);
            norm += amp;
            amp *= 0.5;
            freq *= 2.03;
        }
        sum / norm
    }

    /// How much each biome is present at a point (sums to 1).
    pub fn biome_weights(&self, x: f32, z: f32) -> [f32; 5] {
        let warp = self.fbm(x / 900.0 + 11.0, z / 900.0 - 7.0, 2) * 0.35;
        let c = Vec2::new(
            self.fbm(x / 1400.0 + 3.1 + warp, z / 1400.0, 3),
            self.fbm(x / 1400.0 - 5.7, z / 1400.0 + 9.2 + warp, 3),
        ) * 1.6;
        let mut w = [0.0; 5];
        let mut total = 0.0;
        for (i, b) in Biome::ALL.iter().enumerate() {
            let d = c.distance(b.anchor());
            // Sharp-ish falloff: mostly one biome, soft borders.
            let v = (-d * d * 22.0).exp();
            w[i] = v;
            total += v;
        }
        for v in &mut w {
            *v /= total.max(1e-6);
        }
        w
    }

    pub fn biome(&self, x: f32, z: f32) -> Biome {
        let w = self.biome_weights(x, z);
        let mut best = 0;
        for i in 1..5 {
            if w[i] > w[best] {
                best = i;
            }
        }
        Biome::ALL[best]
    }

    fn biome_height(&self, b: Biome, x: f32, z: f32) -> f32 {
        match b {
            // Barely above the water, pocked with pools.
            Biome::Marsh => 0.1 + 4.5 * self.fbm(x / 55.0, z / 55.0, 4),
            // Rounded hills.
            Biome::Fungal => 4.0 + 9.0 * self.fbm(x / 140.0 + 4.0, z / 140.0, 4),
            // Long dunes: stretched noise plus a ripple.
            Biome::Bones => {
                let d = self.fbm(x / 220.0, z / 70.0 + 2.0, 3);
                6.0 + 10.0 * d + 1.2 * ((x * 0.07 + d * 3.0).sin())
            }
            // A plateau with terraces.
            Biome::Crystal => {
                // Terraces 3 m high: steep risers, but continuous (a floor()
                // would tear the mesh).
                let h = 9.0 + 7.0 * self.fbm(x / 180.0 - 3.0, z / 180.0, 3);
                let t = h / 3.0;
                let step = t.floor() + smoothstep(0.35, 0.65, t - t.floor());
                step * 3.0
            }
            // A dark basin, just under the plain around it.
            Biome::Monoliths => 0.8 + 1.5 * self.fbm(x / 90.0, z / 90.0, 2),
        }
    }

    /// Height before flattening.
    fn raw_height(&self, x: f32, z: f32) -> f32 {
        let w = self.biome_weights(x, z);
        let mut h = 0.0;
        for (i, b) in Biome::ALL.iter().enumerate() {
            if w[i] > 0.01 {
                h += w[i] * self.biome_height(*b, x, z);
            }
        }
        // Lake basins, cutting across biomes (not the crystal plateau).
        let basin = smoothstep(0.08, 0.38, self.fbm(x / 520.0 + 17.0, z / 520.0 - 3.0, 3));
        h -= basin * 14.0 * (1.0 - w[3]);
        // A rare ridge system crossing everything, for scale.
        let ridge = 1.0 - self.fbm(x / 1100.0 - 21.0, z / 1100.0 + 4.0, 3).abs() * 4.0;
        h + ridge.max(0.0).powi(3) * 28.0
    }

    /// Terrain height at a point.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let mut h = self.raw_height(x, z);
        for f in &self.flatten {
            let d = Vec2::new(x, z).distance(f.centre);
            if d < f.radius * 1.8 {
                let k = 1.0 - smoothstep(f.radius, f.radius * 1.8, d);
                h = lerp(h, f.height, k);
            }
        }
        h
    }

    /// What a flier or walker rests on: the ground, or the water surface.
    pub fn surface(&self, x: f32, z: f32) -> f32 {
        self.height(x, z).max(SEA_LEVEL)
    }

    pub fn is_water(&self, x: f32, z: f32) -> bool {
        self.height(x, z) < SEA_LEVEL
    }

    pub fn normal(&self, x: f32, z: f32) -> Vec3 {
        let e = 0.8;
        let dx = self.height(x + e, z) - self.height(x - e, z);
        let dz = self.height(x, z + e) - self.height(x, z - e);
        Vec3::new(-dx, 2.0 * e, -dz).normalize()
    }

    /// Ground colour: biome blend, darker when wet, lighter on crests.
    pub fn color(&self, x: f32, z: f32, h: f32) -> [f32; 4] {
        let w = self.biome_weights(x, z);
        let mut base = [0.0f32; 3];
        let mut hi = [0.0f32; 3];
        for (i, b) in Biome::ALL.iter().enumerate() {
            let (g, l) = (b.ground(), b.highlight());
            for c in 0..3 {
                base[c] += w[i] * g[c];
                hi[c] += w[i] * l[c];
            }
        }
        let crest = smoothstep(-0.3, 0.6, self.fbm(x / 23.0, z / 23.0, 2));
        let wet = 1.0 - smoothstep(SEA_LEVEL - 0.5, SEA_LEVEL + 1.5, h);
        let mut out = [0.0; 4];
        for c in 0..3 {
            out[c] = lerp(base[c], hi[c], crest * 0.55) * (1.0 - 0.45 * wet);
        }
        out[3] = 1.0;
        out
    }
}

/// A deterministic 64-bit hash of a few integers, for per-chunk seeds.
pub fn hash(parts: &[i64]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        h = splitmix(h ^ (*p as u64));
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

    #[test]
    fn deterministic_per_seed() {
        let (a, b, c) = (Terrain::new(7), Terrain::new(7), Terrain::new(8));
        for i in 0..50 {
            let (x, z) = (i as f32 * 37.3 - 900.0, i as f32 * -21.7 + 400.0);
            assert_eq!(a.height(x, z), b.height(x, z));
        }
        let differs = (0..50).any(|i| {
            let (x, z) = (i as f32 * 37.3, i as f32 * 11.0);
            a.height(x, z) != c.height(x, z)
        });
        assert!(differs, "different seeds must give different worlds");
    }

    #[test]
    fn continuous_everywhere() {
        // No seams: neighbouring samples never jump. Checked on a long walk,
        // far from the origin too.
        let t = Terrain::new(42);
        let mut prev = t.height(-50_000.0, 1234.0);
        let mut worst: f32 = 0.0;
        for i in 1..20_000 {
            let x = -50_000.0 + i as f32 * 5.0;
            let h = t.height(x, 1234.0 + (i as f32 * 0.01).sin() * 300.0);
            worst = worst.max((h - prev).abs());
            prev = h;
        }
        assert!(worst < 4.0, "height jumped by {worst} m between 5 m steps");
    }

    #[test]
    fn has_every_biome_and_water() {
        let t = Terrain::new(2026);
        let mut seen = std::collections::HashSet::new();
        let mut water = 0;
        let n = 120;
        for i in 0..n {
            for j in 0..n {
                let (x, z) = (i as f32 * 150.0 - 9000.0, j as f32 * 150.0 - 9000.0);
                seen.insert(t.biome(x, z));
                water += t.is_water(x, z) as usize;
            }
        }
        assert_eq!(seen.len(), 5, "biomes seen: {seen:?}");
        let frac = water as f32 / (n * n) as f32;
        assert!((0.03..0.45).contains(&frac), "water fraction {frac}");
    }

    #[test]
    fn flatten_holds_its_height() {
        let mut t = Terrain::new(3);
        t.flatten.push(Flatten {
            centre: Vec2::new(100.0, -40.0),
            radius: 20.0,
            height: 2.5,
        });
        for (dx, dz) in [(0.0, 0.0), (10.0, 5.0), (-14.0, 0.0)] {
            assert!((t.height(100.0 + dx, -40.0 + dz) - 2.5).abs() < 1e-4);
        }
    }
}

#[cfg(test)]
mod survey {
    use super::*;
    /// `cargo test survey -- --ignored --nocapture`: biome / water statistics.
    #[test]
    #[ignore]
    fn print_survey() {
        let t = Terrain::new(2026);
        let mut counts = std::collections::HashMap::new();
        let (mut water, mut hmin, mut hmax) = (0, f32::MAX, f32::MIN);
        let n = 200;
        for i in 0..n {
            for j in 0..n {
                let (x, z) = (i as f32 * 100.0 - 10000.0, j as f32 * 100.0 - 10000.0);
                *counts.entry(t.biome(x, z)).or_insert(0) += 1;
                let h = t.height(x, z);
                hmin = hmin.min(h);
                hmax = hmax.max(h);
                water += (h < SEA_LEVEL) as usize;
            }
        }
        eprintln!(
            "biomes {counts:?}\nwater {:.1}%  height {hmin:.1}..{hmax:.1}",
            water as f32 * 100.0 / (n * n) as f32
        );
        let c = Vec2::new(t.fbm(3.1, 0.0, 3), t.fbm(-5.7, 9.2, 3));
        eprintln!("climate sample {c:?}");
    }
}
