//! Optional 3D models, e.g. generated with meshy.ai.
//!
//! The game ships with primitive placeholders for everything. Drop a GLB at
//! `assets/models/<name>.glb` and every matching object switches to it on the
//! next run — no code change. Missing files simply keep the placeholder.
//!
//! Every kind of thing in the world has a slot (`ModelKind::file_stem`):
//! the creature, Judith, the villagers, every plant, rock, building and
//! animal. A slot holds any number of **variants**: `tree.glb`, `tree_2.glb`,
//! `tree_3.glb`… and each spawned thing picks one at random, so a forest is
//! not one tree repeated.
//!
//! Most slots are **auto-fitted**: the model's bounding box is scaled to the
//! size the world asked for (a 14 m tree, a 5 × 4 × 6 m house), its bottom
//! put on the ground, so any Meshy export fits without tuning. The creature,
//! Judith and the villagers are fitted by hand in `assets/models/models.cfg`
//! (`scale`, `yaw`, `y`), where any slot's auto fit can also be nudged.
//!
//! The mechanism is a plain ECS swap: spawners tag entities with `Swappable`,
//! and `swap_in_models` (on `Added<Swappable>`) hides the placeholder meshes
//! and attaches a `SceneRoot` child instead. Gameplay never knows.

use bevy::asset::io::file::FileAssetReader;
use bevy::ecs::system::RunSystemOnce;
use bevy::gltf::Gltf;
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKind {
    Ephemera,
    Judith,
    Villager,
    ReedTall,
    Reed,
    Gate,
    Mesa,
    Tree,
    Mushroom,
    Boulder,
    Spire,
    Crystal,
    Rib,
    Hut,
    House,
    Tower,
    Torch,
    Canoe,
    Wader,
    Grazer,
    Goat,
    Kite,
    Leaper,
}

impl ModelKind {
    pub const ALL: [ModelKind; 23] = [
        ModelKind::Ephemera,
        ModelKind::Judith,
        ModelKind::Villager,
        ModelKind::ReedTall,
        ModelKind::Reed,
        ModelKind::Gate,
        ModelKind::Mesa,
        ModelKind::Tree,
        ModelKind::Mushroom,
        ModelKind::Boulder,
        ModelKind::Spire,
        ModelKind::Crystal,
        ModelKind::Rib,
        ModelKind::Hut,
        ModelKind::House,
        ModelKind::Tower,
        ModelKind::Torch,
        ModelKind::Canoe,
        ModelKind::Wader,
        ModelKind::Grazer,
        ModelKind::Goat,
        ModelKind::Kite,
        ModelKind::Leaper,
    ];

