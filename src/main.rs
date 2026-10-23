//! EPHEMERA — arcade racing on the back of a giant mosquito.
//!
//! Judith, an orphaned human raised by the marsh tribe, must earn her name by
//! capturing, taming and racing an Ephemera through the alien wetlands.
//!
//! The app is organised around Bevy's ECS: every phase is a `GamePhase` state,
//! every phase owns its entities (tagged `PhaseScoped`) and its systems, and
//! the only things that survive a phase transition are the shared resources in
//! `state.rs` (the Ephemera's stats, the run log) and the world/camera.

// Bevy system signatures are long by nature.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::collapsible_if
)]

mod camera;
mod capture;
mod components;
mod creature;
mod hud;
mod input;
mod models;
mod racing;
mod sky;
mod state;
mod taming;
mod title;
mod world;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowMode};

/// Ordering of the per-frame simulation. Input is sampled first, the
/// simulation integrates, the camera follows the *result* of the simulation,
/// and the HUD reads the final state — so what you see is always one frame
/// fresh, which matters a lot for the feeling of responsive controls.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameSet {
    Input,
    Simulation,
    Camera,
    Hud,
}

/// Everything that is the game, independent of windowing and rendering.
/// `main` adds it on top of `DefaultPlugins`; the headless smoke test adds it
/// on top of `MinimalPlugins`.
pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            Update,
            (
                GameSet::Input,
                GameSet::Simulation,
                GameSet::Camera,
                GameSet::Hud,
            )
                .chain(),
        )
        .add_plugins((
            state::StatePlugin,
            input::InputPlugin,
            models::ModelsPlugin,
            world::WorldPlugin,
            sky::SkyPlugin,
            creature::CreaturePlugin,
            camera::CameraPlugin,
            hud::HudPlugin,
            title::TitlePlugin,
            capture::CapturePlugin,
            taming::TamingPlugin,
            racing::RacingPlugin,
        ));
    }
}

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.03, 0.04, 0.07)))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "EPHEMERA".into(),
                        resolution: (1280, 720).into(),
                        present_mode: PresentMode::AutoVsync,
                        mode: WindowMode::Windowed,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_plugins(GamePlugin)
        .run();
}

#[cfg(test)]
mod smoke {
    //! Runs the whole game loop headless (no window, no GPU): every system
    //! gets initialised, which catches conflicting queries, and every phase
    //! gets ticked, which catches missing resources and arithmetic panics.

