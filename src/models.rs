//! Optional 3D models, e.g. generated with meshy.ai.
//!
//! The game ships with primitive placeholders for everything. Drop a GLB at
//! `assets/models/<name>.glb` and every matching object switches to it on the
//! next run — no code change. Missing files simply keep the placeholder.
//!
//! | file              | replaces                                   |
//! |-------------------|--------------------------------------------|
//! | `ephemera.glb`    | the creature (player and rivals)           |
//! | `judith.glb`      | Judith, on foot and in the saddle          |
//! | `reed_tall.glb`   | tall (solid) reed clumps                   |
//! | `reed.glb`        | short reed clumps                          |
//! | `gate.glb`        | race gates                                 |
//! | `mesa.glb`        | distant rock formations on the horizon     |
//!
//! Per-model fit is tuned in `assets/models/models.cfg`, one line each:
//!
//! ```text
//! ephemera  scale=6.0  yaw=180  y=-1.0
//! ```
//!
//! The mechanism is a plain ECS swap: spawners tag entities with `Swappable`,
//! and `swap_in_models` (on `Added<Swappable>`) hides the placeholder meshes
//! and attaches a `SceneRoot` child instead. Gameplay never knows.

use bevy::asset::io::file::FileAssetReader;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKind {
    Ephemera,
    Judith,
    ReedTall,
    Reed,
    Gate,
    Mesa,
}

impl ModelKind {
    const ALL: [ModelKind; 6] = [
        ModelKind::Ephemera,
        ModelKind::Judith,
        ModelKind::ReedTall,
        ModelKind::Reed,
        ModelKind::Gate,
        ModelKind::Mesa,
    ];

