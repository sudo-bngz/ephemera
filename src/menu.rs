//! Menus, story and how-to-play: everything that stands between the player
//! and the game, on purpose.
//!
//! One `Overlay` at a time covers the screen:
//!
//! - **Menu** — the title screen's main menu (begin the rite, into the veils,
//!   race, controls, quit), over the slowly orbiting title world;
//! - **Intro** — story cards before a journey starts, one after the other,
//!   skippable;
//! - **Briefing** — on entering a phase: what to do and the keys to do it,
//!   until ENTER;
//! - **Pause** (ESC in any phase) and **Controls** (TAB, or from a menu).
//!
//! While an overlay is up, gameplay sees no input and — outside the title —
//! virtual time stands still, so a race countdown waits for the briefing.
//! Keys and mouse both work: ↑/↓ or W/S to choose, ENTER or a click to pick.

use bevy::prelude::*;

use crate::input::InputAxes;
use crate::state::{EphemeraStats, GamePhase, RunLog};

const INK: Color = Color::srgb(0.90, 0.94, 0.97);
const DIM: Color = Color::srgba(0.70, 0.80, 0.88, 0.75);
const ACCENT: Color = Color::srgb(0.40, 0.95, 1.0);
const VIOLET: Color = Color::srgb(0.72, 0.52, 1.0);
const PANEL: Color = Color::srgba(0.02, 0.04, 0.08, 0.78);
const VEIL: Color = Color::srgba(0.01, 0.02, 0.05, 0.55);

/// Which story a journey opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Story {
    Rite,
    Veils,
}

impl Story {
    fn cards(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Story::Rite => &[
                (
                    "THE MARSH OF KHERA",
                    "Under a violet gas giant, a moon of shallow seas and drifting light.\n\
                     The settlers live in clans — an old way of living, kept by choice —\n\
                     in spires of grown ceramic, among machines older than their names.",
                ),
                (
                    "THE EPHEMERA",
                    "The wetlands' wild flyers: mosquito-shaped, house-sized, fast as weather.\n\
                     A clan that rides them crosses the moon in a day.\n\
                     A child of the clans is not named until she has ridden one.",
                ),
                (
                    "JUDITH",
                    "She was found as an infant at the foot of the veils —\n\
                     the auroras that hang over the marsh and hum at night.\n\
                     Raised by the clans, born of none. She has no name yet.",
                ),
                (
                    "THE RITE",
                    "This season she will stalk a wild Ephemera, bond with it,\n\
                     and race it against the clans' riders at the Gathering.\n\
                     Win, and they will give her a name.",
                ),
            ],
            Story::Veils => &[
                (
                    "EDITIONS",
                    "Every world is drawn from a seed, the way a work is drawn:\n\
                     the same rules, a different edition every time.",
                ),
                (
                    "THE VEILS",
                    "The auroras are seams between editions. Judith flew into one.\n\
                     The world on the other side was almost hers. The next one, less so.",
                ),
                (
                    "THE WAY BACK",
                    "Read each world — find its editions — and its veils open.\n\
                     One in three leads back towards the reality you came from.\n\
                     The others lead deeper. Find the origin.",
                ),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    Menu,
    Intro {
        story: Story,
        card: usize,
        then: GamePhase,
    },
    Briefing(GamePhase),
    Pause,
    /// The controls, then back to where they were opened from.
    Controls {
        from_pause: bool,
    },
}

impl Overlay {
    pub fn blocks(&self) -> bool {
        *self != Overlay::None
    }
}

/// Turn the intros and briefings off (the headless tests do).
#[derive(Resource, Debug, Clone, Copy)]
pub struct UxSettings {
    pub briefings: bool,
    pub intros: bool,
}

impl Default for UxSettings {
    fn default() -> Self {
        Self {
            briefings: true,
            intros: true,
        }
    }
}

#[derive(Resource, Default)]
pub struct Ux {
    pub overlay: Overlay,
    selected: usize,
    /// Seconds the current intro card has been up.
    card_t: f32,
    /// Phases already briefed this session.
    briefed: Vec<GamePhase>,
    rebuild: bool,
    /// A restart passing through the title.
    pending: Option<GamePhase>,
}

impl Ux {
    pub fn show(&mut self, o: Overlay) {
        self.overlay = o;
        self.selected = 0;
        self.card_t = 0.0;
        self.rebuild = true;
    }
}

#[derive(Component)]
struct OverlayRoot;

#[derive(Component)]
struct MenuItem(usize);

#[derive(Component)]
struct FadeIn;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Rite,
    Veils,
    Race,
    Controls,
    Quit,
    Resume,
    Restart,
    Title,
}

impl Choice {
    fn label(self) -> &'static str {
        match self {
            Choice::Rite => "BEGIN THE RITE",
            Choice::Veils => "INTO THE VEILS",
            Choice::Race => "RACE",
            Choice::Controls => "CONTROLS",
            Choice::Quit => "QUIT",
            Choice::Resume => "RESUME",
            Choice::Restart => "RESTART",
            Choice::Title => "BACK TO THE MENU",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Choice::Rite => "The story: stalk, bond and race for a name",
            Choice::Veils => "An endless maze of worlds drawn by alea — find the way home",
            Choice::Race => "Straight to the Gathering, on a well-bonded mount",
            Choice::Controls => "Every key, phase by phase",
            Choice::Quit => "",
            Choice::Resume => "",
            Choice::Restart => "Start this phase again",
            Choice::Title => "",
        }
    }
}

