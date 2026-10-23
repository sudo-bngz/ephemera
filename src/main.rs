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

mod anim;
mod art;
mod audio;
mod camera;
mod capture;
mod components;
mod creature;
mod explore;
mod fauna;
mod hud;
mod input;
mod menu;
mod models;
mod onfoot;
mod racing;
mod reality;
mod rig;
mod settlement;
mod sky;
mod state;
mod taming;
mod terrain;
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
        // The engine of the world…
        .add_plugins((
            state::StatePlugin,
            input::InputPlugin,
            models::ModelsPlugin,
            world::WorldPlugin,
            settlement::SettlementPlugin,
            fauna::FaunaPlugin,
            sky::SkyPlugin,
            art::ArtPlugin,
            audio::SoundPlugin,
            rig::RigPlugin,
            anim::AnimPlugin,
            creature::CreaturePlugin,
            camera::CameraPlugin,
            hud::HudPlugin,
            menu::MenuPlugin,
        ))
        // …and the phases of the rite.
        .add_plugins((
            title::TitlePlugin,
            capture::CapturePlugin,
            taming::TamingPlugin,
            racing::RacingPlugin,
            explore::ExplorePlugin,
            onfoot::OnFootPlugin,
            reality::RealityPlugin,
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
        .init_asset::<Image>()
        .init_asset::<AudioSource>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .init_asset::<bevy::gltf::Gltf>()
        .init_asset::<AnimationGraph>()
        .init_asset::<AnimationClip>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )))
        .add_plugins(GamePlugin)
        // Straight to the action: no story cards or briefings to click away.
        .insert_resource(crate::menu::UxSettings {
            briefings: false,
            intros: false,
        });
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
        let ahead = tf.translation + fwd * 25.0;
        let terrain = app
            .world()
            .resource::<crate::world::WorldTerrain>()
            .0
            .clone();
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
                    yaw: 0.0,
                },
                Transform::from_translation(on_ground(&terrain, ahead + side * s * 4.0)),
                GlobalTransform::from_translation(on_ground(&terrain, ahead + side * s * 4.0)),
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

    fn on_ground(t: &crate::terrain::Terrain, p: Vec3) -> Vec3 {
        p.with_y(t.surface(p.x, p.z))
    }

    fn explore(app: &mut App, seed: u64) {
        app.world_mut()
            .resource_mut::<crate::explore::ExploreSeed>()
            .0 = Some(seed);
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Explore);
        tick(app, 3);
        assert_eq!(phase(app), GamePhase::Explore);
    }

    /// Chunks build on the compute pool: tick with real time until none is
    /// pending (or the deadline passes).
    fn settle(app: &mut App, max_ms: u64) {
        let start = std::time::Instant::now();
        loop {
            app.update();
            let pending = app.world().resource::<crate::world::WorldGen>().pending();
            if pending == 0 || start.elapsed().as_millis() as u64 > max_ms {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn chunks_loaded(app: &App) -> Vec<IVec2> {
        app.world()
            .resource::<crate::world::WorldGen>()
            .loaded()
            .copied()
            .collect()
    }

    /// The world streams around the player: fly 3 km away and the chunks
    /// follow, the old ones are freed, and nothing piles up.
    #[test]
    fn world_streams_endlessly() {
        let mut app = app();
        explore(&mut app, 42);
        settle(&mut app, 30_000);
        let near_origin = chunks_loaded(&app);
        assert!(
            near_origin.len() > 30,
            "only {} chunks around the start",
            near_origin.len()
        );
        assert!(near_origin.contains(&IVec2::ZERO));

        // Teleport the player 3 km east.
        let player = app
            .world_mut()
            .query_filtered::<Entity, With<crate::racing::Player>>()
            .single(app.world())
            .unwrap();
        let far = Vec3::new(3000.0, 40.0, -150.0);
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = far;
        settle(&mut app, 30_000);

        let loaded = chunks_loaded(&app);
        let centre = (far.xz() / crate::world::CHUNK).floor().as_ivec2();
        assert!(loaded.contains(&centre), "no chunk under the player");
        assert!(
            loaded.iter().all(|c| (*c - centre).length_squared() <= 26),
            "chunks left behind near the origin"
        );
        let mut live: Vec<IVec2> = app
            .world_mut()
            .query::<&crate::world::Chunk>()
            .iter(app.world())
            .map(|c| c.0)
            .collect();
        let mut expected = loaded.clone();
        live.sort_by_key(|c| (c.x, c.y));
        expected.sort_by_key(|c| (c.x, c.y));
        assert_eq!(
            live, expected,
            "despawned chunks are still alive (or missing)"
        );
        // And the player is flying above the ground there, not under it.
        let y = app.world().get::<Transform>(player).unwrap().translation.y;
        let ground = app
            .world()
            .resource::<crate::world::WorldTerrain>()
            .0
            .surface(far.x, far.z);
        assert!(y >= ground, "player at {y} under the ground at {ground}");
    }

    /// Landmarks wear editions painted by alea: wait for the background
    /// renders and check they landed on the materials.
    #[test]
    fn landmarks_are_painted_by_alea() {
        use crate::art::{Art, Landmark};
        let mut app = app();
        explore(&mut app, 7);
        settle(&mut app, 30_000);
        let landmarks = app
            .world_mut()
            .query::<&Landmark>()
            .iter(app.world())
            .count();
        assert!(landmarks > 0, "no landmark in the first chunks");

        // Rendering happens on background threads: give it real time.
        for _ in 0..600 {
            if app.world().resource::<Art>().pending() == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
            app.update();
        }
        tick(&mut app, 2);
        let art = app.world().resource::<Art>();
        assert_eq!(art.pending(), 0, "alea still painting after 15 s");
        assert_eq!(art.failed(), 0, "some editions failed to paint");
        assert!(art.ready() > 0);

        let mats = app
            .world_mut()
            .query_filtered::<&MeshMaterial3d<StandardMaterial>, With<Landmark>>()
            .iter(app.world())
            .map(|m| m.0.clone())
            .collect::<Vec<_>>();
        let assets = app.world().resource::<Assets<StandardMaterial>>();
        let textured = mats
            .iter()
            .filter(|h| {
                assets
                    .get(*h)
                    .is_some_and(|m| m.base_color_texture.is_some())
            })
            .count();
        assert_eq!(textured, mats.len(), "landmarks without their alea texture");
    }

    /// The world sounds: wait for alea to render, then the player's wings
    /// hum, the lands around play their ambience, nearby landmarks hum in
    /// place, and a chime plays on request. (No audio device here: the
    /// players exist, Bevy's sinks don't.)
    #[test]
    fn the_world_sounds() {
        use crate::audio::{PlaySound, Sfx, Sounds};
        let mut app = app();
        explore(&mut app, 7);
        settle(&mut app, 30_000);
        // Rendering happens on background threads: give it real time.
        let start = std::time::Instant::now();
        loop {
            app.update();
            let s = app.world().resource::<Sounds>();
            if s.pending() == 0 && s.ready() > 0 || start.elapsed().as_secs() > 60 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        tick(&mut app, 3);
        let s = app.world().resource::<Sounds>();
        assert_eq!(s.failed(), 0, "some sounds failed to render");
        assert!(
            s.ready() >= 9,
            "only {} clips ready (wings + 7 events + ambience…)",
            s.ready()
        );

        let players = app
            .world_mut()
            .query::<(&AudioPlayer, &PlaybackSettings)>()
            .iter(app.world())
            .count();
        assert!(players >= 2, "no ambience/wings players ({players})");
        let spatial = app
            .world_mut()
            .query::<&PlaybackSettings>()
            .iter(app.world())
            .filter(|p| p.spatial)
            .count();
        let near_landmarks = app
            .world_mut()
            .query::<(&GlobalTransform, &crate::art::Landmark)>()
            .iter(app.world())
            .filter(|(t, _)| t.translation().length() < 170.0)
            .count();
        assert!(
            near_landmarks == 0 || spatial > 0,
            "{near_landmarks} landmark(s) in earshot but none hums"
        );

        // A chime: the message spawns a one-shot player.
        let before = app
            .world_mut()
            .query::<&AudioPlayer>()
            .iter(app.world())
            .count();
        app.world_mut().write_message(PlaySound(Sfx::Gate));
        tick(&mut app, 2);
        let after = app
            .world_mut()
            .query::<&AudioPlayer>()
            .iter(app.world())
            .count();
        assert_eq!(after, before + 1, "the gate chime didn't play");
    }

    /// Judith walks (her limbs swing) and crouches on foot; in the race
    /// every Ephemera carries a rider who leans into the bank.
    #[test]
    fn figures_move() {
        use crate::rig::{Gait, Rig, human};
        let mut app = app();
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Capture);
        tick(&mut app, 3);
        let judith = app
            .world_mut()
            .query_filtered::<Entity, With<crate::components::Judith>>()
            .single(app.world())
            .unwrap();
        let hip = app.world().get::<Rig>(judith).unwrap().joints[human::HIP[0]];
        let rest = app.world().get::<Transform>(hip).unwrap().rotation;
        press(&mut app, KeyCode::KeyW);
        let mut swing: f32 = 0.0;
        for _ in 0..90 {
            app.update();
            let r = app.world().get::<Transform>(hip).unwrap().rotation;
            swing = swing.max(rest.angle_between(r));
        }
        assert!(swing > 0.25, "Judith's legs didn't swing walking ({swing})");
        // Crouching drops her hips.
        let hips = app.world().get::<Rig>(judith).unwrap().joints[human::HIPS];
        let standing = app.world().get::<Transform>(hips).unwrap().translation.y;
        press(&mut app, KeyCode::KeyC);
        tick(&mut app, 60);
        let low = app.world().get::<Transform>(hips).unwrap().translation.y;
        assert!(
            low < standing - 0.15,
            "crouching didn't lower her ({standing} → {low})"
        );
        release(&mut app, KeyCode::KeyC);
        release(&mut app, KeyCode::KeyW);

        // The race: four Ephemera, four riders, all seated (ride pose).
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Racing);
        tick(&mut app, 60);
        let riders: Vec<f32> = app
            .world_mut()
            .query::<(&Gait, &ChildOf)>()
            .iter(app.world())
            .map(|(g, _)| g.pose.drop)
            .collect();
        assert_eq!(riders.len(), 4, "riders: {}", riders.len());
        assert!(
            riders.iter().all(|d| *d > 0.15),
            "riders not seated: {riders:?}"
        );
    }

    /// F lands the Ephemera and puts Judith on her feet; she walks, jumps,
    /// turns the camera, whistles the creature over and climbs back on.
    #[test]
    fn judith_walks_the_world() {
        use crate::onfoot::Walker;
        use crate::racing::Parked;
        let mut app = app();
        explore(&mut app, 11);
        settle(&mut app, 30_000);
        let eph = app
            .world_mut()
            .query_filtered::<Entity, With<crate::racing::Player>>()
            .single(app.world())
            .unwrap();
        let judith = app
            .world_mut()
            .query_filtered::<Entity, With<crate::components::Judith>>()
            .single(app.world())
            .unwrap();
        assert!(
            app.world().get::<ChildOf>(judith).is_some(),
            "starts mounted"
        );

        // Land and dismount.
        press(&mut app, KeyCode::KeyF);
        tick(&mut app, 2);
        release(&mut app, KeyCode::KeyF);
        for _ in 0..400 {
            app.update();
            if app.world().get::<Walker>(judith).is_some() {
                break;
            }
        }
        assert!(
            app.world().get::<Walker>(judith).is_some(),
            "never dismounted"
        );
        assert!(app.world().get::<ChildOf>(judith).is_none());
        assert!(
            app.world().get::<Parked>(eph).is_some(),
            "Ephemera not parked"
        );
        assert!(
            app.world()
                .get::<crate::components::CameraTarget>(judith)
                .is_some()
                && app
                    .world()
                    .get::<crate::components::CameraTarget>(eph)
                    .is_none(),
            "the camera should follow Judith"
        );
        let cam = app
            .world_mut()
            .query::<&crate::camera::ChaseCamera>()
            .single(app.world())
            .unwrap();
        assert_eq!(cam.mode, crate::camera::CameraMode::Orbit);

        // Walk forward (relative to the camera) for 3 s: she moves, stays on
        // the ground and faces the way she goes.
        let start = app.world().get::<Transform>(judith).unwrap().translation;
        press(&mut app, KeyCode::KeyW);
        tick(&mut app, 180);
        let tf = *app.world().get::<Transform>(judith).unwrap();
        let moved = (tf.translation - start).xz().length();
        assert!(moved > 8.0, "walked only {moved} m in 3 s");
        let terrain = app
            .world()
            .resource::<crate::world::WorldTerrain>()
            .0
            .clone();
        let ground = terrain.height(tf.translation.x, tf.translation.z);
        assert!(
            (tf.translation.y - ground.max(crate::terrain::SEA_LEVEL - 1.1)).abs() < 0.3
                || tf.translation.y < crate::terrain::SEA_LEVEL,
            "floating: y {} ground {ground}",
            tf.translation.y
        );
        // Running is faster.
        let before = tf.translation;
        press(&mut app, KeyCode::ShiftLeft);
        tick(&mut app, 120);
        let ran = (app.world().get::<Transform>(judith).unwrap().translation - before)
            .xz()
            .length();
        release(&mut app, KeyCode::ShiftLeft);
        assert!(ran > moved * 0.9, "sprint {ran} m vs walk {moved} m");
        release(&mut app, KeyCode::KeyW);

        // The arrows turn the camera.
        let yaw0 = app
            .world_mut()
            .query::<&crate::camera::ChaseCamera>()
            .single(app.world())
            .unwrap()
            .orbit_yaw;
        press(&mut app, KeyCode::ArrowLeft);
        tick(&mut app, 30);
        release(&mut app, KeyCode::ArrowLeft);
        let yaw1 = app
            .world_mut()
            .query::<&crate::camera::ChaseCamera>()
            .single(app.world())
            .unwrap()
            .orbit_yaw;
        assert!((yaw1 - yaw0).abs() > 0.3, "camera didn't turn");

        // A jump leaves the ground and comes back.
        let on_ground = app.world().get::<Transform>(judith).unwrap().translation.y;
        press(&mut app, KeyCode::Space);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Space);
        let mut peak = on_ground;
        for _ in 0..90 {
            app.update();
            peak = peak.max(app.world().get::<Transform>(judith).unwrap().translation.y);
        }
        assert!(
            peak > on_ground + 1.0,
            "jump peaked at +{}",
            peak - on_ground
        );
        assert!(app.world().get::<Walker>(judith).unwrap().grounded);

        // Far from the creature: F whistles it over; it arrives and parks.
        let d0 = {
            let e = app.world().get::<Transform>(eph).unwrap().translation;
            let j = app.world().get::<Transform>(judith).unwrap().translation;
            (e - j).xz().length()
        };
        if d0 < 6.5 {
            // Walk away first.
            press(&mut app, KeyCode::KeyS);
            tick(&mut app, 200);
            release(&mut app, KeyCode::KeyS);
        }
        press(&mut app, KeyCode::KeyF);
        tick(&mut app, 2);
        release(&mut app, KeyCode::KeyF);
        assert!(
            app.world().get::<Parked>(eph).unwrap().summon.is_some(),
            "no whistle"
        );
        for _ in 0..600 {
            app.update();
            if app.world().get::<Parked>(eph).unwrap().summon.is_none() {
                break;
            }
        }
        let d1 = {
            let e = app.world().get::<Transform>(eph).unwrap().translation;
            let j = app.world().get::<Transform>(judith).unwrap().translation;
            (e - j).xz().length()
        };
        assert!(d1 < 6.5, "the Ephemera stayed {d1} m away");

        // Back in the saddle.
        press(&mut app, KeyCode::KeyF);
        tick(&mut app, 2);
        release(&mut app, KeyCode::KeyF);
        assert!(
            app.world().get::<ChildOf>(judith).is_some(),
            "didn't remount"
        );
        assert!(app.world().get::<Walker>(judith).is_none());
        assert!(app.world().get::<Parked>(eph).is_none());
        assert!(
            app.world()
                .get::<crate::components::CameraTarget>(eph)
                .is_some()
        );
        // And she can fly away again.
        let p0 = app.world().get::<Transform>(eph).unwrap().translation;
        press(&mut app, KeyCode::KeyW);
        tick(&mut app, 120);
        let p1 = app.world().get::<Transform>(eph).unwrap().translation;
        assert!((p1 - p0).xz().length() > 20.0, "can't fly after remounting");
    }

    /// The world has people and animals in it: within a few kilometres of
    /// the start there are settlements with villagers who walk about, and
    /// beasts that move.
    #[test]
    fn the_world_is_inhabited() {
        use crate::fauna::{Beast, Critter};
        use crate::settlement::Villager;
        let mut app = app();
        explore(&mut app, 5);
        settle(&mut app, 30_000);
        let terrain = app
            .world()
            .resource::<crate::world::WorldTerrain>()
            .0
            .clone();
        // Fly to the nearest settlement.
        let site = terrain
            .sites_near(Vec2::ZERO, 3000.0)
            .into_iter()
            .min_by(|a, b| a.centre.length().total_cmp(&b.centre.length()))
            .expect("a settlement within 3 km");
        let player = app
            .world_mut()
            .query_filtered::<Entity, With<crate::racing::Player>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = Vec3::new(site.centre.x, site.height + 30.0, site.centre.y);
        settle(&mut app, 30_000);
        tick(&mut app, 5);
        let villagers: Vec<Vec3> = app
            .world_mut()
            .query::<(&GlobalTransform, &Villager)>()
            .iter(app.world())
            .map(|(t, _)| t.translation())
            .collect();
        let at_home = villagers
            .iter()
            .filter(|p| p.xz().distance(site.centre) < site.radius * 1.5)
            .count();
        assert!(
            at_home > 0,
            "a {:?} with nobody in it ({} elsewhere)",
            site.kind,
            villagers.len()
        );
        // Houses are solid.
        let houses = app
            .world_mut()
            .query::<(&GlobalTransform, &crate::world::Obstacle)>()
            .iter(app.world())
            .filter(|(t, _)| t.translation().xz().distance(site.centre) < site.radius * 1.2)
            .count();
        assert!(houses >= 3, "only {houses} solid things in the settlement");
        // People move about.
        tick(&mut app, 240);
        let later: Vec<Vec3> = app
            .world_mut()
            .query::<(&GlobalTransform, &Villager)>()
            .iter(app.world())
            .map(|(t, _)| t.translation())
            .collect();
        assert!(
            villagers
                .iter()
                .zip(&later)
                .any(|(a, b)| a.distance(*b) > 0.5),
            "nobody moved"
        );

        // Animals: something of every kind somewhere in the loaded world, or
        // at least two kinds, and they move.
        let beasts: Vec<(Beast, Vec3)> = app
            .world_mut()
            .query::<(&Critter, &GlobalTransform)>()
            .iter(app.world())
            .map(|(c, t)| (c.kind, t.translation()))
            .collect();
        let kinds: std::collections::HashSet<_> = beasts.iter().map(|b| b.0).collect();
        assert!(kinds.len() >= 2, "beasts around the settlement: {kinds:?}");
        tick(&mut app, 120);
        let after: Vec<Vec3> = app
            .world_mut()
            .query::<(&Critter, &GlobalTransform)>()
            .iter(app.world())
            .map(|(_, t)| t.translation())
            .collect();
        assert!(
            beasts
                .iter()
                .zip(&after)
                .any(|(a, b)| a.1.distance(*b) > 0.5),
            "no beast moved"
        );
        // Limbs swing: at least one leg's rotation differs from its rest.
        let swung = app
            .world_mut()
            .query::<(&crate::fauna::Limb, &Transform)>()
            .iter(app.world())
            .filter(|(l, t)| t.rotation.angle_between(l.rest.rotation) > 0.05)
            .count();
        assert!(swung > 0, "no limb moved");
    }

    /// The maze: an open veil is a portal. Crossing one shifts the world
    /// into the reality below (depth 1), morphing the ground; crossing an
    /// upward veil there brings the origin back.
    #[test]
    fn veils_lead_between_realities() {
        use crate::art::{EditionId, Work};
        use crate::explore::Readings;
        use crate::reality::{Gate, Portal, Realities, Shift};
        let mut app = app();
        explore(&mut app, 21);
        settle(&mut app, 30_000);
        tick(&mut app, 3);
        let origin = app.world().resource::<Realities>().current();
        assert_eq!(app.world().resource::<Realities>().depth(), 0);

        let find = |app: &mut App, up: bool| -> Option<(Entity, Vec3)> {
            app.world_mut()
                .query::<(Entity, &GlobalTransform, &Portal)>()
                .iter(app.world())
                .find(|(_, _, p)| matches!(p.gate, Gate::Up) == up)
                .map(|(e, t, _)| (e, t.translation()))
        };
        let (_, veil) = find(&mut app, false).expect("a veil leading down");
        assert!(
            find(&mut app, true).is_none(),
            "no veil leads up from the origin"
        );
        // Sealed until the reality is read.
        let open = |app: &mut App| {
            app.world_mut()
                .query::<&Portal>()
                .iter(app.world())
                .any(|p| p.open)
        };
        app.world_mut().resource_mut::<Readings>().0.clear();
        tick(&mut app, 2);
        assert!(!open(&mut app), "veils open before the reality is read");
        for k in 0..crate::reality::READINGS_TO_OPEN as u64 {
            app.world_mut()
                .resource_mut::<Readings>()
                .0
                .insert(EditionId {
                    work: Work::Orb,
                    seed: 1000 + k,
                });
        }
        tick(&mut app, 2);
        assert!(open(&mut app), "reading the reality didn't open the veils");

        // Fly into it.
        let player = app
            .world_mut()
            .query_filtered::<Entity, With<crate::racing::Player>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = veil;
        tick(&mut app, 2);
        assert!(
            app.world().contains_resource::<Shift>(),
            "crossing didn't start a shift"
        );
        assert!(app.world().resource::<crate::world::WorldGen>().paused);
        // The next reality builds on the compute pool; then the morph runs.
        let start = std::time::Instant::now();
        let mut blended = false;
        while app.world().contains_resource::<Shift>() {
            app.update();
            if app
                .world()
                .resource::<crate::world::WorldTerrain>()
                .0
                .blend
                .is_some()
            {
                blended = true;
            }
            if start.elapsed().as_secs() > 60 {
                panic!("the shift never finished");
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(blended, "the ground never morphed");
        let r = app.world().resource::<Realities>();
        assert_eq!(r.depth(), 1);
        let below = r.current();
        assert_ne!(below, origin);
        assert_eq!(
            app.world()
                .resource::<crate::world::WorldTerrain>()
                .0
                .land
                .seed,
            below
        );
        assert_eq!(
            app.world()
                .resource::<crate::world::WorldTerrain>()
                .0
                .land
                .depth,
            1
        );
        assert!(!app.world().resource::<crate::world::WorldGen>().paused);
        assert_eq!(
            app.world().resource::<Readings>().count(),
            0,
            "readings reset"
        );
        // The new reality streams in: props grow, chunks come back.
        settle(&mut app, 30_000);
        tick(&mut app, 3);
        assert!(chunks_loaded(&app).len() > 30);

        // Somewhere here a veil leads up; it is sealed until read, then
        // takes us home.
        if let Some((_, up)) = find(&mut app, true) {
            for k in 0..crate::reality::READINGS_TO_OPEN as u64 {
                app.world_mut()
                    .resource_mut::<Readings>()
                    .0
                    .insert(EditionId {
                        work: Work::Orb,
                        seed: 2000 + k,
                    });
            }
            tick(&mut app, 2);
            app.world_mut()
                .get_mut::<Transform>(player)
                .unwrap()
                .translation = up;
            tick(&mut app, 2);
            let start = std::time::Instant::now();
            while app.world().contains_resource::<Shift>() {
                app.update();
                assert!(start.elapsed().as_secs() < 60, "the way up never finished");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let r = app.world().resource::<Realities>();
            assert_eq!(r.depth(), 0);
            assert_eq!(r.current(), origin);
        }
    }

    /// What a player sees: the menu, the story, a briefing that holds the
    /// game until ENTER, then the game.
    #[test]
    fn menu_story_briefing_then_play() {
        use crate::menu::{Overlay, Ux, UxSettings};
        let mut app = app();
        app.insert_resource(UxSettings::default());
        tick(&mut app, 3);
        assert_eq!(app.world().resource::<Ux>().overlay, Overlay::Menu);
        // Gameplay keys do nothing under the menu; ENTER picks the rite.
        press(&mut app, KeyCode::Enter);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Enter);
        assert!(matches!(
            app.world().resource::<Ux>().overlay,
            Overlay::Intro { card: 0, .. }
        ));
        assert_eq!(phase(&app), GamePhase::Title);
        // Through the cards.
        for _ in 0..4 {
            press(&mut app, KeyCode::Enter);
            tick(&mut app, 1);
            release(&mut app, KeyCode::Enter);
            tick(&mut app, 1);
        }
        tick(&mut app, 2);
        assert_eq!(phase(&app), GamePhase::Capture);
        assert_eq!(
            app.world().resource::<Ux>().overlay,
            Overlay::Briefing(GamePhase::Capture)
        );
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        // Judith doesn't move while the briefing is up.
        let judith = app
            .world_mut()
            .query_filtered::<Entity, With<crate::components::Judith>>()
            .single(app.world())
            .unwrap();
        let at = app.world().get::<Transform>(judith).unwrap().translation;
        press(&mut app, KeyCode::KeyW);
        tick(&mut app, 30);
        assert_eq!(
            app.world().get::<Transform>(judith).unwrap().translation,
            at
        );
        press(&mut app, KeyCode::Enter);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Enter);
        assert_eq!(app.world().resource::<Ux>().overlay, Overlay::None);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        tick(&mut app, 40);
        assert_ne!(
            app.world().get::<Transform>(judith).unwrap().translation,
            at,
            "can't move after the briefing"
        );
        // TAB shows the controls, and hides them again.
        press(&mut app, KeyCode::Tab);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Tab);
        assert!(matches!(
            app.world().resource::<Ux>().overlay,
            Overlay::Controls { .. }
        ));
        press(&mut app, KeyCode::Tab);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Tab);
        assert_eq!(app.world().resource::<Ux>().overlay, Overlay::None);
    }

    /// N draws a new world in place.
    #[test]
    fn new_world_on_demand() {
        let mut app = app();
        explore(&mut app, 1);
        let first = app.world().resource::<crate::world::WorldGen>().seed;
        press(&mut app, KeyCode::KeyN);
        tick(&mut app, 3);
        release(&mut app, KeyCode::KeyN);
        assert_eq!(phase(&app), GamePhase::Explore);
        let second = app.world().resource::<crate::world::WorldGen>().seed;
        assert_ne!(first, second, "N didn't change the world");
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

        // Results reachable; ESC pauses, and the pause menu goes back to
        // the title.
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Results);
        tick(&mut app, 3);
        assert_eq!(phase(&app), GamePhase::Results);
        press(&mut app, KeyCode::Escape);
        tick(&mut app, 2);
        release(&mut app, KeyCode::Escape);
        assert_eq!(
            app.world().resource::<crate::menu::Ux>().overlay,
            crate::menu::Overlay::Pause
        );
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        for _ in 0..3 {
            press(&mut app, KeyCode::ArrowDown);
            tick(&mut app, 1);
            release(&mut app, KeyCode::ArrowDown);
            tick(&mut app, 1);
        }
        press(&mut app, KeyCode::Enter);
        tick(&mut app, 3);
        release(&mut app, KeyCode::Enter);
        assert_eq!(phase(&app), GamePhase::Title);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(
            app.world().resource::<crate::menu::Ux>().overlay,
            crate::menu::Overlay::Menu
        );

        // Leaving the race must not leave any race entity behind.
        let leftovers = app
            .world_mut()
            .query_filtered::<Entity, With<crate::racing::Racer>>()
            .iter(app.world())
            .count();
        assert_eq!(leftovers, 0);
    }
}