    pub fn file_stem(self) -> &'static str {
        match self {
            ModelKind::Ephemera => "ephemera",
            ModelKind::Judith => "judith",
            ModelKind::Villager => "villager",
            ModelKind::ReedTall => "reed_tall",
            ModelKind::Reed => "reed",
            ModelKind::Gate => "gate",
            ModelKind::Mesa => "mesa",
            ModelKind::Tree => "tree",
            ModelKind::Mushroom => "mushroom",
            ModelKind::Boulder => "boulder",
            ModelKind::Spire => "spire",
            ModelKind::Crystal => "crystal",
            ModelKind::Rib => "rib",
            ModelKind::Hut => "hut",
            ModelKind::House => "house",
            ModelKind::Tower => "tower",
            ModelKind::Torch => "torch",
            ModelKind::Canoe => "canoe",
            ModelKind::Wader => "wader",
            ModelKind::Grazer => "grazer",
            ModelKind::Goat => "goat",
            ModelKind::Kite => "kite",
            ModelKind::Leaper => "leaper",
        }
    }

    /// Sensible defaults. Meshy (like most tools) exports glTF facing +Z,
    /// while Bevy's forward is -Z, hence the 180° for creatures. Everything
    /// but the three characters is auto-fitted to the size it is spawned at.
    fn default_fit(self) -> Fit {
        match self {
            ModelKind::Ephemera => Fit {
                scale: 6.0,
                yaw_deg: 180.0,
                y: -1.0,
                auto: false,
                rig: Default::default(),
            },
            ModelKind::Judith | ModelKind::Villager => Fit {
                scale: 1.8,
                yaw_deg: 180.0,
                y: 0.0,
                auto: false,
                rig: Default::default(),
            },
            ModelKind::ReedTall | ModelKind::Reed | ModelKind::Gate | ModelKind::Mesa => Fit {
                scale: 1.0,
                yaw_deg: 0.0,
                y: 0.0,
                auto: false,
                rig: Default::default(),
            },
            ModelKind::Wader
            | ModelKind::Grazer
            | ModelKind::Goat
            | ModelKind::Kite
            | ModelKind::Leaper => Fit {
                scale: 1.0,
                yaw_deg: 180.0,
                y: 0.0,
                auto: true,
                rig: Default::default(),
            },
            _ => Fit {
                scale: 1.0,
                yaw_deg: 0.0,
                y: 0.0,
                auto: true,
                rig: Default::default(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub scale: f32,
    pub yaw_deg: f32,
    pub y: f32,
    /// Fit the model's bounding box to the spawned size (`anim.rs`), with
    /// `scale` and `y` as nudges on top.
    pub auto: bool,
    pub rig: crate::rig::RigTuning,
}

#[derive(Clone, Debug)]
pub struct Model {
    scene: Handle<Scene>,
    /// The whole glTF, for its animation clips.
    gltf: Handle<Gltf>,
    fit: Fit,
}

#[derive(Resource, Default, Debug)]
pub struct Models {
    /// Every variant of every slot, in file order.
    loaded: Vec<(ModelKind, Model)>,
}

impl Models {
    /// One of the slot's variants, chosen by `variant` (any number).
    fn get(&self, kind: ModelKind, variant: u32) -> Option<&Model> {
        let n = self.loaded.iter().filter(|(k, _)| *k == kind).count();
        if n == 0 {
            return None;
        }
        self.loaded
            .iter()
            .filter(|(k, _)| *k == kind)
            .nth(variant as usize % n)
            .map(|(_, m)| m)
    }

    /// Slots with at least one model.
    pub fn kinds(&self) -> impl Iterator<Item = ModelKind> + '_ {
        let mut seen = Vec::new();
        self.loaded.iter().filter_map(move |(k, _)| {
            if seen.contains(k) {
                None
            } else {
                seen.push(*k);
                Some(*k)
            }
        })
    }

    /// For tests: use `scene` for `kind`, with the default fit.
    #[cfg(test)]
    pub fn with(mut self, kind: ModelKind, scene: Handle<Scene>) -> Self {
        self.loaded.push((
            kind,
            Model {
                scene,
                gltf: Handle::default(),
                fit: kind.default_fit(),
            },
        ));
        self
    }
}

/// Put this on an entity whose placeholder meshes may be replaced.
/// `size` is the box the model should fill (auto-fitted slots) or a
/// multiplier on the hand-tuned scale (the others). `variant` picks among
/// the slot's files.
#[derive(Component, Debug, Clone, Copy)]
pub struct Swappable {
    pub kind: ModelKind,
    pub size: Vec3,
    pub variant: u32,
}

impl Swappable {
    pub fn new(kind: ModelKind) -> Self {
        Self {
            kind,
            size: Vec3::ONE,
            variant: 0,
        }
    }

    pub fn sized(kind: ModelKind, size: Vec3) -> Self {
        Self {
            kind,
            size,
            variant: 0,
        }
    }

    pub fn variant(mut self, variant: u32) -> Self {
        self.variant = variant;
        self
    }
}

/// Marks a model that replaced a placeholder; the rigger and the measurer
/// read it.
#[derive(Component, Clone, Debug)]
pub struct SwappedModel {
    pub kind: ModelKind,
    pub fit: Fit,
    /// The box to fill (auto fit) or the scale multiplier (manual fit).
    pub size: Vec3,
    pub gltf: Handle<Gltf>,
}

impl SwappedModel {
    pub fn rig(&self) -> crate::rig::RigTuning {
        self.fit.rig
    }
}

fn parse_cfg(text: &str) -> Vec<(String, Fit)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
        let Some(kind) = ModelKind::ALL
            .iter()
            .find(|k| k.file_stem() == slot_of(name))
        else {
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
                "auto" => fit.auto = v != 0.0,
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

/// The slot a file belongs to: `tree_2` is a variant of `tree`.
pub fn slot_of(stem: &str) -> &str {
    match stem.rsplit_once('_') {
        Some((slot, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => slot,
        _ => stem,
    }
}

fn load_models(mut commands: Commands, assets: Res<AssetServer>) {
    let dir = FileAssetReader::get_base_path()
        .join("assets")
        .join("models");
    let overrides = std::fs::read_to_string(dir.join("models.cfg"))
        .map(|t| parse_cfg(&t))
        .unwrap_or_default();

    let mut stems: Vec<String> = std::fs::read_dir(&dir)
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "glb"))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    stems.sort();
    let mut models = Models::default();
    for stem in stems {
        let Some(kind) = ModelKind::ALL
            .iter()
            .copied()
            .find(|k| k.file_stem() == slot_of(&stem))
        else {
            if !stem.ends_with(".static") {
                warn!("assets/models/{stem}.glb: no slot of that name (see README.md)");
            }
            continue;
        };
        let file = format!("{stem}.glb");
        // A variant's own line first, then the slot's, then the default.
        let fit = overrides
            .iter()
            .find(|(n, _)| *n == stem)
            .or_else(|| overrides.iter().find(|(n, _)| n == kind.file_stem()))
            .map(|(_, f)| *f)
            .unwrap_or_else(|| kind.default_fit());
        info!("model: {file} → {} ({fit:?})", kind.file_stem());
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/{file}")));
        let gltf = assets.load(format!("models/{file}"));
        models.loaded.push((kind, Model { scene, gltf, fit }));
    }
    if !models.loaded.is_empty() {
        info!(
            "{} model file(s) for {} slot(s): {}",
            models.loaded.len(),
            models.kinds().count(),
            models
                .kinds()
                .map(|k| k.file_stem())
                .collect::<Vec<_>>()
                .join(", ")
        );
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
        let Some(model) = models.get(swap.kind, swap.variant) else {
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
        // Auto-fitted models are placed properly once measured (`anim.rs`);
        // until then they sit at the hand fit.
        let child = commands
            .spawn((
                SwappedModel {
                    kind: swap.kind,
                    fit,
                    size: swap.size,
                    gltf: model.gltf.clone(),
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
                        gltf: Handle::default(),
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
    fn variants_share_a_slot() {
        assert_eq!(slot_of("tree_2"), "tree");
        assert_eq!(slot_of("reed_tall"), "reed_tall");
        assert_eq!(slot_of("reed_tall_3"), "reed_tall");
        assert_eq!(slot_of("tree_"), "tree_");
        let m = Models::default()
            .with(ModelKind::Tree, Handle::default())
            .with(ModelKind::Tree, Handle::default())
            .with(ModelKind::Boulder, Handle::default());
        assert!(m.get(ModelKind::Tree, 7).is_some());
        assert!(m.get(ModelKind::House, 0).is_none());
        assert_eq!(m.kinds().count(), 2);
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
        .add_plugins((ModelsPlugin, crate::rig::RigPlugin, crate::anim::AnimPlugin));
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

    /// The screenshot bug: a riding clip with a horse baked into the file.
    /// The horse is hidden and left out of her measurements, and her hips
    /// end up on the Ephemera's saddle, not 1.2 m above it on the horse.
    #[test]
    fn rider_sits_on_the_saddle_without_her_horse() {
        use crate::anim::{Hips, ModelBounds, saddle_point};
        let mut app = app();
        let scene = load(&mut app, "rider_horse.glb");
        let gltf: Handle<Gltf> = app
            .world()
            .resource::<AssetServer>()
            .load("rider_horse.glb");
        let mut models = Models::default().with(ModelKind::Judith, scene);
        models.loaded[0].1.gltf = gltf;
        app.insert_resource(models);
        let saddle = Vec3::new(0.0, 1.1, 0.3);
        let mount = app
            .world_mut()
            .spawn((
                Flap::default(),
                ModelBounds {
                    bottom: -1.5,
                    top: 2.0,
                    saddle,
                },
                Transform::from_xyz(5.0, 10.0, -3.0).with_rotation(Quat::from_rotation_y(0.7)),
                Visibility::default(),
            ))
            .id();
        let judith = app
            .world_mut()
            .spawn((
                Swappable::new(ModelKind::Judith),
                Gait::default(),
                Transform::from_xyz(0.0, 0.1, -0.5).with_scale(Vec3::splat(0.9)),
                Visibility::default(),
                ChildOf(mount),
            ))
            .id();
        let found = run_until(&mut app, |w| w.get::<Hips>(judith).is_some());
        assert!(found, "her hips were never found");
        for _ in 0..30 {
            app.update();
        }
        // The horse is hidden; the character is not.
        let w = app.world_mut();
        let mut shown = 0;
        let mut hidden = 0;
        for (e, vis) in w
            .query_filtered::<(Entity, &Visibility), With<bevy::mesh::skinning::SkinnedMesh>>()
            .iter(w)
        {
            let _ = e;
            match vis {
                Visibility::Hidden => hidden += 1,
                _ => shown += 1,
            }
        }
        assert_eq!(
            (shown, hidden),
            (1, 1),
            "horse not hidden (shown {shown}, hidden {hidden})"
        );
        // Her measurements are hers: feet near her origin, not the horse's.
        let b = w.get::<ModelBounds>(judith).unwrap();
        assert!(
            b.bottom > 1.0,
            "measured with the horse: bottom {}",
            b.bottom
        );
        // And her hips sit on the saddle.
        let hips = w.get::<Hips>(judith).unwrap().0;
        let h = w.get::<GlobalTransform>(hips).unwrap().translation();
        let seat = w
            .get::<GlobalTransform>(mount)
            .unwrap()
            .transform_point(saddle_point(Some(&ModelBounds {
                bottom: -1.5,
                top: 2.0,
                saddle,
            })));
        assert!(h.distance(seat) < 0.05, "hips at {h}, saddle at {seat}");
    }

    /// The horse rigged into her own skeleton and mesh: its triangles are
    /// cut away, her measurements are hers, her hips go on the saddle.
    #[test]
    fn horse_in_her_own_skeleton_is_cut_away() {
        use crate::anim::{Hips, ModelBounds};
        let mut app = app();
        let scene = load(&mut app, "rider_horse_one.glb");
        let gltf: Handle<Gltf> = app
            .world()
            .resource::<AssetServer>()
            .load("rider_horse_one.glb");
        let mut models = Models::default().with(ModelKind::Judith, scene);
        models.loaded[0].1.gltf = gltf;
        app.insert_resource(models);
        let mount = app
            .world_mut()
            .spawn((
                Flap::default(),
                ModelBounds {
                    bottom: -1.5,
                    top: 2.0,
                    saddle: Vec3::new(0.0, 1.1, 0.3),
                },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        let judith = app
            .world_mut()
            .spawn((
                Swappable::new(ModelKind::Judith),
                Gait::default(),
                Transform::default(),
                Visibility::default(),
                ChildOf(mount),
            ))
            .id();
        assert!(run_until(&mut app, |w| w.get::<Hips>(judith).is_some()));
        for _ in 0..30 {
            app.update();
        }
        let w = app.world_mut();
        let (mesh, hidden) = w
            .query_filtered::<(&Mesh3d, &Visibility), With<bevy::mesh::skinning::SkinnedMesh>>()
            .single(w)
            .map(|(m, v)| (m.0.clone(), *v == Visibility::Hidden))
            .unwrap();
        assert!(!hidden, "she was hidden with her horse");
        let mesh = w.resource::<Assets<Mesh>>().get(&mesh).unwrap();
        // Her box: 20 triangles; the horse's 8 are gone.
        assert_eq!(
            mesh.indices().unwrap().len() / 3,
            20,
            "horse triangles left"
        );
        let b = w.get::<ModelBounds>(judith).unwrap();
        assert!(
            b.bottom > 1.0,
            "measured with the horse: bottom {}",
            b.bottom
        );
        let hips = w.get::<Hips>(judith).unwrap().0;
        let name = w.get::<Name>(hips).map(|n| n.as_str().to_string());
        assert_eq!(name.as_deref(), Some("Hips"));
    }

    /// An auto-fitted slot: any model is scaled to the box the world asked
    /// for, bottom on the ground.
    #[test]
    fn auto_fit_fills_the_requested_box() {
        let mut app = app();
        let scene = load(&mut app, "model.glb");
        app.insert_resource(Models::default().with(ModelKind::Tree, scene));
        let tree = app
            .world_mut()
            .spawn((
                Swappable::sized(ModelKind::Tree, Vec3::new(3.0, 10.0, 3.0)).variant(5),
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        let measured = run_until(&mut app, |w| {
            w.get::<crate::anim::ModelBounds>(tree).is_some()
        });
        assert!(measured, "never measured");
        let b = app.world().get::<crate::anim::ModelBounds>(tree).unwrap();
        assert!(b.bottom.abs() < 0.02, "bottom {}", b.bottom);
        assert!((b.top - 10.0).abs() < 0.1, "top {}", b.top);
        let w = app.world_mut();
        let placement = w
            .query_filtered::<&Transform, With<SwappedModel>>()
            .single(w)
            .unwrap();
        assert!(
            (placement.scale.y - 10.0).abs() < 0.1,
            "{:?}",
            placement.scale
        );
        assert!(
            (placement.scale.x - 3.0).abs() < 0.1,
            "{:?}",
            placement.scale
        );
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
        // Half a beat later: the other end of the stroke.
        app.world_mut().get_mut::<Flap>(creature).unwrap().phase = 4.7;
        app.update();
        let b = app.world().get::<Transform>(wing).unwrap().rotation;
        assert!(a.angle_between(b) > 0.2, "the wing didn't move ({a} → {b})");
    }

    /// A model that arrives rigged and animated (Meshy's Animate) keeps its
    /// skeleton: the game plays its clips, picking "Walking" once she moves.
    #[test]
    fn animated_model_plays_its_own_clips() {
        use crate::anim::{Action, Clips};
        let mut app = app();
        let scene = load(&mut app, "animated.glb");
        let gltf: Handle<Gltf> = app.world().resource::<AssetServer>().load("animated.glb");
        let mut models = Models::default().with(ModelKind::Judith, scene);
        models.loaded[0].1.gltf = gltf;
        app.insert_resource(models);
        let judith = app
            .world_mut()
            .spawn((
                Swappable::new(ModelKind::Judith),
                Gait::default(),
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        let adopted = run_until(&mut app, |w| w.query::<&Clips>().iter(w).count() > 0);
        assert!(adopted, "the clips were never adopted");
        assert_eq!(
            app.world_mut().query::<&Rig>().iter(app.world()).count(),
            0,
            "an animated model must not be auto-rigged"
        );
        // Measured too: feet at the origin, 1.8 tall.
        let b = app.world().get::<crate::anim::ModelBounds>(judith).unwrap();
        assert!(
            b.bottom.abs() < 1e-3 && (b.top - 1.8 * 1.8).abs() < 0.05,
            "{b:?}"
        );

        for _ in 0..5 {
            app.update();
        }
        let current = |w: &mut World| w.query::<&Clips>().single(w).unwrap().current;
        assert_eq!(current(app.world_mut()), Some(Action::Idle));
        for _ in 0..40 {
            app.world_mut()
                .get_mut::<Transform>(judith)
                .unwrap()
                .translation
                .z -= 0.05;
            app.update();
        }
        assert_eq!(current(app.world_mut()), Some(Action::Walk));
        // The player is really running the clip.
        let w = app.world_mut();
        let (player, clips) = {
            let c = w.query::<&Clips>().single(w).unwrap();
            (c.player, c.actions.clone())
        };
        let p = w.get::<AnimationPlayer>(player).unwrap();
        assert!(p.is_playing_animation(clips[&Action::Walk]));
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
