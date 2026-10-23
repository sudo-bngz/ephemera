//! Phase 2 — taming, as a dawn-by-dawn resource-management loop.
//!
//! Judith has a handful of dawns before the Gathering. Each dawn she spends
//! on one thing: foraging nectar, feeding and grooming, flight training, or
//! resting. Three pools are juggled — the nectar pouch, her own energy and
//! the creature's hunger/trust — and the outcome is the `EphemeraStats` that
//! the race is then flown with. Let hunger or trust bottom out and the
//! creature snaps its tether and the stalk begins again.

use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::art::Art;
use crate::camera::ChaseCamera;
use crate::components::{CameraTarget, Ephemera, Judith, PhaseScoped, spawn_judith};
use crate::creature::{Coat, Flap, spawn_ephemera};
use crate::hud::Hud;
use crate::input::InputAxes;
use crate::state::{EphemeraStats, GamePhase, RunLog};
use crate::terrain::Flatten;
use crate::world::{Mood, WorldSetup, spawn_world};

pub const DAWNS: u32 = 7;
/// Height of the flattened camp clearing.
const CAMP_GROUND: f32 = 1.5;
const MIN_DAWNS_BEFORE_RACE: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Forage,
    Feed,
    Train,
    Rest,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Action::Forage => "Forage nectar",
            Action::Feed => "Feed & groom",
            Action::Train => "Flight training",
            Action::Rest => "Rest together",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Choosing,
    Playing { action: Action, t: f32 },
    Escaping(f32),
    Departing(f32),
}

#[derive(Resource)]
struct Taming {
    dawn: u32,
    /// Nectar pouch, 0..1.
    nectar: f32,
    /// Judith's energy, 0..1.
    energy: f32,
    stage: Stage,
    journal: Vec<String>,
    rng: StdRng,
}

#[derive(Component)]
struct Tethered {
    post: Vec3,
    angle: f32,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut art: ResMut<Art>,
    mut hud: ResMut<Hud>,
) {
    // The camp: a flat, dry clearing.
    let ground = CAMP_GROUND;
    let mut world = WorldSetup::new(311, Mood::Dawn);
    world.flatten.push(Flatten {
        centre: Vec2::ZERO,
        radius: 24.0,
        height: ground,
    });
    let (cam, _) = spawn_world(&mut commands, &mut meshes, &mut materials, &mut art, world);
    commands.entity(cam).insert(ChaseCamera {
        orbit: Some(0.08),
        ..default()
    });

    // Camp: a tether post and a hide tent.
    let wood = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.24, 0.16),
        perceptual_roughness: 0.9,
        ..default()
    });
    let hide = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.42, 0.30),
        perceptual_roughness: 1.0,
        ..default()
    });
    commands.spawn((
        PhaseScoped,
        Mesh3d(meshes.add(Cylinder::new(0.18, 3.0))),
        MeshMaterial3d(wood.clone()),
        Transform::from_xyz(0.0, ground + 1.5, 0.0),
    ));
    commands.spawn((
        PhaseScoped,
        Mesh3d(meshes.add(Cone::new(2.6, 3.2))),
        MeshMaterial3d(hide),
        Transform::from_xyz(-7.0, ground + 1.6, 4.0),
    ));
    // A ring of nectar gourds, purely decorative.
    for i in 0..5 {
        let a = i as f32 * 1.2;
        commands.spawn((
            PhaseScoped,
            Mesh3d(meshes.add(Sphere::new(0.4).mesh().ico(2).unwrap())),
            MeshMaterial3d(wood.clone()),
            Transform::from_xyz(-5.5 + a.cos() * 1.5, ground + 0.4, 7.5 + a.sin() * 1.5),
        ));
    }

    spawn_judith(
        &mut commands,
        &mut meshes,
        &mut materials,
        Vec3::new(-3.0, ground, 2.5),
    );

    let post = Vec3::new(0.0, ground + 2.2, 0.0);
    spawn_ephemera(
        &mut commands,
        &mut meshes,
        &mut materials,
        post + Vec3::new(4.0, 0.0, 0.0),
        Coat::WILD,
        0.15,
        (
            PhaseScoped,
            CameraTarget::CAMP,
            Tethered { post, angle: 0.0 },
        ),
    );

    commands.insert_resource(Taming {
        dawn: 1,
        nectar: 0.30,
        energy: 0.80,
        stage: Stage::Choosing,
        journal: vec!["The creature strains against the tether. It does not know her yet.".into()],
        rng: StdRng::seed_from_u64(99),
    });

    hud.title = "II. THE TETHER".into();
    hud.banner = String::new();
    hud.footer = format!(
        "1–4 choose what to do with this dawn · ENTER ride to the Gathering (after {} dawns, trust ≥ 30%)",
        MIN_DAWNS_BEFORE_RACE
    );
}

fn teardown(mut commands: Commands) {
    commands.remove_resource::<Taming>();
}

