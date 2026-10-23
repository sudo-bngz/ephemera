//! Unified player input, sampled once per frame before any simulation.
//!
//! Gameplay systems read `InputAxes` instead of raw key codes, so remapping
//! (or adding a gamepad later) touches exactly one system.

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;

#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct InputAxes {
    /// -1 = left, +1 = right (A/D and the arrows, in flight).
    pub steer: f32,
    /// -1 = brake / back, +1 = throttle / forward (W/S and the arrows).
    pub throttle: f32,
    /// -1 = dive, +1 = climb.
    pub pitch: f32,
    /// On foot: WASD only, x right, y forward — the arrows look instead.
    pub walk: Vec2,
    /// On foot: camera turn from the arrow keys (−1..1 per axis, held)…
    pub look_keys: Vec2,
    /// …and from the mouse (pixels this frame).
    pub look_mouse: Vec2,
    /// Boost (Shift) / sprint while on foot.
    pub boost: bool,
    /// Crouch while on foot, fold and drop in flight (C, or Ctrl — but
    /// Ctrl+arrows is a macOS shortcut).
    pub crouch: bool,
    /// SPACE held: beat straight up in flight.
    pub up: bool,
    /// V: switch between first and third person.
    pub view_pressed: bool,
    /// Primary action (Space) — throw net, jump, confirm.
    pub action_pressed: bool,
    /// Mount, dismount or whistle (F).
    pub interact_pressed: bool,
    /// Confirm (Enter).
    pub confirm_pressed: bool,
    /// Numeric choices 1..=4, used by the taming menu.
    pub choice: Option<u8>,
}

pub(crate) fn sample(
    keys: Res<ButtonInput<KeyCode>>,
    mut mouse: MessageReader<MouseMotion>,
    mut axes: ResMut<InputAxes>,
) {
    let axis = |neg: &[KeyCode], pos: &[KeyCode]| -> f32 {
        let n = neg.iter().any(|k| keys.pressed(*k)) as i32 as f32;
        let p = pos.iter().any(|k| keys.pressed(*k)) as i32 as f32;
        p - n
    };
    let look_mouse = mouse.read().fold(Vec2::ZERO, |acc, m| acc + m.delta);
    *axes = InputAxes {
        steer: axis(
            &[KeyCode::KeyA, KeyCode::ArrowLeft],
            &[KeyCode::KeyD, KeyCode::ArrowRight],
        ),
        throttle: axis(
            &[KeyCode::KeyS, KeyCode::ArrowDown],
            &[KeyCode::KeyW, KeyCode::ArrowUp],
        ),
        pitch: axis(&[KeyCode::KeyQ], &[KeyCode::KeyE]),
        walk: Vec2::new(
            axis(&[KeyCode::KeyA], &[KeyCode::KeyD]),
            axis(&[KeyCode::KeyS], &[KeyCode::KeyW]),
        ),
        look_keys: Vec2::new(
            axis(&[KeyCode::ArrowLeft], &[KeyCode::ArrowRight]),
            axis(&[KeyCode::ArrowDown], &[KeyCode::ArrowUp]),
        ),
        look_mouse,
        boost: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
        crouch: keys.pressed(KeyCode::KeyC)
            || keys.pressed(KeyCode::ControlLeft)
            || keys.pressed(KeyCode::ControlRight),
        action_pressed: keys.just_pressed(KeyCode::Space),
        up: keys.pressed(KeyCode::Space),
        view_pressed: keys.just_pressed(KeyCode::KeyV),
        interact_pressed: keys.just_pressed(KeyCode::KeyF),
        confirm_pressed: keys.just_pressed(KeyCode::Enter)
            || keys.just_pressed(KeyCode::NumpadEnter),
        choice: [
            (KeyCode::Digit1, 1),
            (KeyCode::Digit2, 2),
            (KeyCode::Digit3, 3),
            (KeyCode::Digit4, 4),
        ]
        .into_iter()
        .find(|(k, _)| keys.just_pressed(*k))
        .map(|(_, n)| n),
    };
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputAxes>()
            .add_systems(Update, sample.in_set(crate::GameSet::Input));
    }
}
