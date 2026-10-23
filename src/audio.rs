//! Sound. Every loop and chime in the game is rendered by alea at runtime:
//!
//! - each land region's ambience (its work's `render sound`), crossfaded by
//!   distance as you fly from land to land;
//! - each landmark's hum (the picture's own work, `render sound`), placed
//!   in space on the landmark;
//! - the Ephemera's wings (`sound/wings.alea`), pitched up with speed;
//! - music for the title, the camp and the race (`sound/*.alea`);
//! - the chimes of the rite (`sound/events.alea`, one render per event).
//!
//! Renders run on the compute pool and are cached on disk as WAV, keyed by
//! the work's text, its seed and the render, so an edited work re-renders
//! and everything else is instant. The game never waits for a sound: a clip
//! plays when it is ready.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::io::file::FileAssetReader;
use bevy::audio::{AudioSink, AudioSinkPlayback, SpatialScale, Volume};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::art::{Art, Landmark, Work};
use crate::components::CameraTarget;
use crate::state::GamePhase;
use crate::terrain::{REGION, Region, RegionId, hash};
use crate::world::{WorldGen, WorldTerrain};

const SAMPLE_RATE: u32 = 44_100;
/// alea fades every sound in and out over half a second: that is the loop
/// crossfade, and the silence trimmed from a one-shot.
const FADE: usize = SAMPLE_RATE as usize / 2;
/// Landmarks hum within this distance of the listener.
const HUM_RADIUS: f32 = 170.0;
/// One world unit of sound distance per this many metres.
const SPATIAL_SCALE: f32 = 1.0 / 22.0;

/// The events of the rite, in the order of `events.alea`'s renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sfx {
    Gate = 0,
    NetThrown = 1,
    Caught = 2,
    Spooked = 3,
    Bump = 4,
    Buck = 5,
    Boost = 6,
}

/// Ask for a chime. Any system can send it.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlaySound(pub Sfx);

/// What a clip is, and the cache key for it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SoundKey {
    /// `land/marsh`, `monolith`, `sound/wings`…
    pub work: String,
    pub seed: u64,
    pub render: usize,
}

impl SoundKey {
    pub fn region(id: &RegionId, render: usize) -> Self {
        Self {
            work: format!("land/{}", id.work),
            seed: id.seed,
            render,
        }
    }

    pub fn landmark(work: Work, seed: u64) -> Self {
        Self {
            work: work.file().trim_end_matches(".alea").to_string(),
            seed,
            render: 0,
        }
    }

    fn sound_work(name: &str, seed: u64, render: usize) -> Self {
        Self {
            work: format!("sound/{name}"),
            seed,
            render,
        }
    }
}

type Channels = Result<Vec<Vec<f64>>, String>;

struct Pending {
    key: SoundKey,
    task: Task<Result<Vec<u8>, String>>,
}

#[derive(Resource)]
pub struct Sounds {
    dir: PathBuf,
    cache: Option<PathBuf>,
    pending: Vec<Pending>,
    ready: HashMap<SoundKey, Handle<AudioSource>>,
    failed: HashSet<SoundKey>,
    /// Starts muted (`--mute`); `M` toggles.
    pub muted: bool,
}

impl Sounds {
    fn new() -> Self {
        Self {
            dir: FileAssetReader::get_base_path().join("assets").join("alea"),
            cache: cache_dir(),
            pending: Vec::new(),
            ready: HashMap::new(),
            failed: HashSet::new(),
            muted: std::env::args().any(|a| a == "--mute"),
        }
    }

    pub fn get(&self, key: &SoundKey) -> Option<Handle<AudioSource>> {
        self.ready.get(key).cloned()
    }

    fn known(&self, key: &SoundKey) -> bool {
        self.ready.contains_key(key)
            || self.failed.contains(key)
            || self.pending.iter().any(|p| &p.key == key)
    }

