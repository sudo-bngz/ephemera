//! Explore: free flight through an endless world.
//!
//! Every visit draws a new world (or replays one with `--seed`), the way an
//! alea work draws a new edition at every run. The HUD names the biome below
//! and the edition of the nearest landmark, and counts the editions found.

use std::collections::HashSet;

use bevy::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::art::{Art, EditionId, Landmark};
use crate::camera::ChaseCamera;
use crate::components::{Judith, Velocity};
use crate::hud::Hud;
use crate::onfoot::Walker;
use crate::racing::{Parked, Player, Race, Rider, spawn_player_rider, spawn_speed_lines};
use crate::state::{EphemeraStats, GamePhase};
use crate::terrain::{Flatten, RegionId};
use crate::world::{Mood, WorldSetup, WorldTerrain, spawn_world};

const FLIGHT_KEYS: &str = "W/S throttle · A/D bank · Q dive / E climb · SHIFT boost · F land & dismount · N new world · ESC title";
const FOOT_KEYS: &str = "WASD walk · mouse / arrows look · SHIFT run · SPACE jump · C crouch · F mount (close) / whistle · N new world";

/// A world to replay instead of a new one (`--seed`), consumed on entry.
#[derive(Resource, Default)]
pub struct ExploreSeed(pub Option<u64>);

/// The editions read in the current reality: what opens its veils.
#[derive(Resource, Default, Debug)]
pub struct Readings(pub HashSet<EditionId>);

impl Readings {
    pub fn count(&self) -> usize {
        self.0.len()
    }
}

#[derive(Resource)]
struct Explore {
    seed: u64,
    travelled: f32,
    last: Option<Vec3>,
    /// The land under the player: its edition.
    region: Option<RegionId>,
    region_banner: f32,
    lands_seen: HashSet<RegionId>,
}

fn new_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    crate::terrain::splitmix(nanos) & 0xffff_ffff
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut art: ResMut<Art>,
    mut wanted: ResMut<ExploreSeed>,
    stats: Res<EphemeraStats>,
    mut hud: ResMut<Hud>,
) {
    let seed = wanted.0.take().unwrap_or_else(new_seed);
    // The light is part of the edition too.
    let mood = match seed % 3 {
        0 => Mood::Noon,
        1 => Mood::Dusk,
        _ => Mood::Dawn,
    };
    let mut world = WorldSetup::new(seed, mood);
    world.flatten.push(Flatten {
        centre: Vec2::ZERO,
        radius: 12.0,
        height: 1.5,
    });
    let (cam, terrain) = spawn_world(&mut commands, &mut meshes, &mut materials, &mut art, world);
    commands.entity(cam).insert(ChaseCamera {
        base_fov: 64f32.to_radians(),
        fov_gain: 30f32.to_radians(),
        fov_speed_ref: 95.0,
        ..default()
    });

    // Fly with the mount you tamed, or a decent one if you came straight here.
    let mount = if stats.trust > EphemeraStats::default().trust {
        stats.clone()
    } else {
        EphemeraStats::quick_race()
    };
    let at = Vec3::new(0.0, terrain.surface(0.0, 0.0) + 6.0, 0.0);
    spawn_player_rider(&mut commands, &mut meshes, &mut materials, at, 0.0, &mount);
    let mut rng = StdRng::seed_from_u64(seed);
    spawn_speed_lines(&mut commands, &mut meshes, &mut materials, &mut rng);
    commands.insert_resource(Race::free_flight(seed));
    commands.insert_resource(crate::reality::Realities::origin(seed));
    commands.insert_resource(Readings::default());
    commands.insert_resource(Explore {
        seed,
        travelled: 0.0,
        last: None,
        region: None,
        region_banner: 0.0,
        lands_seen: HashSet::new(),
    });

    *hud = Hud {
        title: "THE ENDLESS MARSH".into(),
        body: format!(
            "World 0x{seed:x}. It goes on for ever; nobody has seen its edge.\n\
             Every land here and every light over it is an edition drawn by alea:\n\
             the relief, the ground, the monoliths, the orbs, and what you hear."
        ),
        banner: String::new(),
        footer: FLIGHT_KEYS.into(),
        side: String::new(),
    };
}

fn teardown(mut commands: Commands) {
    commands.remove_resource::<Explore>();
    commands.remove_resource::<Race>();
    commands.remove_resource::<crate::reality::Realities>();
    commands.remove_resource::<Readings>();
}

fn new_world(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<GamePhase>>) {
    if keys.just_pressed(KeyCode::KeyN) {
        // Leaving tears this world down; ExploreReload comes straight back.
        next.set(GamePhase::ExploreReload);
    }
}

fn come_back(mut next: ResMut<NextState<GamePhase>>) {
    next.set(GamePhase::Explore);
}

