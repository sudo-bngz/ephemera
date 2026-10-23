//! Title screen: a slow orbit around a wild Ephemera resting by a monolith.

use bevy::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::art::Art;
use crate::camera::ChaseCamera;
use crate::components::{CameraTarget, PhaseScoped};
use crate::creature::{Coat, spawn_ephemera};
use crate::hud::Hud;
use crate::input::InputAxes;
use crate::state::GamePhase;
use crate::terrain::Flatten;
use crate::world::{Mood, WetlandAssets, WorldSetup, monolith, sigil, spawn_world};

/// The world every new rite starts in (Explore picks a new one each time).
pub const RITE_SEED: u64 = 7;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut art: ResMut<Art>,
    assets: Res<WetlandAssets>,
    mut hud: ResMut<Hud>,
) {
    let mut world = WorldSetup::new(RITE_SEED, Mood::Dusk);
    world.flatten.push(Flatten {
        centre: Vec2::ZERO,
        radius: 22.0,
        height: 1.5,
    });
    let (cam, terrain) = spawn_world(&mut commands, &mut meshes, &mut materials, &mut art, world);
    commands.entity(cam).insert(ChaseCamera {
        orbit: Some(0.12),
        ..default()
    });

    let ground = terrain.surface(0.0, 0.0);
    spawn_ephemera(
        &mut commands,
        &mut meshes,
        &mut materials,
        Vec3::new(0.0, ground + 2.2, 0.0),
        Coat::WILD,
        0.12,
        (PhaseScoped, CameraTarget::CAMP),
    );
    // A monolith over a sigil, to set the tone from the first frame.
    let mut rng = StdRng::seed_from_u64(RITE_SEED);
    for e in [
        monolith(
            &mut commands,
            &mut materials,
            &mut art,
            &assets,
            &mut rng,
            Vec3::new(-6.0, ground - 6.0, -26.0),
            3,
        ),
        sigil(
            &mut commands,
            &mut materials,
            &mut art,
            &assets,
            &mut rng,
            Vec3::new(0.0, ground, 0.0),
            5,
        ),
    ] {
        commands.entity(e).insert(PhaseScoped);
    }

    *hud = Hud {
        title: "E P H E M E R A".into(),
        body: "The marsh tribe does not name a child until she has ridden the wind.\n\
               Judith, raised by them but born of no clan, has one season to prove herself:\n\
               find a wild Ephemera, make it hers, and race it at the Gathering."
            .into(),
        banner: "PRESS ENTER".into(),
        footer: "ENTER — begin the rite      F3 — race      F4 — explore the endless marsh      ESC — quit\n\
                 On foot: WASD move · C crouch · SHIFT sprint · SPACE throw net\n\
                 In flight: W/S throttle · A/D bank · Q/E dive & climb · SHIFT boost"
            .into(),
        side: "".into(),
    };
}

fn update(axes: Res<InputAxes>, mut next: ResMut<NextState<GamePhase>>) {
    if axes.confirm_pressed || axes.action_pressed {
        next.set(GamePhase::Capture);
    }
}

pub struct TitlePlugin;

impl Plugin for TitlePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GamePhase::Title), setup)
            .add_systems(
                Update,
                update
                    .in_set(crate::GameSet::Simulation)
                    .run_if(in_state(GamePhase::Title)),
            );
    }
}