fn choose(
    axes: Res<InputAxes>,
    mut tam: ResMut<Taming>,
    mut stats: ResMut<EphemeraStats>,
    mut log: ResMut<RunLog>,
) {
    if tam.stage != Stage::Choosing {
        return;
    }

    if axes.confirm_pressed {
        if tam.dawn > MIN_DAWNS_BEFORE_RACE && stats.trust >= 0.30 {
            tam.stage = Stage::Departing(2.0);
            tam.journal
                .push("She climbs onto its back. The Gathering awaits.".into());
        } else {
            tam.journal
                .push("Not yet — it would throw her before the first gate.".into());
        }
        return;
    }

    let action = match axes.choice {
        Some(1) => Action::Forage,
        Some(2) => Action::Feed,
        Some(3) => Action::Train,
        Some(4) => Action::Rest,
        _ => return,
    };

    let mut s = stats.clone();
    let note: String;
    let mut spend_dawn = true;

    match action {
        Action::Forage => {
            let haul = 0.30 + tam.rng.gen_range(0.0..0.15);
            tam.nectar = (tam.nectar + haul).min(1.0);
            tam.energy -= 0.30;
            s.hunger += 0.14;
            s.trust -= 0.03;
            note = format!(
                "Foraged {:.0}% of a pouch. The creature was left alone all day.",
                haul * 100.0
            );
        }
        Action::Feed => {
            if tam.nectar < 0.25 {
                note = "The pouch is empty. Forage first.".into();
                spend_dawn = false;
            } else {
                tam.nectar -= 0.25;
                tam.energy -= 0.10;
                s.hunger -= 0.40;
                s.trust += 0.16 + 0.06 * (1.0 - s.trust);
                s.stamina += 0.08;
                note = "It drank from her hands. Its wings stilled.".into();
            }
        }
        Action::Train => {
            if tam.energy < 0.30 {
                note = "Judith is too tired to hold on.".into();
                spend_dawn = false;
            } else if s.trust < 0.20 {
                tam.energy -= 0.20;
                s.trust -= 0.05;
                s.hunger += 0.10;
                note = "It threw her in the mud before the first stroke. It does not trust her."
                    .into();
            } else {
                tam.energy -= 0.35;
                s.hunger += 0.20;
                let bond = 0.5 + 0.5 * s.trust;
                let dspeed = 0.13 * bond;
                let dagi = 0.12 * bond;
                s.speed += dspeed;
                s.agility += dagi;
                s.trust += if s.trust >= 0.40 { 0.05 } else { -0.04 };
                note = format!(
                    "Low over the pools, faster each pass. Speed +{:.0}, agility +{:.0}.",
                    dspeed * 100.0,
                    dagi * 100.0
                );
            }
        }
        Action::Rest => {
            tam.energy = (tam.energy + 0.50).min(1.0);
            s.hunger += 0.16;
            s.trust += 0.06;
            note = "A quiet dawn. She hums; it listens.".into();
        }
    }

    tam.energy = tam.energy.clamp(0.0, 1.0);
    s.clamp();
    *stats = s;
    tam.journal.push(note);
    if tam.journal.len() > 4 {
        tam.journal.remove(0);
    }

    if !spend_dawn {
        return;
    }
    log.dawns_spent += 1;
    tam.stage = Stage::Playing { action, t: 1.6 };

    if stats.hunger >= 1.0 || stats.trust <= 0.0 {
        tam.stage = Stage::Escaping(2.8);
        tam.journal
            .push("It tears the tether out of the mud and is gone.".into());
    }
}