    pub fn file_stem(self) -> &'static str {
        match self {
            ModelKind::Ephemera => "ephemera",
            ModelKind::Judith => "judith",
            ModelKind::ReedTall => "reed_tall",
            ModelKind::Reed => "reed",
            ModelKind::Gate => "gate",
            ModelKind::Mesa => "mesa",
        }
    }

    /// Sensible defaults. Meshy (like most tools) exports glTF facing +Z,
    /// while Bevy's forward is -Z, hence the 180° for creatures.
    fn default_fit(self) -> Fit {
        match self {
            ModelKind::Ephemera => Fit {
                scale: 6.0,
                yaw_deg: 180.0,
                y: -1.0,
                rig: Default::default(),
            },
            ModelKind::Judith => Fit {
                scale: 1.8,
                yaw_deg: 180.0,
                y: 0.0,
                rig: Default::default(),
            },
            _ => Fit {
                scale: 1.0,
                yaw_deg: 0.0,
                y: 0.0,
                rig: Default::default(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Fit {
    scale: f32,
    yaw_deg: f32,
    y: f32,
    rig: crate::rig::RigTuning,
}

#[derive(Clone, Debug)]
pub struct Model {
    scene: Handle<Scene>,
    fit: Fit,
}

#[derive(Resource, Default, Debug)]
pub struct Models {
    loaded: Vec<(ModelKind, Model)>,
}

impl Models {
    fn get(&self, kind: ModelKind) -> Option<&Model> {
        self.loaded.iter().find(|(k, _)| *k == kind).map(|(_, m)| m)
    }

    /// For tests: use `scene` for `kind`, with the default fit.
    #[cfg(test)]
    pub fn with(mut self, kind: ModelKind, scene: Handle<Scene>) -> Self {
        self.loaded.push((
            kind,
            Model {
                scene,
                fit: kind.default_fit(),
            },
        ));
        self
    }
}

/// Put this on an entity whose placeholder meshes may be replaced.
/// `size` multiplies the model's fitted scale (e.g. a clump's radius/height).
#[derive(Component, Debug, Clone, Copy)]
pub struct Swappable {
    pub kind: ModelKind,
    pub size: Vec3,
}

impl Swappable {
    pub fn new(kind: ModelKind) -> Self {
        Self {
            kind,
            size: Vec3::ONE,
        }
    }

    pub fn sized(kind: ModelKind, size: Vec3) -> Self {
        Self { kind, size }
    }
}

/// Marks a model that replaced a placeholder; the rigger reads it.
#[derive(Component, Clone, Copy, Debug)]
pub struct SwappedModel {
    pub kind: ModelKind,
    pub rig: crate::rig::RigTuning,
}

fn parse_cfg(text: &str) -> Vec<(String, Fit)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
        let Some(kind) = ModelKind::ALL.iter().find(|k| k.file_stem() == name) else {
            warn!("models.cfg: unknown model `{name}`");
            continue;
        };
        let mut fit = kind.default_fit();
        for kv in parts {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            let Ok(v) = v.parse::<f32>() else {
                warn!("models.cfg: `{kv}` is not a number");
                continue;
            };
            match k {
                "scale" => fit.scale = v,
                "yaw" => fit.yaw_deg = v,
                "y" => fit.y = v,
                // Rig thresholds (see rig.rs), fractions of the model's size.
                "wing_hinge" => fit.rig.wing_hinge = v,
                "wing_min_y" => fit.rig.wing_min_y = v,
                "leg_max_y" => fit.rig.leg_max_y = v,
                _ => warn!("models.cfg: unknown key `{k}`"),
            }
        }
        out.push((name.to_string(), fit));
    }
    out
}

fn load_models(mut commands: Commands, assets: Res<AssetServer>) {
    let dir = FileAssetReader::get_base_path()
        .join("assets")
        .join("models");
    let overrides = std::fs::read_to_string(dir.join("models.cfg"))
        .map(|t| parse_cfg(&t))
        .unwrap_or_default();

    let mut models = Models::default();
    for kind in ModelKind::ALL {
        let file = format!("{}.glb", kind.file_stem());
        if !dir.join(&file).exists() {
            continue;
        }
        let fit = overrides
            .iter()
            .find(|(n, _)| n == kind.file_stem())
            .map(|(_, f)| *f)
            .unwrap_or_else(|| kind.default_fit());
        info!("model override: {file} ({fit:?})");
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/{file}")));
        models.loaded.push((kind, Model { scene, fit }));
    }
    commands.insert_resource(models);
}

fn swap_in_models(
    mut commands: Commands,
    models: Res<Models>,
    added: Query<(Entity, &Swappable, Option<&Children>, Has<Mesh3d>), Added<Swappable>>,
    nested: Query<(), With<Swappable>>,
) {
    if models.loaded.is_empty() {
        return;
    }
    for (entity, swap, children, has_own_mesh) in &added {
        let Some(model) = models.get(swap.kind) else {
            continue;
        };
        // Hide the placeholder: the entity's own mesh, and/or its children.
        if has_own_mesh {
            commands.entity(entity).remove::<Mesh3d>();
        }
        if let Some(children) = children {
            for child in children.iter() {
                // Leave nested swappables alone: Judith rides as a child of
                // her Ephemera and has a model slot of her own.
                if !nested.contains(child) {
                    commands.entity(child).insert(Visibility::Hidden);
                }
            }
        }
        let fit = model.fit;
        let child = commands
            .spawn((
                SwappedModel {
                    kind: swap.kind,
                    rig: fit.rig,
                },
                SceneRoot(model.scene.clone()),
                Transform::from_xyz(0.0, fit.y * swap.size.y, 0.0)
                    .with_rotation(Quat::from_rotation_y(fit.yaw_deg.to_radians()))
                    .with_scale(swap.size * fit.scale),
            ))
            .id();
        commands.entity(entity).add_child(child);
    }
}

pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        // Needs the AssetServer, which `AssetPlugin` (in DefaultPlugins)
        // has inserted by now; run immediately so phases entered on the very
        // first frame (`--race`) already see the overrides.
        app.world_mut()
            .run_system_once(load_models)
            .expect("models need the AssetServer");
        app.add_systems(PostUpdate, swap_in_models);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With a model available, the placeholder children get hidden, a
    /// scene child is attached — and a nested swappable (Judith in the
    /// saddle) is left visible.
    #[test]
    fn swap_hides_placeholder_but_not_rider() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Models {
                loaded: vec![(
                    ModelKind::Ephemera,
                    Model {
                        scene: Handle::default(),
                        fit: ModelKind::Ephemera.default_fit(),
                    },
                )],
            })
            .add_systems(Update, swap_in_models);

        let world = app.world_mut();
        let wing = world
            .spawn((Transform::default(), Visibility::default()))
            .id();
        let rider = world
            .spawn((Swappable::new(ModelKind::Judith), Visibility::default()))
            .id();
        let creature = world
            .spawn((Swappable::new(ModelKind::Ephemera), Transform::default()))
            .add_children(&[wing, rider])
            .id();
        app.update();

        let world = app.world();
        assert_eq!(world.get::<Visibility>(wing), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Visibility>(rider), Some(&Visibility::Inherited));
        let children = world.get::<Children>(creature).unwrap();
        assert!(
            children
                .iter()
                .any(|c| world.get::<SwappedModel>(c).is_some()),
            "a SceneRoot child should have been attached"
        );
        assert!(
            world.get::<SwappedModel>(rider).is_none(),
            "Judith has no model loaded, so she keeps her placeholder"
        );
    }

    /// The shipped template must only mention models the game knows about.
    #[test]
    fn shipped_cfg_is_valid() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/models/models.cfg");
        let text = std::fs::read_to_string(path).expect("assets/models/models.cfg");
        let named = text
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
            .filter(|l| !l.is_empty())
            .count();
        assert_eq!(
            parse_cfg(&text).len(),
            named,
            "unknown model name in models.cfg"
        );
    }

    #[test]
    fn cfg_parsing() {
        let fits = parse_cfg(
            "# comment\n\
             ephemera scale=4.5 yaw=90   # trailing\n\
             gate y=2\n\
             nonsense scale=1\n\
             judith scale=abc\n",
        );
        assert_eq!(fits.len(), 3);
        assert_eq!(fits[0].0, "ephemera");
        assert_eq!(fits[0].1.scale, 4.5);
        assert_eq!(fits[0].1.yaw_deg, 90.0);
        assert_eq!(fits[0].1.y, -1.0, "unspecified keys keep the default");
        assert_eq!(fits[1].1.y, 2.0);
        assert_eq!(fits[2].1.scale, 1.8, "bad number keeps the default");
    }
}