    /// Renders a clip in the background unless it is known. `render`
    /// produces alea's channels; the WAV is built and cached here.
    pub fn request(
        &mut self,
        key: SoundKey,
        looped: bool,
        src_text: Arc<str>,
        render: impl FnOnce() -> Channels + Send + 'static,
    ) {
        if self.known(&key) {
            return;
        }
        let cache = self.cache.clone();
        let (k, l) = (key.clone(), looped);
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let file = cache.as_ref().map(|c| {
                let id = hash(&[
                    fnv(src_text.as_bytes()) as i64,
                    k.seed as i64,
                    k.render as i64,
                    l as i64,
                ]);
                c.join(format!("{id:016x}.wav"))
            });
            if let Some(f) = &file {
                if let Ok(bytes) = std::fs::read(f) {
                    if bytes.len() > 44 {
                        return Ok(bytes);
                    }
                }
            }
            let channels = render()?;
            let bytes = wav(&channels, l);
            if let (Some(f), Some(dir)) = (&file, &cache) {
                let _ = std::fs::create_dir_all(dir);
                let _ = std::fs::write(f, &bytes);
            }
            Ok(bytes)
        });
        self.pending.push(Pending { key, task });
    }

    /// A render of one of the works in `assets/alea/sound/`.
    fn request_sound_work(&mut self, name: &str, seed: u64, render: usize, looped: bool) {
        let key = SoundKey::sound_work(name, seed, render);
        if self.known(&key) {
            return;
        }
        let path = self.dir.join("sound").join(format!("{name}.alea"));
        let src: Arc<str> = std::fs::read_to_string(&path).unwrap_or_default().into();
        self.request(key, looped, src, move || render_sound(&path, seed, render));
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn ready(&self) -> usize {
        self.ready.len()
    }

    pub fn failed(&self) -> usize {
        self.failed.len()
    }
}

/// Renders `render sound` number `render` (counting sounds only) of the
/// work at `path`, edition `seed`.
pub fn render_sound(path: &Path, seed: u64, render: usize) -> Channels {
    use alea::{eval, library, media};
    let file = path.to_string_lossy().into_owned();
    let module = library::load(&file).map_err(|e| e.display("", &file))?;
    let show = |e: alea::error::Error| e.display(&module.src, &module.file);
    let ed = eval::resolve(&module, seed).map_err(show)?;
    media::validate(&ed).map_err(show)?;
    let r = ed
        .renders
        .iter()
        .filter(|r| r.kind == "sound")
        .nth(render)
        .ok_or_else(|| format!("{file} has no `render sound` number {render}"))?;
    media::sound_channels(r, ed.seed).map_err(show)
}

/// A region's ambience, from its already resolved edition.
pub fn region_ambience(region: &Region) -> Channels {
    let i = region.ambience().ok_or("no ambience")?;
    alea::media::sound_channels(&region.edition.renders[i], region.id.seed)
        .map_err(|e| e.display("", &region.id.work))
}

