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
            },
            ModelKind::Judith => Fit {
                scale: 1.8,
                yaw_deg: 180.0,
                y: 0.0,
            },
            _ => Fit {
                scale: 1.0,
                yaw_deg: 0.0,
                y: 0.0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Fit {
    scale: f32,
    yaw_deg: f32,
    y: f32,
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

/// Marks a model that replaced a placeholder (useful for animation hooks).
#[derive(Component)]
pub struct SwappedModel;

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
                SwappedModel,
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
