//! A single, persistent HUD. Phases don't build UI; they write into the
//! `Hud` resource and this module renders it. Keeps all the layout in one
//! place and makes every screen look like the same game.

use bevy::prelude::*;

#[derive(Resource, Default, Debug, Clone)]
pub struct Hud {
    /// Big line, top-left.
    pub title: String,
    /// Multi-line body under the title.
    pub body: String,
    /// Centre of the screen — countdowns, "CAUGHT!", etc. Flashes.
    pub banner: String,
    /// Bottom strip — controls hint.
    pub footer: String,
    /// Right-hand column — stats, lap times.
    pub side: String,
}

#[derive(Component)]
struct HudTitle;
#[derive(Component)]
struct HudBody;
#[derive(Component)]
struct HudBanner;
#[derive(Component)]
struct HudFooter;
#[derive(Component)]
struct HudSide;

const INK: Color = Color::srgb(0.93, 0.92, 0.86);
const ACCENT: Color = Color::srgb(0.45, 0.98, 0.82);

fn setup(mut commands: Commands) {
    // Top-left block.
    commands
        .spawn((
            Name::new("HUD top-left"),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(24.0),
                top: Val::Px(20.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                max_width: Val::Px(520.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((
                HudTitle,
                Text::new(""),
                TextFont {
                    font_size: 30.0,
                    ..default()
                },
                TextColor(ACCENT),
            ));
            p.spawn((
                HudBody,
                Text::new(""),
                TextFont {
                    font_size: 17.0,
                    ..default()
                },
                TextColor(INK),
            ));
        });

    // Right column.
    commands.spawn((
        HudSide,
        Name::new("HUD side"),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(24.0),
            top: Val::Px(20.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 20.0,
            ..default()
        },
        TextColor(INK),
        TextLayout::new_with_justify(Justify::Right),
    ));

    // Banner, centred.
    commands.spawn((
        HudBanner,
        Name::new("HUD banner"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(0.0),
            right: Val::Percent(0.0),
            top: Val::Percent(36.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 54.0,
            ..default()
        },
        TextColor(ACCENT),
        TextLayout::new_with_justify(Justify::Center),
    ));

    // Footer.
    commands.spawn((
        HudFooter,
        Name::new("HUD footer"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(24.0),
            right: Val::Px(24.0),
            bottom: Val::Px(18.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgba(0.93, 0.92, 0.86, 0.75)),
    ));
}

fn render(
    hud: Res<Hud>,
    time: Res<Time>,
    mut sets: ParamSet<(
        Query<&mut Text, With<HudTitle>>,
        Query<&mut Text, With<HudBody>>,
        Query<(&mut Text, &mut TextColor), With<HudBanner>>,
        Query<&mut Text, With<HudFooter>>,
        Query<&mut Text, With<HudSide>>,
    )>,
) {
    if hud.is_changed() {
        if let Ok(mut t) = sets.p0().single_mut() {
            t.0 = hud.title.clone();
        }
        if let Ok(mut t) = sets.p1().single_mut() {
            t.0 = hud.body.clone();
        }
        if let Ok((mut t, _)) = sets.p2().single_mut() {
            t.0 = hud.banner.clone();
        }
        if let Ok(mut t) = sets.p3().single_mut() {
            t.0 = hud.footer.clone();
        }
        if let Ok(mut t) = sets.p4().single_mut() {
            t.0 = hud.side.clone();
        }
    }
    // Pulse the banner so it reads as an event, not a label.
    if let Ok((_, mut color)) = sets.p2().single_mut() {
        let a = 0.75 + 0.25 * (time.elapsed_secs() * 6.0).sin();
        *color = TextColor(ACCENT.with_alpha(a));
    }
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hud>()
            .add_systems(Startup, setup)
            .add_systems(Update, render.in_set(crate::GameSet::Hud));
    }
}