/// 16-bit stereo WAV. A loop gets its fade-out added onto its fade-in and
/// cut, so it joins seamlessly; a one-shot loses its leading silence.
pub fn wav(channels: &[Vec<f64>], looped: bool) -> Vec<u8> {
    let (l, r) = match channels {
        [m] => (m.as_slice(), m.as_slice()),
        [l, r, ..] => (l.as_slice(), r.as_slice()),
        [] => (&[][..], &[][..]),
    };
    let n = l.len().min(r.len());
    let (mut left, mut right) = (l[..n].to_vec(), r[..n].to_vec());
    if looped && n > FADE * 3 {
        for ch in [&mut left, &mut right] {
            let keep = n - FADE;
            for i in 0..FADE {
                ch[i] += ch[keep + i];
            }
            ch.truncate(keep);
        }
    } else if !looped {
        let first = (0..n)
            .find(|&i| left[i].abs() > 0.004 || right[i].abs() > 0.004)
            .unwrap_or(0)
            .saturating_sub(SAMPLE_RATE as usize / 500);
        left.drain(..first);
        right.drain(..first);
    }
    let frames = left.len();
    let data_len = (frames * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for s in [left[i], right[i]] {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
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

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

// ------------------------------------------------------------ players --

/// The ambience of one region, looping; its volume follows the listener.
#[derive(Component)]
struct AmbienceLayer {
    region: Arc<Region>,
}

/// The landmark it is on hums.
#[derive(Component)]
struct Hum;

/// The Ephemera's wings, under the player.
#[derive(Component)]
struct WingDrone;

/// The current phase's music.
#[derive(Component)]
struct Music(&'static str);

fn listener_pos(
    focus: &Query<&Transform, With<CameraTarget>>,
    cams: &Query<&Transform, With<Camera3d>>,
) -> Option<Vec3> {
    focus
        .iter()
        .next()
        .or(cams.iter().next())
        .map(|t| t.translation)
}

/// Finished renders become audio sources.
fn collect(mut sounds: ResMut<Sounds>, mut sources: ResMut<Assets<AudioSource>>) {
    let mut i = 0;
    while i < sounds.pending.len() {
        let Some(result) = block_on(future::poll_once(&mut sounds.pending[i].task)) else {
            i += 1;
            continue;
        };
        let p = sounds.pending.remove(i);
        match result {
            Ok(bytes) => {
                let handle = sources.add(AudioSource {
                    bytes: bytes.into(),
                });
                sounds.ready.insert(p.key, handle);
            }
            Err(msg) => {
                warn!(
                    "alea: sound {} 0x{:x} #{} could not be rendered:\n{msg}",
                    p.key.work, p.key.seed, p.key.render
                );
                sounds.failed.insert(p.key);
            }
        }
    }
}

/// Requests the clips every phase needs as soon as the world exists.
fn request_basics(mut sounds: ResMut<Sounds>, wg: Option<Res<WorldGen>>) {
    let Some(wg) = wg else {
        return;
    };
    sounds.request_sound_work("wings", wg.seed, 0, true);
    for e in 0..7 {
        sounds.request_sound_work("events", 0xa1ea, e, false);
    }
}

/// Ambience: the regions around the listener each get a looping layer whose
/// volume is the region's share of influence at the listener.
fn ambience(
    mut commands: Commands,
    mut sounds: ResMut<Sounds>,
    terrain: Option<Res<WorldTerrain>>,
    focus: Query<&Transform, With<CameraTarget>>,
    cams: Query<&Transform, With<Camera3d>>,
    mut layers: Query<(Entity, &AmbienceLayer, Option<&mut AudioSink>)>,
) {
    let (Some(terrain), Some(p)) = (terrain, listener_pos(&focus, &cams)) else {
        return;
    };
    let land = &terrain.0.land;
    let here = Vec2::new(p.x, p.z);
    let cell = (here / REGION).floor().as_ivec2();

    // Influence of each nearby region, normalised.
    let mut near: Vec<(Arc<Region>, f32)> = Vec::new();
    for dz in -1..=1 {
        for dx in -1..=1 {
            if let Some(r) = land.region(cell + IVec2::new(dx, dz)) {
                let w = r.weight(here);
                if w > 0.01 {
                    near.push((r, w));
                }
            }
        }
    }
    let total: f32 = near.iter().map(|(_, w)| w).sum::<f32>().max(1e-6);

    // Make sure their ambiences are rendering, and spawn layers when ready.
    for (r, _) in &near {
        let Some(render) = r.ambience() else {
            continue;
        };
        let key = SoundKey::region(&r.id, render);
        if !sounds.known(&key) {
            let region = r.clone();
            let src: Arc<str> = land
                .works
                .iter()
                .find(|w| w.name == r.id.work)
                .map(|w| w.src.clone())
                .unwrap_or_else(|| "".into());
            sounds.request(key.clone(), true, src, move || region_ambience(&region));
        }
        let playing = layers.iter().any(|(_, l, _)| l.region.id == r.id);
        if !playing {
            if let Some(handle) = sounds.get(&key) {
                commands.spawn((
                    AmbienceLayer { region: r.clone() },
                    AudioPlayer(handle),
                    PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
                ));
            }
        }
    }

    // Follow the listener; drop layers that fell out of reach.
    for (e, layer, sink) in &mut layers {
        let w = layer.region.weight(here);
        if w < 0.005 {
            commands.entity(e).despawn();
            continue;
        }
        if let Some(mut sink) = sink {
            sink.set_volume(Volume::Linear((w / total * 0.9).min(1.0)));
        }
    }
}

/// Landmarks near the listener hum their edition, in place.
fn hums(
    mut commands: Commands,
    mut sounds: ResMut<Sounds>,
    art: Res<Art>,
    focus: Query<&Transform, With<CameraTarget>>,
    cams: Query<&Transform, With<Camera3d>>,
    landmarks: Query<(Entity, &GlobalTransform, &Landmark, Has<Hum>)>,
) {
    let Some(p) = listener_pos(&focus, &cams) else {
        return;
    };
    for (e, tf, lm, humming) in &landmarks {
        let d = tf.translation().distance(p);
        let key = SoundKey::landmark(lm.0.work, lm.0.seed);
        if humming {
            if d > HUM_RADIUS * 1.3 {
                // Dropping the sink is what stops the sound (spatial sources
                // get a SpatialAudioSink).
                commands.entity(e).remove::<(
                    Hum,
                    AudioPlayer,
                    PlaybackSettings,
                    AudioSink,
                    bevy::audio::SpatialAudioSink,
                )>();
            }
            continue;
        }
        if d > HUM_RADIUS {
            continue;
        }
        if !sounds.known(&key) {
            let path = art.work_path(lm.0.work);
            let src: Arc<str> = std::fs::read_to_string(&path).unwrap_or_default().into();
            let seed = lm.0.seed;
            sounds.request(key.clone(), true, src, move || render_sound(&path, seed, 0));
        }
        if let Some(handle) = sounds.get(&key) {
            commands.entity(e).insert((
                Hum,
                AudioPlayer(handle),
                PlaybackSettings::LOOP
                    .with_spatial(true)
                    .with_spatial_scale(SpatialScale::new(SPATIAL_SCALE))
                    .with_volume(Volume::Linear(0.9)),
            ));
        }
    }
}

/// The wings: a loop whose pitch and level follow the player's speed.
fn wings(
    mut commands: Commands,
    sounds: Res<Sounds>,
    wg: Option<Res<WorldGen>>,
    player: Query<&crate::racing::Rider, With<crate::racing::Player>>,
    mut drone: Query<(Entity, Option<&mut AudioSink>), With<WingDrone>>,
) {
    let Some(wg) = wg else {
        return;
    };
    let rider = player.single().ok();
    let existing = drone.single_mut().ok();
    match (rider, existing) {
        (None, Some((e, _))) => commands.entity(e).despawn(),
        (Some(_), None) => {
            if let Some(h) = sounds.get(&SoundKey::sound_work("wings", wg.seed, 0)) {
                commands.spawn((
                    WingDrone,
                    AudioPlayer(h),
                    PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
                ));
            }
        }
        (Some(r), Some((_, Some(mut sink)))) => {
            let s = r.speed_fraction();
            sink.set_speed(0.55 + 0.9 * s);
            sink.set_volume(Volume::Linear(0.25 + 0.5 * s));
        }
        _ => {}
    }
}

/// Which music a phase wants.
fn music_for(phase: GamePhase) -> Option<&'static str> {
    match phase {
        GamePhase::Title | GamePhase::Results => Some("title"),
        GamePhase::Taming => Some("camp"),
        GamePhase::Racing => Some("race"),
        _ => None,
    }
}

fn music(
    mut commands: Commands,
    mut sounds: ResMut<Sounds>,
    phase: Res<State<GamePhase>>,
    wg: Option<Res<WorldGen>>,
    playing: Query<(Entity, &Music)>,
) {
    let wanted = music_for(*phase.get());
    let current = playing.single().ok();
    match (wanted, current) {
        (None, Some((e, _))) => commands.entity(e).despawn(),
        (Some(name), Some((e, m))) if m.0 != name => commands.entity(e).despawn(),
        (Some(name), None) => {
            let Some(wg) = wg else {
                return;
            };
            sounds.request_sound_work(name, wg.seed, 0, true);
            if let Some(h) = sounds.get(&SoundKey::sound_work(name, wg.seed, 0)) {
                commands.spawn((
                    Music(name),
                    AudioPlayer(h),
                    PlaybackSettings::LOOP.with_volume(Volume::Linear(0.7)),
                ));
            }
        }
        _ => {}
    }
}

/// One-shot chimes. A chime whose clip isn't rendered yet is dropped.
fn chimes(mut commands: Commands, sounds: Res<Sounds>, mut events: MessageReader<PlaySound>) {
    for PlaySound(sfx) in events.read() {
        let key = SoundKey::sound_work("events", 0xa1ea, *sfx as usize);
        if let Some(h) = sounds.get(&key) {
            commands.spawn((
                AudioPlayer(h),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.8)),
            ));
        }
    }
}

fn mute_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut sounds: ResMut<Sounds>,
    global: Option<ResMut<GlobalVolume>>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        sounds.muted = !sounds.muted;
    }
    if let Some(mut g) = global {
        let wanted = if sounds.muted { 0.0 } else { 1.0 };
        if g.volume.to_linear() != wanted {
            g.volume = Volume::Linear(wanted);
        }
    }
}