const MENU: [Choice; 5] = [
    Choice::Rite,
    Choice::Veils,
    Choice::Race,
    Choice::Controls,
    Choice::Quit,
];
const PAUSE: [Choice; 5] = [
    Choice::Resume,
    Choice::Controls,
    Choice::Restart,
    Choice::Title,
    Choice::Quit,
];

fn choices(o: Overlay) -> &'static [Choice] {
    match o {
        Overlay::Menu => &MENU,
        Overlay::Pause => &PAUSE,
        _ => &[],
    }
}

/// What to do and how, for a phase.
fn briefing(
    phase: GamePhase,
) -> (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
) {
    match phase {
        GamePhase::Capture => (
            "I · THE STALK",
            "A wild Ephemera rests on a pad in the dusk marsh. Get close enough to throw\n\
             the net while it is calm. Noise and being seen raise its awareness; crouching\n\
             still in the reeds lets it fall. Pools splash. If it bolts, follow it.",
            &[
                ("W / S", "walk forward / back"),
                ("A / D", "turn"),
                ("C", "crouch — quiet, hidden in reeds"),
                ("SHIFT", "sprint — loud"),
                ("SPACE", "throw the net (close range)"),
            ],
        ),
        GamePhase::Taming => (
            "II · THE TETHER",
            "Seven dawns at camp to make it yours. Each dawn, choose one thing to do.\n\
             Keep it fed and keep its trust: starve it or lose its trust and it snaps\n\
             the tether. What you build here is what you race with.",
            &[
                ("1", "forage nectar"),
                ("2", "feed and groom — trust"),
                ("3", "flight training — speed, agility"),
                ("4", "rest — Judith's energy"),
                ("ENTER", "ride to the Gathering"),
            ],
        ),
        GamePhase::Racing => (
            "III · THE GATHERING",
            "Three laps through the gates, in order, against three clan riders.\n\
             Skim low over water for speed. Tall reeds and stone stop you dead.\n\
             A mount that doesn't trust you will buck.",
            &[
                ("W / S", "throttle / brake"),
                ("A / D", "bank left / right"),
                ("Q / E", "dive / climb"),
                ("SHIFT", "boost"),
            ],
        ),
        GamePhase::Explore => (
            "THE MAZE OF REALITIES",
            "Fly. Land. Walk. Every world here is an edition of the same work.\n\
             Fly close to three of its landmarks to read it: then its veils open.\n\
             Fly through a veil to shift into another reality. Find the way home.",
            &[
                ("W / S · A / D", "throttle · bank"),
                ("Q / E · SHIFT", "dive / climb · boost"),
                (
                    "F",
                    "land and get off · beside it: mount · far: whistle it over",
                ),
                ("WASD · MOUSE", "on foot: walk · look"),
                ("SHIFT · SPACE · C", "on foot: run · jump · crouch"),
                ("N", "a new world"),
            ],
        ),
        _ => ("", "", &[]),
    }
}

const GLOBAL_KEYS: [(&str, &str); 5] = [
    ("ESC", "pause"),
    ("TAB", "controls"),
    ("M", "mute"),
    ("F1 – F4", "jump to stalk · tether · race · veils"),
    ("ENTER", "confirm"),
];

fn setup(mut commands: Commands) {
    commands.spawn((
        OverlayRoot,
        Name::new("Overlay"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(10),
        Visibility::Hidden,
    ));
}

fn text(s: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
        TextLayout::new_with_justify(Justify::Center),
    )
}