#[cfg(test)]
mod gltf_tests {
    //! Real GLBs (shaped like Meshy's output) through the game's own
    //! pipeline — swap, load, spawn, rig, animate — with the same plugins
    //! as the game, minus the window and the GPU.
    use super::*;
    use crate::creature::Flap;
    use crate::rig::{Gait, Rig, RigKind};
    use bevy::mesh::skinning::SkinnedMesh;
    use bevy::render::RenderPlugin;
    use bevy::render::settings::{RenderCreation, WgpuSettings};
    use bevy::window::ExitCondition;
    use bevy::winit::WinitPlugin;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<WinitPlugin>()
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(WgpuSettings {
                        backends: None,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures").into(),
                    ..default()
                }),
        )
        .init_state::<crate::state::GamePhase>()
        .init_resource::<crate::input::InputAxes>()
        .add_plugins((ModelsPlugin, crate::rig::RigPlugin));
        app.finish();
        app.cleanup();
        app
    }

    fn load(app: &mut App, file: &str) -> Handle<Scene> {
        app.world()
            .resource::<AssetServer>()
            .load(GltfAssetLabel::Scene(0).from_asset(file.to_string()))
    }

    fn run_until(app: &mut App, mut done: impl FnMut(&mut World) -> bool) -> bool {
        for _ in 0..600 {
            app.update();
            if done(app.world_mut()) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    fn rig_of(world: &mut World) -> Option<(Vec<Entity>, RigKind)> {
        world
            .query::<&Rig>()
            .iter(world)
            .find(|r| {
                !r.joints.is_empty() && r.to_char != Quat::IDENTITY || r.kind == RigKind::Wings
            })
            .map(|r| (r.joints.clone(), r.kind))
    }

    #[test]
    fn meshy_like_glb_spawns() {
        let mut app = app();
        let scene = load(&mut app, "model.glb");
        app.world_mut().spawn(SceneRoot(scene));
        let spawned = run_until(&mut app, |w| w.query::<&Mesh3d>().iter(w).count() > 0);
        assert!(spawned, "the GLB scene never spawned a mesh");
    }

    /// A static mosquito model becomes a skinned mesh whose wings beat.
    #[test]
    fn ephemera_model_is_rigged_and_flaps() {
        let mut app = app();
        let scene = load(&mut app, "mosquito.glb");
        let models = Models::default().with(ModelKind::Ephemera, scene);
        app.insert_resource(models);
        let creature = app
            .world_mut()
            .spawn((
                Swappable::new(ModelKind::Ephemera),
                Flap {
                    intensity: 0.8,
                    phase: 0.4,
                },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        let rigged = run_until(&mut app, |w| w.query::<&Rig>().iter(w).count() > 0);
        assert!(rigged, "the model was never rigged");

        let (joints, kind) = {
            let w = app.world_mut();
            let r = w.query::<&Rig>().single(w).unwrap();
            (r.joints.clone(), r.kind)
        };
        assert_eq!(kind, RigKind::Wings);
        assert_eq!(joints.len(), crate::rig::wing::COUNT);
        // The mesh is skinned to those joints, with weights on every vertex.
        let w = app.world_mut();
        let (skin, mesh) = w
            .query::<(&SkinnedMesh, &Mesh3d)>()
            .single(w)
            .expect("one skinned mesh");
        assert_eq!(skin.joints, joints);
        let mesh = w.resource::<Assets<Mesh>>().get(&mesh.0).unwrap();
        assert!(mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some());

        // Wings beat as the flap's phase turns.
        let wing = joints[crate::rig::wing::WING_L];
        app.update();
        let a = app.world().get::<Transform>(wing).unwrap().rotation;
        app.world_mut().get_mut::<Flap>(creature).unwrap().phase = 2.0;
        app.update();
        let b = app.world().get::<Transform>(wing).unwrap().rotation;
        assert!(a.angle_between(b) > 0.2, "the wing didn't move ({a} → {b})");
    }

    /// A static T-posed model becomes a skinned humanoid that walks.
    #[test]
    fn judith_model_is_rigged_and_walks() {
        let mut app = app();
        let scene = load(&mut app, "humanoid.glb");
        app.insert_resource(Models::default().with(ModelKind::Judith, scene));
        let judith = app
            .world_mut()
            .spawn((
                Swappable::new(ModelKind::Judith),
                Gait::default(),
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        let rigged = run_until(&mut app, |w| w.query::<&Rig>().iter(w).count() > 0);
        assert!(rigged, "the model was never rigged");
        let (joints, kind) = rig_of(app.world_mut()).unwrap();
        assert_eq!(kind, RigKind::Humanoid);
        let knee = joints[crate::rig::human::KNEE[0]];
        let hip = joints[crate::rig::human::HIP[0]];

        // Standing still, then walking: the legs swing.
        for _ in 0..10 {
            app.update();
        }
        let still = app.world().get::<Transform>(hip).unwrap().rotation;
        let mut swing: f32 = 0.0;
        for k in 0..60 {
            app.world_mut()
                .get_mut::<Transform>(judith)
                .unwrap()
                .translation
                .z -= 0.06;
            app.update();
            let _ = k;
            let r = app.world().get::<Transform>(hip).unwrap().rotation;
            swing = swing.max(still.angle_between(r));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(swing > 0.2, "walking didn't swing the hip ({swing})");
        let bent = app.world().get::<Transform>(knee).unwrap().rotation;
        assert_ne!(bent, Quat::IDENTITY);
    }
}
