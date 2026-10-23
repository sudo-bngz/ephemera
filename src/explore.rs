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
use crate::components::Velocity;
use crate::hud::Hud;
use crate::racing::{Player, Race, Rider, spawn_player_rider, spawn_speed_lines};
use crate::state::{EphemeraStats, GamePhase};
use crate::terrain::{Biome, Flatten};
use crate::world::{Mood, WorldSetup, WorldTerrain, spawn_world};

/// A world to replay instead of a new one (`--seed`), consumed on entry.
#[derive(Resource, Default)]
pub struct ExploreSeed(pub Option<u64>);

#[derive(Resource)]
struct Explore {
    seed: u64,
    seen: HashSet<EditionId>,
    travelled: f32,
    last: Option<Vec3>,
    biome: Option<Biome>,
    biome_banner: f32,
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
    commands.insert_resource(Explore {
        seed,
        seen: HashSet::new(),
        travelled: 0.0,
        last: None,
        biome: None,
        biome_banner: 0.0,
    });

    *hud = Hud {
        title: "THE ENDLESS MARSH".into(),
        body: format!(
            "World 0x{seed:x}. It goes on for ever; nobody has seen its edge.\n\
             Follow the lights: the monoliths, orbs, veils and sigils are painted by alea,\n\
             and every one of them is an edition of its own."
        ),
        banner: String::new(),
        footer:
            "W/S throttle · A/D bank · Q dive / E climb · SHIFT boost · N new world · ESC title"
                .into(),
        side: String::new(),
    };
}

fn teardown(mut commands: Commands) {
    commands.remove_resource::<Explore>();
    commands.remove_resource::<Race>();
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
    player: Query<(&Transform, &Velocity, &Rider), With<Player>>,
    landmarks: Query<(&GlobalTransform, &Landmark)>,
    mut hud: ResMut<Hud>,
) {
    let Ok((tf, vel, rider)) = player.single() else {
        return;
    };
    let p = tf.translation;
    if let Some(last) = ex.last {
        ex.travelled += (p - last).xz().length();
    }
    ex.last = Some(p);

    // Entering a new biome gets a banner for a few seconds.
    let biome = terrain.0.biome(p.x, p.z);
    if ex.biome != Some(biome) {
        if ex.biome.is_some() {
            ex.biome_banner = 3.0;
        }
        ex.biome = Some(biome);
    }
    ex.biome_banner = (ex.biome_banner - time.delta_secs()).max(0.0);

    // The nearest landmark within sight: its edition, alea-style.
    let near = landmarks
        .iter()
        .map(|(t, l)| (t.translation().distance(p), l.0))
        .filter(|(d, _)| *d < 90.0)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((d, id)) = near {
        if d < 45.0 {
            ex.seen.insert(id);
        }
    }

    let ground = terrain.0.surface(p.x, p.z);
    let painting = match (art.pending(), art.failed()) {
        (0, 0) => format!("\nalea: {} edition(s) painted", art.ready()),
        (0, f) => format!("\nalea: {f} edition(s) failed — see the log"),
        (p, _) => format!("\nalea: painting {p} edition(s)…"),
    };
    hud.side = format!(
        "{:>4.0} km/h\nalt {:>3.0} m\nboost {}\n\n{}\n{:.1} km travelled\n{} edition(s) found{}",
        vel.0.length() * 3.6,
        (p.y - ground).max(0.0),
        "█".repeat((rider.boost_fraction() * 10.0).round() as usize),
        biome.name(),
        ex.travelled / 1000.0,
        ex.seen.len(),
        painting,
    );
    hud.banner = if ex.biome_banner > 0.0 {
        format!("— {} —", biome.name())
    } else if let Some((_, id)) = near {
        id.label()
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