fn panel() -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(10.0),
            padding: UiRect::axes(Val::Px(48.0), Val::Px(32.0)),
            border: UiRect::all(Val::Px(1.0)),
            max_width: Val::Px(900.0),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor::all(ACCENT.with_alpha(0.35)),
    )
}

fn keys_table(p: &mut ChildSpawnerCommands, keys: &[(&str, &str)]) {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(4.0),
        margin: UiRect::top(Val::Px(8.0)),
        ..default()
    })
    .with_children(|t| {
        for (k, what) in keys {
            t.spawn(Node {
                column_gap: Val::Px(18.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Node {
                        width: Val::Px(190.0),
                        justify_content: JustifyContent::FlexEnd,
                        ..default()
                    },
                    children![(
                        Text::new(*k),
                        TextFont {
                            font_size: 17.0,
                            ..default()
                        },
                        TextColor(ACCENT),
                    )],
                ));
                row.spawn((
                    Text::new(*what),
                    TextFont {
                        font_size: 17.0,
                        ..default()
                    },
                    TextColor(INK),
                ));
            });
        }
    });
}

/// Rebuilds the overlay's contents when it changes.
fn render(
    mut commands: Commands,
    mut ux: ResMut<Ux>,
    phase: Res<State<GamePhase>>,
    mut root: Query<(Entity, &mut Visibility, &mut BackgroundColor), With<OverlayRoot>>,
) {
    if !ux.rebuild {
        return;
    }
    ux.rebuild = false;
    let Ok((root, mut vis, mut bg)) = root.single_mut() else {
        return;
    };
    commands.entity(root).despawn_related::<Children>();
    *vis = if ux.overlay.blocks() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let selected = ux.selected;
    match ux.overlay {
        Overlay::None => {}
        Overlay::Menu => {
            bg.0 = Color::NONE;
            commands.entity(root).with_children(|p| {
                p.spawn(text("E P H E M E R A", 64.0, ACCENT));
                p.spawn(text(
                    "a rider, a wild flyer, and a maze of worlds",
                    18.0,
                    DIM,
                ));
                p.spawn(Node {
                    height: Val::Px(28.0),
                    ..default()
                });
                menu_items(p, &MENU, selected);
                p.spawn(Node {
                    height: Val::Px(18.0),
                    ..default()
                });
                p.spawn(text(MENU[selected].hint(), 16.0, DIM));
                p.spawn(text(
                    "↑ / ↓  choose       ENTER  select       TAB  controls",
                    14.0,
                    DIM,
                ));
            });
        }
        Overlay::Pause => {
            bg.0 = VEIL;
            commands.entity(root).with_children(|p| {
                p.spawn(panel()).with_children(|p| {
                    p.spawn(text("PAUSED", 40.0, ACCENT));
                    menu_items(p, &PAUSE, selected);
                    p.spawn(text(PAUSE[selected].hint(), 15.0, DIM));
                });
            });
        }
        Overlay::Intro { story, card, .. } => {
            bg.0 = Color::srgba(0.0, 0.0, 0.02, 0.86);
            let (head, body) = story.cards()[card];
            let n = story.cards().len();
            commands.entity(root).with_children(|p| {
                p.spawn((FadeIn, text(head, 34.0, VIOLET)));
                p.spawn(Node {
                    height: Val::Px(18.0),
                    ..default()
                });
                p.spawn((FadeIn, text(body, 21.0, INK)));
                p.spawn(Node {
                    height: Val::Px(40.0),
                    ..default()
                });
                p.spawn(text(
                    &format!("{} / {n}       ENTER  next       ESC  skip", card + 1),
                    14.0,
                    DIM,
                ));
            });
        }
        Overlay::Briefing(ph) => {
            bg.0 = VEIL;
            let (head, body, keys) = briefing(ph);
            commands.entity(root).with_children(|p| {
                p.spawn(panel()).with_children(|p| {
                    p.spawn(text(head, 34.0, ACCENT));
                    p.spawn(text(body, 18.0, INK));
                    keys_table(p, keys);
                    p.spawn(Node {
                        height: Val::Px(12.0),
                        ..default()
                    });
                    p.spawn(text(
                        "ENTER  start       TAB  all controls       ESC  pause",
                        15.0,
                        DIM,
                    ));
                });
            });
        }
        Overlay::Controls { .. } => {
            bg.0 = VEIL;
            let current = *phase.get();
            commands.entity(root).with_children(|p| {
                p.spawn(panel()).with_children(|p| {
                    p.spawn(text("CONTROLS", 34.0, ACCENT));
                    for ph in [
                        GamePhase::Capture,
                        GamePhase::Taming,
                        GamePhase::Racing,
                        GamePhase::Explore,
                    ] {
                        // In a phase, its own keys first and only; from the
                        // menu, all of them.
                        if current != GamePhase::Title && ph != current {
                            continue;
                        }
                        let (head, _, keys) = briefing(ph);
                        p.spawn(text(head, 18.0, VIOLET));
                        keys_table(p, keys);
                    }
                    p.spawn(text("ANYWHERE", 18.0, VIOLET));
                    keys_table(p, &GLOBAL_KEYS);
                    p.spawn(text("ENTER / ESC / TAB  back", 14.0, DIM));
                });
            });
        }
    }
}

