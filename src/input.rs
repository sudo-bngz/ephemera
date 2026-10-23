//! Unified player input, sampled once per frame before any simulation.
//!
//! Gameplay systems read `InputAxes` instead of raw key codes, so remapping
//! (or adding a gamepad later) touches exactly one system.

use bevy::prelude::*;

#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct InputAxes {
    /// -1 = left, +1 = right.
    pub steer: f32,
    /// -1 = brake / back, +1 = throttle / forward.
    pub throttle: f32,
    /// -1 = dive, +1 = climb.
    pub pitch: f32,
    /// Boost (Shift) / sprint while on foot.
    pub boost: bool,
    /// Crouch while on foot (Ctrl).
    pub crouch: bool,
    /// Primary action (Space) — throw net, confirm.
    pub action_pressed: bool,
    /// Confirm (Enter).
    pub confirm_pressed: bool,
    /// Numeric choices 1..=4, used by the taming menu.
    pub choice: Option<u8>,
}

fn sample(keys: Res<ButtonInput<KeyCode>>, mut axes: ResMut<InputAxes>) {
    let axis = |neg: &[KeyCode], pos: &[KeyCode]| -> f32 {
        let n = neg.iter().any(|k| keys.pressed(*k)) as i32 as f32;
        let p = pos.iter().any(|k| keys.pressed(*k)) as i32 as f32;
        p - n
    };
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
        boost: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
        crouch: keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight),
        action_pressed: keys.just_pressed(KeyCode::Space),
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