fn play(
    time: Res<Time>,
    mut tam: ResMut<Taming>,
    mut stats: ResMut<EphemeraStats>,
    mut next: ResMut<NextState<GamePhase>>,
    mut log: ResMut<RunLog>,
    mut creature: Query<(&mut Transform, &mut Tethered, &mut Flap), With<Ephemera>>,
    mut judith: Query<&mut Transform, (With<Judith>, Without<Ephemera>)>,
) {
    let dt = time.delta_secs();
    let Ok((mut etf, mut tether, mut flap)) = creature.single_mut() else {
        return;
    };

    // Idle orbit on the tether, slow and low.
    let (radius, height, speed, intensity) = match tam.stage {
        Stage::Playing {
            action: Action::Train,
            ..
        } => (9.0, 5.0, 2.2, 0.85),
        Stage::Playing {
            action: Action::Feed,
            ..
        } => (2.0, 0.0, 0.0, 0.05),
        Stage::Playing {
            action: Action::Rest,
            ..
        } => (3.5, 0.0, 0.15, 0.02),
        Stage::Escaping(_) => (16.0, 14.0, 3.0, 1.0),
        Stage::Departing(_) => (6.0, 3.0, 1.4, 0.7),
        _ => (4.0, 0.0, 0.25, 0.12 + 0.3 * stats.hunger),
    };
    tether.angle += speed * dt;
    let wanted = tether.post
        + Vec3::new(
            tether.angle.cos() * radius,
            height,
            tether.angle.sin() * radius,
        );
    etf.translation = etf.translation.lerp(wanted, 1.0 - (-3.0 * dt).exp());
    let tangent = Vec3::new(-tether.angle.sin(), 0.0, tether.angle.cos());
    let yaw = (-tangent.x).atan2(-tangent.z);
    etf.rotation = etf
        .rotation
        .slerp(Quat::from_rotation_y(yaw), 1.0 - (-4.0 * dt).exp());
    flap.intensity += (intensity - flap.intensity) * (1.0 - (-4.0 * dt).exp());

    // Judith walks up to the creature when feeding, otherwise stays by camp.
    if let Ok(mut jtf) = judith.single_mut() {
        let wanted = match tam.stage {
            Stage::Playing {
                action: Action::Feed,
                ..
            } => etf.translation.with_y(CAMP_GROUND) + Vec3::new(0.0, 0.0, 2.6),
            Stage::Departing(_) => etf.translation + Vec3::new(0.0, 0.9, 0.0),
            _ => Vec3::new(-3.0, CAMP_GROUND, 2.5),
        };
        jtf.translation = jtf.translation.lerp(wanted, 1.0 - (-3.0 * dt).exp());
        let to = (etf.translation - jtf.translation).xz();
        if to.length() > 0.1 {
            jtf.rotation = Quat::from_rotation_y((-to.x).atan2(-to.y));
        }
    }

    match tam.stage {
        Stage::Playing { action, t } => {
            let t = t - dt;
            if t <= 0.0 {
                tam.dawn += 1;
                tam.stage = Stage::Choosing;
                if tam.dawn > DAWNS {
                    tam.stage = Stage::Departing(2.0);
                    tam.journal
                        .push("The last dawn. Ready or not, the Gathering begins.".into());
                }
            } else {
                tam.stage = Stage::Playing { action, t };
            }
        }
        Stage::Escaping(t) => {
            let t = t - dt;
            if t <= 0.0 {
                log.last_outcome = Some("The creature escaped during taming.".into());
                *stats = EphemeraStats::default();
                next.set(GamePhase::Capture);
            } else {
                tam.stage = Stage::Escaping(t);
            }
        }
        Stage::Departing(t) => {
            let t = t - dt;
            if t <= 0.0 {
                next.set(GamePhase::Racing);
            } else {
                tam.stage = Stage::Departing(t);
            }
        }
        Stage::Choosing => {}
    }
}

fn hud(tam: Res<Taming>, stats: Res<EphemeraStats>, mut hud: ResMut<Hud>) {
    let bar = |v: f32| -> String {
        let n = ((v * 12.0).round() as usize).min(12);
        format!("{}{}", "█".repeat(n), "░".repeat(12 - n))
    };
    hud.side = format!(
        "EPHEMERA\ntrust    {} {:>3.0}%\nhunger   {} {:>3.0}%\nspeed    {} {:>3.0}%\nagility  {} {:>3.0}%\nstamina  {} {:>3.0}%\n\nJUDITH\nenergy   {} {:>3.0}%\nnectar   {} {:>3.0}%",
        bar(stats.trust),
        stats.trust * 100.0,
        bar(stats.hunger),
        stats.hunger * 100.0,
        bar(stats.speed),
        stats.speed * 100.0,
        bar(stats.agility),
        stats.agility * 100.0,
        bar(stats.stamina),
        stats.stamina * 100.0,
        bar(tam.energy),
        tam.energy * 100.0,
        bar(tam.nectar),
        tam.nectar * 100.0,
    );

    let menu = match tam.stage {
        Stage::Choosing => format!(
            "\n  1  {:<16} +nectar  −energy   (creature left alone)\n  2  {:<16} −nectar  −hunger   +trust +stamina\n  3  {:<16} −energy  +hunger   +speed +agility (needs trust)\n  4  {:<16} +energy  +hunger   +trust",
            Action::Forage.name(),
            Action::Feed.name(),
            Action::Train.name(),
            Action::Rest.name(),
        ),
        Stage::Playing { action, .. } => format!("\n  {}…", action.name()),
        Stage::Escaping(_) => "\n  …".into(),
        Stage::Departing(_) => "\n  To the Gathering.".into(),
    };
    hud.body = format!(
        "Dawn {} of {}\n{}\n\n{}",
        tam.dawn.min(DAWNS),
        DAWNS,
        menu,
        tam.journal.join("\n")
    );
    hud.banner = match tam.stage {
        Stage::Escaping(_) => "ESCAPED".into(),
        _ => String::new(),
    };
}

pub struct TamingPlugin;

impl Plugin for TamingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GamePhase::Taming), setup)
            .add_systems(OnExit(GamePhase::Taming), teardown)
            .add_systems(
                Update,
                (choose, play)
                    .chain()
                    .in_set(crate::GameSet::Simulation)
                    .run_if(in_state(GamePhase::Taming)),
            )
            .add_systems(
                Update,
                hud.in_set(crate::GameSet::Hud)
                    .run_if(in_state(GamePhase::Taming)),
            );
    }
}
