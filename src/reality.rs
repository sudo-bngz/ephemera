//! The maze of realities.
//!
//! Every world is an edition of the same work: a seed. Judith fell through
//! the veil at the Gathering and is lost among editions, each drawn from the
//! last. A **reality** is a seed and a *depth* — how many veils below the
//! origin it lies. Its veils are **portals**: one in three leads back up to
//! the reality it was drawn from, the rest down into realities drawn from
//! this one (`child_seed`). Which is which is not known until crossed.
//!
//! A veil is sealed until the reality has been *read*: enough of its
//! editions found (the landmarks counted by `explore.rs`). Then flying or
//! walking through it starts a **shift**: the next reality's chunks are
//! built in the background for the ground already loaded, and over a few
//! seconds the ground morphs from one relief into the other, the props of
//! the old world dissolve, the fog closes in in the veil's colour, the sky
//! turns, and the new world grows in. Judith keeps her place; the world
//! changes under her.
//!
//! Depth leaks into everything: the land works get it as a parameter, the
//! sky drifts in hue, the fog thickens.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::art::{Landmark, Work};
use crate::components::{CameraTarget, PhaseScoped};
use crate::hud::Hud;
use crate::state::GamePhase;
use crate::terrain::{Terrain, splitmix};
use crate::world::{
    Chunk, ChunkData, ChunkGround, Mood, Prop, WorldGen, WorldTerrain, build_chunk,
};

/// Editions found before a reality's veils open.
pub const READINGS_TO_OPEN: usize = 3;
/// Seconds the ground takes to become the next reality.
const MORPH_SECS: f32 = 4.0;

/// Where Judith has been: the origin first, the current reality last.
#[derive(Resource, Debug, Clone)]
pub struct Realities {
    pub path: Vec<u64>,
}

impl Realities {
    pub fn origin(seed: u64) -> Self {
        Self { path: vec![seed] }
    }

    pub fn current(&self) -> u64 {
        *self.path.last().expect("a reality")
    }

    pub fn depth(&self) -> u32 {
        (self.path.len() - 1) as u32
    }

    /// The reality above, if this is not the origin.
    pub fn above(&self) -> Option<u64> {
        (self.path.len() > 1).then(|| self.path[self.path.len() - 2])
    }
}

/// The seed of the reality a veil leads down to.
pub fn child_seed(seed: u64, veil: u64) -> u64 {
    splitmix(seed ^ splitmix(veil ^ 0xA1EA)) & 0xffff_ffff
}

/// Which way a veil leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Up,
    Down(u64),
}

/// A veil that is a way out of this reality.
#[derive(Component, Debug)]
pub struct Portal {
    pub gate: Gate,
    pub open: bool,
}

/// A shift under way: from the current reality to `to`.
#[derive(Resource)]
pub struct Shift {
    pub to: u64,
    pub depth: u32,
    land: Arc<crate::terrain::Land>,
    /// The next reality's chunks, for the ground already here.
    building: Vec<(Entity, Task<ChunkData>)>,
    built: Vec<(Entity, ChunkData)>,
    /// Morph progress, 0–1, once everything is built.
    pub t: f32,
    started: f32,
    swapped: bool,
    /// The sky still has to turn (done by `turn_sky`).
    sky_due: bool,
    colour: Color,
}

/// Veils become portals as they appear.
fn assign_portals(
    mut commands: Commands,
    realities: Res<Realities>,
    veils: Query<(Entity, &Landmark), Added<Landmark>>,
) {
    for (e, l) in &veils {
        if l.0.work != Work::Veil {
            continue;
        }
        // One veil in three leads up (none from the origin: every way out
        // of home leads deeper).
        let up = realities.above().is_some() && l.0.seed % 3 == 0;
        let gate = if up {
            Gate::Up
        } else {
            Gate::Down(child_seed(realities.current(), l.0.seed))
        };
        commands.entity(e).insert(Portal { gate, open: false });
    }
}

/// Veils open once the reality has been read.
fn unseal(readings: Res<crate::explore::Readings>, mut portals: Query<&mut Portal>) {
    let open = readings.count() >= READINGS_TO_OPEN;
    for mut p in &mut portals {
        if p.open != open {
            p.open = open;
        }
    }
}