fn menu_items(p: &mut ChildSpawnerCommands, items: &[Choice], selected: usize) {
    for (i, c) in items.iter().enumerate() {
        let on = i == selected;
        p.spawn((
            Button,
            MenuItem(i),
            Node {
                width: Val::Px(360.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                border: UiRect::left(Val::Px(if on { 3.0 } else { 0.0 })),
                ..default()
            },
            BackgroundColor(if on {
                ACCENT.with_alpha(0.14)
            } else {
                Color::NONE
            }),
            BorderColor::all(ACCENT),
        ))
        .with_children(|b| {
            b.spawn(text(
                c.label(),
                if on { 24.0 } else { 21.0 },
                if on { ACCENT } else { INK },
            ));
        });
    }
}

/// Intro cards fade in.
fn fade(ux: Res<Ux>, mut q: Query<&mut TextColor, With<FadeIn>>) {
    let a = (ux.card_t / 1.2).clamp(0.0, 1.0);
    for mut c in &mut q {
        c.0 = c.0.with_alpha(a);
    }
}

/// Keys and clicks for whatever overlay is up; gameplay input is cleared
/// while it is.
#[allow(clippy::too_many_arguments)]
fn navigate(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut ux: ResMut<Ux>,
    mut axes: ResMut<InputAxes>,
    phase: Res<State<GamePhase>>,
    mut next: ResMut<NextState<GamePhase>>,
    mut stats: ResMut<EphemeraStats>,
    mut log: ResMut<RunLog>,
    mut exit: MessageWriter<AppExit>,
    items: Query<(&Interaction, &MenuItem), Changed<Interaction>>,
) {
    let blocked = ux.overlay.blocks();
    let here = *phase.get();
    let up = keys.any_just_pressed([KeyCode::ArrowUp, KeyCode::KeyW]);
    let down = keys.any_just_pressed([KeyCode::ArrowDown, KeyCode::KeyS]);
    let enter = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space]);
    let esc = keys.just_pressed(KeyCode::Escape);
    let tab = keys.just_pressed(KeyCode::Tab);

    let mut pick: Option<Choice> = None;
    let list = choices(ux.overlay);
    if !list.is_empty() {
        let n = list.len();
        if up {
            ux.selected = (ux.selected + n - 1) % n;
            ux.rebuild = true;
        }
        if down {
            ux.selected = (ux.selected + 1) % n;
            ux.rebuild = true;
        }
        for (i, item) in &items {
            match i {
                Interaction::Hovered if ux.selected != item.0 => {
                    ux.selected = item.0;
                    ux.rebuild = true;
                }
                Interaction::Pressed => pick = list.get(item.0).copied(),
                _ => {}
            }
        }
        if enter {
            pick = Some(list[ux.selected.min(n - 1)]);
        }
    }

    match ux.overlay {
        Overlay::None => {
            if esc && here != GamePhase::Title {
                ux.show(Overlay::Pause);
            } else if tab {
                ux.show(Overlay::Controls { from_pause: false });
            }
        }
        Overlay::Menu => {
            if tab {
                ux.show(Overlay::Controls { from_pause: false });
            } else if esc {
                exit.write(AppExit::Success);
            }
        }
        Overlay::Pause => {
            if esc {
                ux.show(Overlay::None);
            } else if tab {
                ux.show(Overlay::Controls { from_pause: true });
            }
        }
        Overlay::Controls { from_pause } => {
            if esc || tab || enter {
                ux.show(if from_pause {
                    Overlay::Pause
                } else if here == GamePhase::Title {
                    Overlay::Menu
                } else {
                    Overlay::None
                });
            }
        }
        Overlay::Briefing(_) => {
            if esc {
                ux.show(Overlay::Pause);
            } else if tab {
                ux.show(Overlay::Controls { from_pause: false });
            } else if enter {
                ux.show(Overlay::None);
            }
        }
        Overlay::Intro { story, card, then } => {
            ux.card_t += time.delta_secs();
            let last = card + 1 >= story.cards().len();
            if esc || (enter && last) {
                ux.show(Overlay::None);
                next.set(then);
            } else if enter || ux.card_t > 9.0 {
                if last {
                    ux.show(Overlay::None);
                    next.set(then);
                } else {
                    ux.show(Overlay::Intro {
                        story,
                        card: card + 1,
                        then,
                    });
                }
            }
        }
    }

    if let Some(c) = pick {
        match c {
            Choice::Rite => {
                *stats = EphemeraStats::default();
                *log = RunLog::default();
                ux.show(Overlay::Intro {
                    story: Story::Rite,
                    card: 0,
                    then: GamePhase::Capture,
                });
            }
            Choice::Veils => ux.show(Overlay::Intro {
                story: Story::Veils,
                card: 0,
                then: GamePhase::Explore,
            }),
            Choice::Race => {
                *stats = EphemeraStats::quick_race();
                ux.show(Overlay::None);
                next.set(GamePhase::Racing);
            }
            Choice::Controls => {
                let from_pause = ux.overlay == Overlay::Pause;
                ux.show(Overlay::Controls { from_pause });
            }
            Choice::Quit => {
                exit.write(AppExit::Success);
            }
            Choice::Resume => ux.show(Overlay::None),
            Choice::Restart => {
                ux.show(Overlay::None);
                if here == GamePhase::Explore {
                    next.set(GamePhase::ExploreReload);
                } else {
                    // Re-entering the same state is a no-op in Bevy: go via
                    // the title for one frame.
                    next.set(GamePhase::Title);
                    ux.pending = Some(here);
                }
            }
            Choice::Title => {
                ux.show(Overlay::None);
                next.set(GamePhase::Title);
            }
        }
    }

    // Gameplay hears nothing while an overlay is (or was, this frame) up.
    if blocked || ux.overlay.blocks() {
        *axes = InputAxes::default();
    }
}

