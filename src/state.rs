//! Game phases and the handful of resources that survive across them.

use bevy::prelude::*;

use crate::components::PhaseScoped;

/// The three-phase rite of passage, plus title and results screens.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GamePhase {
    #[default]
    Title,
    /// Stealth: creep through the reeds and net a wild Ephemera.
    Capture,
    /// Resource management: a handful of dawns to turn a wild thing into a mount.
    Taming,
    /// High-speed flight race against the tribe's riders.
    Racing,
    Results,
    /// Free flight through an endless world (F4, `--explore`).
    Explore,
    /// One frame between two worlds: Bevy skips OnExit/OnEnter when a state
    /// is set to itself, so "new world" passes through here.
    ExploreReload,
}

/// The Ephemera's stats. Written during taming, read during racing.
/// Everything is normalised to `0.0..=1.0` so systems can compose them.
#[derive(Resource, Debug, Clone)]
pub struct EphemeraStats {
    /// How much the creature accepts Judith. Low trust → it bucks and
    /// randomly refuses boost in the race.
    pub trust: f32,
    /// Hunger. Reaches 1.0 and the creature escapes during taming;
    /// above 0.6 at race time it tires faster.
    pub hunger: f32,
    /// Raw speed. Raised by flight training.
    pub speed: f32,
    /// Turning ability. Raised by flight training.
    pub agility: f32,
    /// Boost reserve. Raised by feeding well, lowered by hunger.
    pub stamina: f32,
}

impl Default for EphemeraStats {
    fn default() -> Self {
        Self {
            trust: 0.10,
            hunger: 0.45,
            speed: 0.35,
            agility: 0.30,
            stamina: 0.40,
        }
    }
}

impl EphemeraStats {
    pub fn clamp(&mut self) {
        self.trust = self.trust.clamp(0.0, 1.0);
        self.hunger = self.hunger.clamp(0.0, 1.0);
        self.speed = self.speed.clamp(0.0, 1.0);
        self.agility = self.agility.clamp(0.0, 1.0);
        self.stamina = self.stamina.clamp(0.0, 1.0);
    }

    /// Derived racing parameters, in world units.
    pub fn top_speed(&self) -> f32 {
        40.0 + 60.0 * self.speed
    }
    pub fn acceleration(&self) -> f32 {
        22.0 + 26.0 * self.speed
    }
    pub fn yaw_rate(&self) -> f32 {
        1.6 + 1.6 * self.agility
    }
    pub fn boost_capacity(&self) -> f32 {
        let hunger_penalty = (self.hunger - 0.6).max(0.0) * 1.5;
        (2.5 + 4.0 * self.stamina - hunger_penalty).max(1.0)
    }
}

/// What happened during this run — shown on the results screen.
#[derive(Resource, Debug, Default, Clone)]
pub struct RunLog {
    pub capture_attempts: u32,
    pub creature_spooked: u32,
    pub dawns_spent: u32,
    pub best_lap: Option<f32>,
    pub total_time: f32,
    pub place: Option<u32>,
    pub last_outcome: Option<String>,
}

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GamePhase>()
            .init_resource::<EphemeraStats>()
            .init_resource::<RunLog>()
            .add_systems(Update, global_hotkeys.in_set(crate::GameSet::Input));
        // Every phase's entities are despawned when the phase ends.
        for phase in [
            GamePhase::Title,
            GamePhase::Capture,
            GamePhase::Taming,
            GamePhase::Racing,
            GamePhase::Results,
            GamePhase::Explore,
        ] {
            app.add_systems(OnExit(phase), despawn_phase_scoped);
        }
        app.add_systems(Startup, skip_from_cli);
    }
}

impl EphemeraStats {
    /// A reasonably tamed mount, used when jumping straight to the race.
    pub fn quick_race() -> Self {
        Self {
            trust: 0.65,
            hunger: 0.30,
            speed: 0.55,
            agility: 0.50,
            stamina: 0.60,
        }
    }
}

/// `cargo run --release -- --race` (or `--capture`, `--taming`) skips ahead.
/// `cargo run --release -- --explore --seed 0x2a` replays world 0x2a.
fn skip_from_cli(
    mut next: ResMut<NextState<GamePhase>>,
    mut stats: ResMut<EphemeraStats>,
    mut explore_seed: ResMut<crate::explore::ExploreSeed>,
) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--explore" => next.set(GamePhase::Explore),
            "--seed" => {
                let v = args.get(i + 1).map(String::as_str).unwrap_or("");
                let parsed = v
                    .strip_prefix("0x")
                    .map(|h| u64::from_str_radix(h, 16).ok())
                    .unwrap_or_else(|| v.parse().ok());
                match parsed {
                    Some(seed) => explore_seed.0 = Some(seed),
                    None => warn!("--seed needs a number (decimal or 0x…), got `{v}`"),
                }
            }
            "--capture" => next.set(GamePhase::Capture),
            "--taming" => next.set(GamePhase::Taming),
            "--race" => {
                *stats = EphemeraStats::quick_race();
                next.set(GamePhase::Racing);
            }
            _ => {}
        }
    }
}

/// Only roots are despawned: `despawn` is recursive, so a `PhaseScoped`
/// child (Judith in the saddle) is already gone by the time its own command
/// would run — despawning it again is what logged "entity does not exist".
fn despawn_phase_scoped(
    mut commands: Commands,
    q: Query<Entity, (With<PhaseScoped>, Without<ChildOf>)>,
) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn global_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    phase: Res<State<GamePhase>>,
    mut next: ResMut<NextState<GamePhase>>,
    mut stats: ResMut<EphemeraStats>,
    mut log: ResMut<RunLog>,
    mut exit: MessageWriter<AppExit>,
) {
    // F1–F4 jump straight to a phase — handy for testing a single one.
    if keys.just_pressed(KeyCode::F1) {
        next.set(GamePhase::Capture);
    } else if keys.just_pressed(KeyCode::F2) {
        next.set(GamePhase::Taming);
    } else if keys.just_pressed(KeyCode::F3) {
        *stats = EphemeraStats::quick_race();
        next.set(GamePhase::Racing);
    } else if keys.just_pressed(KeyCode::F4) {
        next.set(GamePhase::Explore);
    }

    // Escape always backs out to the title; from the title it quits.
    if keys.just_pressed(KeyCode::Escape) {
        if *phase.get() == GamePhase::Title {
            exit.write(AppExit::Success);
        } else {
            *stats = EphemeraStats::default();
            *log = RunLog::default();
            next.set(GamePhase::Title);
        }
    }
}