/// Crossing an open veil starts the shift.
fn cross(
    mut commands: Commands,
    time: Res<Time>,
    shift: Option<Res<Shift>>,
    realities: Res<Realities>,
    mut wg: ResMut<WorldGen>,
    focus: Query<&GlobalTransform, With<CameraTarget>>,
    portals: Query<(&GlobalTransform, &Portal, &Landmark)>,
    grounds: Query<(Entity, &Chunk), With<ChunkGround>>,
    mut hud: ResMut<Hud>,
) {
    if shift.is_some() {
        return;
    }
    let Some(p) = focus.iter().next().map(|g| g.translation()) else {
        return;
    };
    for (gt, portal, landmark) in &portals {
        if !portal.open {
            continue;
        }
        // The veil is a rectangle in its own xy plane: inside it, within a
        // few metres of the plane.
        let local = gt.affine().inverse().transform_point3(p);
        let (w, h) = (gt.scale().x, gt.scale().y);
        let _ = (w, h);
        if local.x.abs() > 0.5 || local.y.abs() > 0.5 || local.z.abs() * gt.scale().z > 6.0 {
            continue;
        }
        let (to, depth) = match portal.gate {
            Gate::Up => (
                realities.above().expect("up from the origin"),
                realities.depth() - 1,
            ),
            Gate::Down(seed) => (seed, realities.depth() + 1),
        };
        info!(
            "crossing a veil: reality 0x{:x} (depth {}) → 0x{to:x} (depth {depth})",
            realities.current(),
            realities.depth()
        );
        // The next reality's land, and its chunks for the ground we stand on.
        let next = Terrain::at_depth(to, depth);
        let land = next.land.clone();
        let mut building = Vec::new();
        for (e, chunk) in &grounds {
            let centre = (chunk.0.as_vec2() + 0.5) * crate::world::CHUNK;
            let sampler = next.sampler(centre, crate::world::CHUNK);
            let c = chunk.0;
            building.push((
                e,
                AsyncComputeTaskPool::get().spawn(async move { build_chunk(sampler, c) }),
            ));
        }
        wg.paused = true;
        let colour = crate::art::edition_colour(landmark.0);
        commands.insert_resource(Shift {
            to,
            depth,
            land,
            building,
            built: Vec::new(),
            t: 0.0,
            started: time.elapsed_secs(),
            swapped: false,
            sky_due: false,
            colour,
        });
        hud.banner = if matches!(portal.gate, Gate::Up) {
            "— UP THROUGH THE VEIL —".into()
        } else {
            "— DOWN THROUGH THE VEIL —".into()
        };
        break;
    }
}

/// Runs a shift: waits for the next reality to be built, then morphs the
/// ground, dissolves the props, closes the fog, turns the sky, and finally
/// hands the world over to the new reality.
#[allow(clippy::too_many_arguments)]
fn shift(
    mut commands: Commands,
    time: Res<Time>,
    shift: Option<ResMut<Shift>>,
    mut realities: ResMut<Realities>,
    mut terrain: ResMut<WorldTerrain>,
    mut wg: ResMut<WorldGen>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    grounds: Query<&ChunkGround>,
    mut props: Query<(&Prop, &mut Transform)>,
    mut fog: Query<(&mut DistanceFog, &mut crate::camera::ChaseCamera)>,
    mut readings: ResMut<crate::explore::Readings>,
    mut hud: ResMut<Hud>,
) {
    let Some(mut shift) = shift else {
        return;
    };
    let dt = time.delta_secs();

    // Land finished builds; keep waiting (briefly) for the rest.
    let mut still = Vec::new();
    let building = std::mem::take(&mut shift.building);
    for (e, mut task) in building {
        match block_on(future::poll_once(&mut task)) {
            Some(data) => shift.built.push((e, data)),
            None => still.push((e, task)),
        }
    }
    let waited = time.elapsed_secs() - shift.started;
    if !still.is_empty() && waited < 8.0 {
        shift.building = still;
        return;
    }
    drop(still);

    // Morph.
    let t0 = shift.t;
    shift.t = (shift.t + dt / MORPH_SECS).min(1.0);
    let t = shift.t;
    let ease = t * t * (3.0 - 2.0 * t);
    terrain.0.blend = Some((shift.land.clone(), ease));

    for (e, data) in &shift.built {
        let Ok(ground) = grounds.get(*e) else {
            continue;
        };
        let mixed: Vec<f32> = ground
            .heights
            .iter()
            .zip(data.heights())
            .map(|(a, b)| a + (b - a) * ease)
            .collect();
        if let Some(mesh) = meshes.get_mut(&ground.mesh) {
            let (positions, normals) = ChunkData::ground_vertices(&mixed);
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        }
        if t >= 0.5 && !shift.swapped {
            if let Some(m) = materials.get_mut(&ground.material) {
                m.base_color_texture = Some(images.add(data.image()));
            }
        }
    }

    // Props dissolve between a fifth and four fifths of the way.
    let gone = ((t - 0.2) / 0.6).clamp(0.0, 1.0);
    for (p, mut tf) in &mut props {
        if p.base_scale.is_nan() {
            continue;
        }
        tf.scale = p.base_scale * (1.0 - gone).max(0.001);
    }

    // The fog closes in in the veil's colour and opens again; the camera
    // shakes as the ground moves.
    let palette = crate::sky::SkyPalette::for_mood(mood_of(shift.to)).drift(shift.depth);
    for (mut f, mut cam) in &mut fog {
        let veil = (1.0 - ((t - 0.5).abs() * 2.0)).clamp(0.0, 1.0).powf(0.6);
        let visibility = palette.visibility * (1.0 - 0.95 * veil) + 4.0;
        f.falloff = FogFalloff::from_visibility_squared(visibility);
        f.color = palette.horizon.mix(&shift.colour, veil * 0.8);
        cam.kick(0.08 * dt * 10.0);
    }

    // Halfway: the sky and the light become the next reality's.
    if t >= 0.5 && !shift.swapped {
        shift.swapped = true;
        shift.sky_due = true;
        hud.banner = format!("REALITY 0x{:x} — DEPTH {}", shift.to, shift.depth);
    }

    if t0 < 1.0 && t >= 1.0 {
        // Done: the world is the new reality. Old chunks go; streaming
        // rebuilds them from the new land (same ground, new props growing in).
        let to = shift.to;
        let depth = shift.depth;
        if depth < realities.depth() {
            realities.path.pop();
        } else {
            realities.path.push(to);
        }
        let mut next = terrain.0.with_land(shift.land.clone());
        next.flatten.clear();
        terrain.0 = next;
        for (_, e) in wg.chunks() {
            commands.entity(e).despawn();
        }
        wg.restart(to);
        wg.paused = false;
        wg.grow_until = time.elapsed_secs() + 6.0;
        readings.0.clear();
        for (mut f, _) in &mut fog {
            *f = palette.fog();
        }
        commands.remove_resource::<Shift>();
        info!("reality 0x{to:x} at depth {depth}");
    }
}