/// Time stands still under an overlay, except on the title screen.
fn hold_time(ux: Res<Ux>, phase: Res<State<GamePhase>>, mut time: ResMut<Time<Virtual>>) {
    let hold = ux.overlay.blocks() && *phase.get() != GamePhase::Title;
    if hold && !time.is_paused() {
        time.pause();
    } else if !hold && time.is_paused() {
        time.unpause();
    }
}

fn enter_title(mut ux: ResMut<Ux>, mut next: ResMut<NextState<GamePhase>>) {
    if let Some(p) = ux.pending.take() {
        // A restart passing through.
        ux.show(Overlay::None);
        next.set(p);
        return;
    }
    ux.show(Overlay::Menu);
}

fn exit_title(mut ux: ResMut<Ux>) {
    if matches!(ux.overlay, Overlay::Menu | Overlay::Controls { .. }) {
        ux.show(Overlay::None);
    }
}

/// The first time a phase is entered, its briefing.
fn brief(phase: Res<State<GamePhase>>, settings: Res<UxSettings>, mut ux: ResMut<Ux>) {
    let p = *phase.get();
    if !settings.briefings || ux.briefed.contains(&p) {
        if !matches!(ux.overlay, Overlay::Pause) {
            ux.show(Overlay::None);
        }
        return;
    }
    ux.briefed.push(p);
    ux.show(Overlay::Briefing(p));
}

/// Without intros, the menu's journeys start at once.
fn skip_intros(
    settings: Res<UxSettings>,
    mut ux: ResMut<Ux>,
    mut next: ResMut<NextState<GamePhase>>,
) {
    if settings.intros {
        return;
    }
    if let Overlay::Intro { then, .. } = ux.overlay {
        ux.show(Overlay::None);
        next.set(then);
    }
}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Ux>()
            .init_resource::<UxSettings>()
            .add_systems(Startup, setup)
            .add_systems(OnEnter(GamePhase::Title), enter_title)
            .add_systems(OnExit(GamePhase::Title), exit_title)
            .add_systems(
                Update,
                (navigate, skip_intros, hold_time)
                    .chain()
                    .in_set(crate::GameSet::Input)
                    .after(crate::input::sample),
            )
            .add_systems(Update, (render, fade).chain().in_set(crate::GameSet::Hud));
        for p in [
            GamePhase::Capture,
            GamePhase::Taming,
            GamePhase::Racing,
            GamePhase::Explore,
        ] {
            app.add_systems(OnEnter(p), brief);
        }
    }
}