fn hud(
    time: Res<Time>,
    mut ex: ResMut<Explore>,
    terrain: Res<WorldTerrain>,
    art: Res<Art>,
    sounds: Res<crate::audio::Sounds>,
    wg: Res<crate::world::WorldGen>,
    player: Query<(&Transform, &Velocity, &Rider, Has<Parked>), With<Player>>,
    walker: Query<(&Transform, &Velocity, &Walker), With<Judith>>,
    landmarks: Query<(&GlobalTransform, &Landmark, Option<&crate::reality::Portal>)>,
    realities: Res<crate::reality::Realities>,
    mut readings: ResMut<Readings>,
    shift: Option<Res<crate::reality::Shift>>,
    mut hud: ResMut<Hud>,
) {
    let Ok((etf, evel, rider, parked)) = player.single() else {
        return;
    };
    let on_foot = walker.single().ok();
    let (tf, vel) = match on_foot {
        Some((t, v, _)) => (t, v),
        None => (etf, evel),
    };
    let p = tf.translation;
    if let Some(last) = ex.last {
        ex.travelled += (p - last).xz().length();
    }
    ex.last = Some(p);

    // Crossing into another land (another alea edition) gets a banner.
    let region = terrain.0.region_at(p.x, p.z).map(|r| r.id.clone());
    if ex.region != region {
        if ex.region.is_some() {
            ex.region_banner = 3.0;
        }
        if let Some(r) = &region {
            ex.lands_seen.insert(r.clone());
        }
        ex.region = region;
    }
    ex.region_banner = (ex.region_banner - time.delta_secs()).max(0.0);
    let land = ex.region.as_ref().map(|r| r.label()).unwrap_or_default();

    // The nearest landmark within sight: its edition, alea-style.
    let near = landmarks
        .iter()
        .map(|(t, l, portal)| (t.translation().distance(p), l.0, portal.map(|p| p.open)))
        .filter(|(d, _, _)| *d < 90.0)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((d, id, _)) = near {
        if d < 45.0 {
            readings.0.insert(id);
        }
    }

    let ground = terrain.0.surface(p.x, p.z);
    let mut painting = match (art.pending(), art.failed()) {
        (0, 0) => format!("\nalea: {} edition(s) painted", art.ready()),
        (0, f) => format!("\nalea: {f} edition(s) failed — see the log"),
        (p, _) => format!("\nalea: painting {p} edition(s)…"),
    };
    painting += &match (sounds.pending(), sounds.failed()) {
        (0, 0) => format!(", {} sound(s)", sounds.ready()),
        (0, f) => format!(", {f} sound(s) failed"),
        (p, _) => format!(", rendering {p} sound(s)…"),
    };
    if wg.pending() > 0 {
        painting += &format!("\nbuilding {} chunk(s)…", wg.pending());
    }
    let status = match on_foot {
        Some((_, _, w)) => {
            let doing = if w.swimming {
                "swimming"
            } else if !w.grounded {
                "in the air"
            } else {
                "on foot"
            };
            let d = (etf.translation - p).xz().length();
            format!(
                "{doing}\n{:>4.1} km/h\nEphemera {:.0} m {}",
                vel.0.length() * 3.6,
                d,
                if d < 6.5 {
                    "— F to mount"
                } else if parked {
                    "— F to whistle"
                } else {
                    ""
                }
            )
        }
        None => format!(
            "{:>4.0} km/h\nalt {:>3.0} m\nboost {}",
            vel.0.length() * 3.6,
            (p.y - ground).max(0.0),
            "█".repeat((rider.boost_fraction() * 10.0).round() as usize),
        ),
    };
    let veils = if readings.count() >= crate::reality::READINGS_TO_OPEN {
        "the veils are open".to_string()
    } else {
        format!(
            "read {}/{} editions to open the veils",
            readings.count(),
            crate::reality::READINGS_TO_OPEN
        )
    };
    hud.side = format!(
        "{status}\n\nREALITY 0x{:x} · depth {}\n{veils}\n\n{}\n{:.1} km travelled\n{} land(s), {} edition(s) read here{}",
        realities.current(),
        realities.depth(),
        land,
        ex.travelled / 1000.0,
        ex.lands_seen.len(),
        readings.count(),
        painting,
    );
    hud.footer = if on_foot.is_some() {
        FOOT_KEYS.into()
    } else {
        FLIGHT_KEYS.into()
    };
    hud.banner = if shift.is_some() {
        hud.banner.clone()
    } else if ex.region_banner > 0.0 {
        format!("— {land} —")
    } else if let Some((_, id, portal)) = near {
        match portal {
            Some(true) => format!("{} — a way out", id.label()),
            Some(false) => format!("{} — sealed", id.label()),
            None => id.label(),
        }
    } else {
        String::new()
    };
    if ex.travelled > 60.0 {
        hud.body = format!("World 0x{:x}", ex.seed);
    }
}

pub struct ExplorePlugin;

impl Plugin for ExplorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExploreSeed>()
            .add_systems(OnEnter(GamePhase::Explore), setup)
            .add_systems(OnExit(GamePhase::Explore), teardown)
            .add_systems(OnEnter(GamePhase::ExploreReload), come_back)
            .add_systems(
                Update,
                new_world
                    .in_set(crate::GameSet::Input)
                    .run_if(in_state(GamePhase::Explore)),
            )
            .add_systems(
                Update,
                hud.in_set(crate::GameSet::Hud)
                    .run_if(in_state(GamePhase::Explore)),
            );
    }
}