/// Halfway through a shift, the sky, the sun and the light are the next
/// reality's (hidden by the fog at its thickest).
fn turn_sky(
    mut commands: Commands,
    shift: Option<ResMut<Shift>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    sky: Query<Entity, Or<(With<crate::sky::SkyDome>, With<crate::sky::MesaRing>)>>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform)>,
    mut ambient: ResMut<AmbientLight>,
    mut clear: ResMut<ClearColor>,
) {
    let Some(mut shift) = shift else {
        return;
    };
    if !shift.sky_due {
        return;
    }
    shift.sky_due = false;
    let palette = crate::sky::SkyPalette::for_mood(mood_of(shift.to)).drift(shift.depth);
    for e in &sky {
        commands.entity(e).despawn();
    }
    crate::sky::spawn_sky(
        &mut commands,
        &mut meshes,
        &mut materials,
        &palette,
        shift.to,
    );
    for (mut light, mut tf) in &mut sun {
        light.color = palette.sun;
        light.illuminance = palette.sun_lux;
        *tf = Transform::default().looking_to(palette.sun_dir.normalize(), Vec3::Y);
    }
    ambient.color = palette.ambient;
    ambient.brightness = palette.ambient_brightness;
    clear.0 = palette.horizon;
}

/// Grow-in stops a while after a shift.
fn settle(time: Res<Time>, wg: Option<ResMut<WorldGen>>) {
    if let Some(mut wg) = wg {
        if wg.grow_until > 0.0 && time.elapsed_secs() > wg.grow_until {
            wg.grow_until = 0.0;
        }
    }
}

/// The light of a reality, from its seed.
pub fn mood_of(seed: u64) -> Mood {
    match seed % 3 {
        0 => Mood::Noon,
        1 => Mood::Dusk,
        _ => Mood::Dawn,
    }
}

fn teardown(mut commands: Commands) {
    commands.remove_resource::<Shift>();
}

pub struct RealityPlugin;

impl Plugin for RealityPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (assign_portals, unseal, cross, shift, turn_sky, settle)
                .chain()
                .in_set(crate::GameSet::Simulation)
                .run_if(in_state(GamePhase::Explore)),
        )
        .add_systems(OnExit(GamePhase::Explore), teardown);
        let _ = PhaseScoped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realities_form_a_path() {
        let mut r = Realities::origin(7);
        assert_eq!(r.depth(), 0);
        assert!(r.above().is_none());
        let down = child_seed(7, 99);
        assert_ne!(down, 7);
        assert_eq!(child_seed(7, 99), down, "deterministic");
        assert_ne!(child_seed(7, 98), down, "each veil its own reality");
        r.path.push(down);
        assert_eq!(r.depth(), 1);
        assert_eq!(r.above(), Some(7));
        assert_eq!(r.current(), down);
    }
}