/// Clears per-world players when the world changes.
fn clear_players(
    mut commands: Commands,
    q: Query<Entity, Or<(With<AmbienceLayer>, With<WingDrone>, With<Music>)>>,
) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Sounds::new())
            .add_message::<PlaySound>()
            .add_systems(
                Update,
                (
                    collect,
                    request_basics,
                    ambience,
                    hums,
                    wings,
                    music,
                    chimes,
                    mute_key,
                )
                    .chain()
                    .in_set(crate::GameSet::Hud),
            );
        for phase in [
            GamePhase::Title,
            GamePhase::Capture,
            GamePhase::Taming,
            GamePhase::Racing,
            GamePhase::Results,
            GamePhase::Explore,
        ] {
            app.add_systems(OnExit(phase), clear_players);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<f64> {
        // alea-like: fade in over FADE, hold, fade out over FADE.
        (0..n)
            .map(|i| {
                let fade_in = (i as f64 / FADE as f64).min(1.0);
                let fade_out = ((n - i) as f64 / FADE as f64).min(1.0);
                0.5 * fade_in * fade_out
            })
            .collect()
    }

    fn samples(bytes: &[u8]) -> Vec<i16> {
        bytes[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    #[test]
    fn loop_has_no_seam() {
        let n = SAMPLE_RATE as usize * 4;
        let w = wav(&[ramp(n), ramp(n)], true);
        let s = samples(&w);
        assert_eq!(
            s.len() / 2,
            n - FADE,
            "the fade-out is folded into the start"
        );
        // Start and end both sit at the held level: no dip at the join.
        let level = (0.5 * 32767.0) as i16;
        assert!((s[0] - level).abs() < 400, "start {} vs {level}", s[0]);
        assert!(
            (s[s.len() - 2] - level).abs() < 400,
            "end {}",
            s[s.len() - 2]
        );
        let min = s.iter().copied().min().unwrap();
        assert!(min > level - 2000, "dip to {min} inside the loop");
    }

    #[test]
    fn one_shot_is_trimmed() {
        let mut v = vec![0.0; SAMPLE_RATE as usize]; // 1 s of silence…
        v.extend(ramp(SAMPLE_RATE as usize)); // …then a sound
        let w = wav(&[v.clone(), v], false);
        let s = samples(&w);
        assert!(
            s.len() / 2 < SAMPLE_RATE as usize + 1000,
            "silence kept: {}",
            s.len() / 2
        );
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 2);
        assert_eq!(
            u32::from_le_bytes([w[24], w[25], w[26], w[27]]),
            SAMPLE_RATE
        );
    }

    fn sound_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/alea/sound")
    }

    #[test]
    fn every_sound_work_renders_audibly() {
        for (name, renders) in [
            ("wings", 1),
            ("title", 1),
            ("camp", 1),
            ("race", 1),
            ("events", 7),
        ] {
            for r in 0..renders {
                let ch = render_sound(&sound_dir().join(format!("{name}.alea")), 0xa1ea, r)
                    .unwrap_or_else(|e| panic!("{name} #{r}: {e}"));
                assert_eq!(ch.len(), 2, "{name} #{r} should be stereo");
                let peak = ch[0].iter().fold(0.0f64, |m, s| m.max(s.abs()));
                assert!(peak > 0.3, "{name} #{r} is too quiet (peak {peak})");
                assert!(peak <= 1.0, "{name} #{r} clips");
            }
        }
    }

    #[test]
    fn landmark_works_hum() {
        for work in Work::ALL {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/alea")
                .join(work.file());
            let ch =
                render_sound(&path, 0xa1ea, 0).unwrap_or_else(|e| panic!("{}: {e}", work.file()));
            let rms = (ch[0].iter().map(|s| s * s).sum::<f64>() / ch[0].len() as f64).sqrt();
            assert!(rms > 0.03, "{} hum is too quiet (rms {rms})", work.file());
        }
    }
}