    use super::*;
    use crate::state::{GamePhase, RunLog};
    use bevy::input::ButtonState;
    use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            bevy::state::app::StatesPlugin,
            bevy::input::InputPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )))
        .add_plugins(GamePlugin);
        app
    }

    fn tick(app: &mut App, frames: usize) {
        for _ in 0..frames {
            app.update();
        }
    }

    // Go through the real keyboard message path so `just_pressed` behaves
    // exactly as it does with a window: `ButtonInput` is cleared and refilled
    // by `keyboard_input_system` in `PreUpdate` every frame.
    fn key(app: &mut App, key: KeyCode, state: ButtonState) {
        app.world_mut().write_message(KeyboardInput {
            key_code: key,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
    }

    fn press(app: &mut App, key_code: KeyCode) {
        key(app, key_code, ButtonState::Pressed);
    }

    fn release(app: &mut App, key_code: KeyCode) {
        key(app, key_code, ButtonState::Released);
    }

    fn phase(app: &App) -> GamePhase {
        *app.world().resource::<State<GamePhase>>().get()
    }

    /// `--race` enters the race on the very first frame, where Bevy reports a
    /// zero delta time. Nothing may turn NaN, or the camera renders nothing.
    #[test]
    fn race_survives_zero_delta_first_frame() {
        let mut app = app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Racing);
        tick(&mut app, 3);
        assert_eq!(phase(&app), GamePhase::Racing);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        press(&mut app, KeyCode::KeyW);
        tick(&mut app, 400);

        let (tf, proj) = app
            .world_mut()
            .query_filtered::<(&Transform, &Projection), With<crate::camera::ChaseCamera>>()
            .single(app.world())
            .expect("one camera");
        assert!(
            tf.translation.is_finite(),
            "camera position: {:?}",
            tf.translation
        );
        assert!(
            tf.rotation.is_finite(),
            "camera rotation: {:?}",
            tf.rotation
        );
        if let Projection::Perspective(p) = proj {
            assert!(p.fov.is_finite(), "camera fov: {}", p.fov);
        }
    }

    /// The screenshot bug: flying full speed into two overlapping tall
    /// clumps used to wedge the Ephemera in place. It must slide or hop out.
    #[test]
    fn never_stuck_between_tall_reeds() {
        use crate::world::ReedClump;
        let mut app = app();
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Racing);
        tick(&mut app, 3);
        press(&mut app, KeyCode::KeyW);
        // Countdown, then get up to speed.
        tick(&mut app, 300);

        let player = app
            .world_mut()
            .query_filtered::<(Entity, &Transform), With<crate::components::Ephemera>>()
            .iter(app.world())
            .find(|(e, _)| {
                app.world()
                    .get::<crate::racing::Racer>(*e)
                    .is_some_and(|r| r.name == "Judith")
            })
            .map(|(e, t)| (e, *t))
            .expect("player");
        let (entity, tf) = player;
        let fwd = tf.forward().as_vec3().with_y(0.0).normalize();
        let side = Vec3::new(-fwd.z, 0.0, fwd.x);
        let ahead = tf.translation.with_y(0.0) + fwd * 25.0;
        // Two tall clumps with a 1 m gap between them, right on the path:
        // narrower than the Ephemera, wide enough to aim for.
        for s in [-1.0, 1.0] {
            app.world_mut().spawn((
                crate::components::PhaseScoped,
                ReedClump {
                    radius: 3.5,
                    height: 12.0,
                    solid: true,
                    tilt: Vec2::ZERO,
                    tilt_vel: Vec2::ZERO,
                    phase: 0.0,
                },
                Transform::from_translation(ahead + side * s * 4.0),
                GlobalTransform::from_translation(ahead + side * s * 4.0),
            ));
        }
        // Like a player aiming for the gate behind the reeds: keep steering
        // at a point on the far side of the clumps.
        let goal = ahead + fwd * 60.0;
        let start = tf.translation;
        let mut held: Option<KeyCode> = None;
        for _ in 0..300 {
            let t = *app.world().get::<Transform>(entity).unwrap();
            let heading = t.forward().as_vec3().with_y(0.0).normalize_or(fwd);
            let to_goal = (goal - t.translation).with_y(0.0).normalize_or(fwd);
            // Positive cross.y = goal is to the left.
            let cross = heading.cross(to_goal).y;
            let want = if cross > 0.05 {
                Some(KeyCode::KeyA)
            } else if cross < -0.05 {
                Some(KeyCode::KeyD)
            } else {
                None
            };
            if want != held {
                if let Some(k) = held {
                    release(&mut app, k);
                }
                if let Some(k) = want {
                    press(&mut app, k);
                }
                held = want;
            }
            app.update();
        }
        let now = app.world().get::<Transform>(entity).unwrap().translation;
        // Reaching the goal (or flying past it) means we got through.
        let progress = (now - start).with_y(0.0).dot(fwd);
        assert!(
            progress > 70.0,
            "stuck at the clumps: only {progress:.1} m of progress in 5 s (now {now:?})"
        );
    }

    #[test]
    fn full_rite_of_passage() {
        let mut app = app();
        tick(&mut app, 3);
        assert_eq!(phase(&app), GamePhase::Title);

        // Title → capture.
        press(&mut app, KeyCode::Enter);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Enter);
        assert_eq!(phase(&app), GamePhase::Capture);

        // Sprint around in the open for a while, then crouch; nothing panics.
        press(&mut app, KeyCode::KeyW);
        press(&mut app, KeyCode::ShiftLeft);
        tick(&mut app, 240);
        release(&mut app, KeyCode::ShiftLeft);
        press(&mut app, KeyCode::ControlLeft);
        press(&mut app, KeyCode::KeyD);
        tick(&mut app, 120);
        // A far-away net throw is refused, not a crash.
        press(&mut app, KeyCode::Space);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Space);
        tick(&mut app, 60);
        assert_eq!(phase(&app), GamePhase::Capture);

        // Skip ahead: jump straight to taming.
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Taming);
        tick(&mut app, 2);
        assert_eq!(phase(&app), GamePhase::Taming);

        // Feed, forage, train, rest — one dawn each, with the playback timer.
        for key in [
            KeyCode::Digit2,
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
        ] {
            press(&mut app, key);
            tick(&mut app, 2);
            release(&mut app, key);
            tick(&mut app, 120);
        }
        assert_eq!(phase(&app), GamePhase::Taming);
        assert!(app.world().resource::<RunLog>().dawns_spent >= 4);

        // Ride to the Gathering.
        press(&mut app, KeyCode::Enter);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Enter);
        tick(&mut app, 150);
        assert_eq!(phase(&app), GamePhase::Racing);

        // Race: full throttle, boost, wiggle the controls for ~60 s of
        // simulated time. We don't assert a finish (that depends on steering)
        // — just that the whole flight model, gates and AI tick without
        // panicking.
        press(&mut app, KeyCode::KeyW);
        press(&mut app, KeyCode::ShiftLeft);
        for i in 0..3600 {
            if i % 300 == 0 {
                press(&mut app, KeyCode::KeyA);
            } else if i % 300 == 150 {
                release(&mut app, KeyCode::KeyA);
                press(&mut app, KeyCode::KeyE);
            } else if i % 300 == 200 {
                release(&mut app, KeyCode::KeyE);
            }
            app.update();
        }
        assert!(matches!(
            phase(&app),
            GamePhase::Racing | GamePhase::Results
        ));
        // The AI riders follow the spline on their own: within a minute at
        // least one of them must have closed a lap, which exercises the whole
        // steering → gates → placing chain.
        let mut riders: Vec<(&str, u32, u32, Vec<f32>)> = app
            .world_mut()
            .query::<&crate::racing::Racer>()
            .iter(app.world())
            .map(|r| (r.name, r.lap, r.place, r.laps.clone()))
            .collect();
        riders.sort_by_key(|r| r.2);
        for (name, lap, place, laps) in &riders {
            eprintln!("P{place} {name:<7} lap {lap}  {laps:?}");
        }
        let best_ai_lap = riders
            .iter()
            .filter(|r| r.0 != "Judith")
            .map(|r| r.1)
            .max()
            .unwrap_or(0);
        assert!(
            best_ai_lap >= 2,
            "no AI rider closed a lap (best: lap {best_ai_lap})"
        );

        // Results reachable and ESC returns to the title.
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Results);
        tick(&mut app, 3);
        assert_eq!(phase(&app), GamePhase::Results);
        press(&mut app, KeyCode::Escape);
        tick(&mut app, 2);
        assert_eq!(phase(&app), GamePhase::Title);

        // Leaving the race must not leave any race entity behind.
        let leftovers = app
            .world_mut()
            .query_filtered::<Entity, With<crate::racing::Racer>>()
            .iter(app.world())
            .count();
        assert_eq!(leftovers, 0);
    }
}
