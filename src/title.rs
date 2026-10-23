//! Title screen: a slow orbit around a wild Ephemera resting in the marsh.

use bevy::prelude::*;

use crate::camera::ChaseCamera;
use crate::components::{CameraTarget, PhaseScoped};
use crate::creature::{Coat, spawn_ephemera};
use crate::hud::Hud;
use crate::input::InputAxes;
use crate::state::GamePhase;
use crate::world::{Mood, WetlandAssets, WetlandsConfig, spawn_wetlands};

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<WetlandAssets>,
    mut hud: ResMut<Hud>,
) {
    let cam = spawn_wetlands(
        &mut commands,
        &mut meshes,
        &mut materials,
        &assets,
        WetlandsConfig {
            seed: 7,
            half_size: 90.0,
            reed_clumps: 110,
            pools: 18,
            spores: 90,
            mood: Mood::Dusk,
            clearing: 10.0,
        },
        &[],
    );
    commands.entity(cam).insert(ChaseCamera {
        orbit: Some(0.12),
        ..default()
    });

    spawn_ephemera(
        &mut commands,
        &mut meshes,
        &mut materials,
        Vec3::new(0.0, 2.2, 0.0),
        Coat::WILD,
        0.12,
        (PhaseScoped, CameraTarget::CAMP),
    );

    *hud = Hud {
        title: "E P H E M E R A".into(),
        body: "The marsh tribe does not name a child until she has ridden the wind.\n\
               Judith, raised by them but born of no clan, has one season to prove herself:\n\
               find a wild Ephemera, make it hers, and race it at the Gathering."
            .into(),
        banner: "PRESS ENTER".into(),
        footer: "ENTER — begin the rite      F3 — skip straight to the race      ESC — quit\n\
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
