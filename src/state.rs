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
        ] {
            app.add_systems(OnExit(phase), despawn_phase_scoped);
        }
    }
}

fn despawn_phase_scoped(mut commands: Commands, q: Query<Entity, With<PhaseScoped>>) {
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
