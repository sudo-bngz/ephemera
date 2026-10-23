//! The tribe's settlements: stilt villages over the shallows, walled towns
//! on flattened ground, holds of towers on the mountains' shoulders.
//!
//! Where they stand is decided by the terrain (`terrain::Site`, a pure
//! function of the world seed, so every chunk agrees); what they look like
//! is drawn here from the site's own seed. The clans live tribally but build
//! like no one on Earth: grown ceramic, alloy and smoked glass, domes and
//! spires, plasma lamps. Every settlement has people in it, rigged figures
//! who wander between the dwellings.

use bevy::ecs::system::RunSystemOnce;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::forms::{FormKind, FormSlot};
use crate::models::{ModelKind, Swappable};
use crate::terrain::{Props, SEA_LEVEL, Site, SiteKind, Terrain};
use crate::world::{Obstacle, WetlandAssets};

/// Deck height of a stilt village over the water.
const DECK: f32 = 2.4;

/// Someone of the tribe going about their day: walks to a spot near home,
/// waits, picks another.
#[derive(Component, Debug)]
pub struct Villager {
    pub home: Vec3,
    pub radius: f32,
    pub target: Vec3,
    pub pause: f32,
    pub speed: f32,
}

/// A flame: its scale flickers, so the shared material can stay shared.
#[derive(Component)]
pub struct Torch {
    pub phase: f32,
    pub size: f32,
}

/// A craft going round a city: on a ring road, or circling in the air.
#[derive(Component, Debug)]
pub struct Traffic {
    pub radius: f32,
    /// Radians per second (signed: the direction).
    pub speed: f32,
    pub angle: f32,
    pub height: f32,
    /// Flying craft weave up and down.
    pub weave: f32,
}

/// Cloth snapping in the wind.
#[derive(Component)]
pub struct Banner(pub f32);

#[derive(Resource)]
pub struct SettlementAssets {
    post: Handle<Mesh>,
    round_wall: Handle<Mesh>,
    cone_roof: Handle<Mesh>,
    block: Handle<Mesh>,
    pyramid_roof: Handle<Mesh>,
    disc: Handle<Mesh>,
    flame: Handle<Mesh>,
    cloth: Handle<Mesh>,
    wood: Handle<StandardMaterial>,
    thatch: Handle<StandardMaterial>,
    stones: Vec<Handle<StandardMaterial>>,
    slate: Handle<StandardMaterial>,
    pub flame_mat: Handle<StandardMaterial>,
    banners: Vec<Handle<StandardMaterial>>,
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // The clans build in grown ceramic, alloy and smoked glass, lit from
    // within: alien and engineered, not timber and thatch.
    let rough = |m: &mut Assets<StandardMaterial>, c: Color| {
        m.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: 0.3,
            reflectance: 0.6,
            ..default()
        })
    };
    let alloy = |m: &mut Assets<StandardMaterial>, c: Color| {
        m.add(StandardMaterial {
            base_color: c,
            metallic: 0.85,
            perceptual_roughness: 0.35,
            ..default()
        })
    };
    let glow = |m: &mut Assets<StandardMaterial>, c: Color, k: f32| {
        m.add(StandardMaterial {
            base_color: c,
            emissive: c.to_linear() * k,
            perceptual_roughness: 0.2,
            ..default()
        })
    };
    commands.insert_resource(SettlementAssets {
        post: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(6)),
        round_wall: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(12)),
        // Domes, not cones and pyramids.
        cone_roof: meshes.add(Sphere::new(0.5).mesh().uv(24, 12)),
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        pyramid_roof: meshes.add(Sphere::new(0.5).mesh().uv(24, 12)),
        disc: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(16)),
        flame: meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap()),
        cloth: meshes.add(Cuboid::new(0.06, 1.0, 1.0)),
        wood: alloy(&mut materials, Color::srgb(0.22, 0.24, 0.28)),
        thatch: rough(&mut materials, Color::srgb(0.86, 0.88, 0.90)),
        stones: vec![
            rough(&mut materials, Color::srgb(0.90, 0.90, 0.88)), // pearl ceramic
            rough(&mut materials, Color::srgb(0.62, 0.80, 0.82)), // celadon
            rough(&mut materials, Color::srgb(0.70, 0.64, 0.86)), // lilac glaze
            alloy(&mut materials, Color::srgb(0.30, 0.32, 0.36)), // graphite
        ],
        slate: materials.add(StandardMaterial {
            base_color: Color::srgba(0.10, 0.14, 0.20, 0.92),
            emissive: LinearRgba::rgb(0.05, 0.25, 0.35),
            metallic: 0.6,
            perceptual_roughness: 0.08,
            ..default()
        }),
        // Cold light: the clans' lamps are plasma, not fire.
        flame_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.95, 1.0),
            emissive: LinearRgba::rgb(3.0, 9.0, 12.0),
            unlit: true,
            ..default()
        }),
        banners: vec![
            glow(&mut materials, Color::srgb(0.35, 0.95, 1.0), 3.0),
            glow(&mut materials, Color::srgb(0.90, 0.40, 1.0), 3.0),
            glow(&mut materials, Color::srgb(1.0, 0.72, 0.35), 2.5),
        ],
    });
}

/// Builds a settlement whose site centre falls in the chunk at `origin`.
/// Positions are chunk-local; `kids` are added to the chunk root.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    wet: &WetlandAssets,
    sa: &SettlementAssets,
    t: &Terrain,
    site: &Site,
    origin: Vec2,
    kids: &mut Vec<Entity>,
) {
    let mut rng = StdRng::seed_from_u64(site.seed ^ 0x5e77);
    let c = site.centre - origin;
    let centre = Vec3::new(c.x, site.height, c.y);
    let stone = match site.props {
        Props::Bones => sa.stones[1].clone(),
        Props::Crystal | Props::Peaks => sa.stones[2].clone(),
        Props::Monoliths => sa.stones[3].clone(),
        _ => sa.stones[0].clone(),
    };
    let root = commands
        .spawn((
            Name::new(format!("{:?} 0x{:x}", site.kind, site.seed)),
            Transform::from_translation(centre),
            Visibility::default(),
        ))
        .id();
    kids.push(root);
    let ground = |x: f32, z: f32| t.height(origin.x + c.x + x, origin.y + c.y + z) - site.height;

    let mut people: Vec<(Vec3, f32)> = Vec::new();
    match site.kind {
        SiteKind::City => {
            city(
                commands,
                meshes,
                materials,
                sa,
                site,
                root,
                &mut rng,
                &mut people,
            );
        }
        SiteKind::Stilts => {
            // Huts on a loose ring around a central platform, decks joined
            // by plank walkways.
            let deck = DECK;
            let n = rng.gen_range(6..10);
            let mut huts: Vec<Vec3> = Vec::new();
            for k in 0..n {
                let a = k as f32 / n as f32 * std::f32::consts::TAU + rng.gen_range(-0.25..0.25);
                let r = rng.gen_range(11.0..site.radius * 0.8);
                huts.push(Vec3::new(a.cos() * r, 0.0, a.sin() * r));
            }
            // The platform.
            commands.entity(root).with_children(|p| {
                p.spawn((
                    Mesh3d(sa.disc.clone()),
                    MeshMaterial3d(sa.wood.clone()),
                    Transform::from_xyz(0.0, deck, 0.0).with_scale(Vec3::new(6.0, 0.25, 6.0)),
                ));
                for k in 0..7 {
                    let a = k as f32 / 7.0 * std::f32::consts::TAU;
                    let (x, z) = (a.cos() * 5.0, a.sin() * 5.0);
                    let g = ground(x, z).min(SEA_LEVEL - site.height - 0.5);
                    p.spawn((
                        Mesh3d(sa.post.clone()),
                        MeshMaterial3d(sa.wood.clone()),
                        Transform::from_xyz(x, (g + deck) * 0.5, z).with_scale(Vec3::new(
                            0.18,
                            deck - g,
                            0.18,
                        )),
                    ));
                }
                // A fire in the middle of the platform.
                torch(p, sa, Vec3::new(0.0, deck + 0.12, 0.0), 0.9, 0.0);
            });
            people.push((Vec3::new(0.0, deck, 0.0), 4.5));
            for (k, h) in huts.iter().enumerate() {
                let hut_r = rng.gen_range(2.2..3.2);
                let yaw = rng.gen_range(0.0..std::f32::consts::TAU);
                let hut =
                    commands
                        .spawn((
                            Transform::from_translation(*h)
                                .with_rotation(Quat::from_rotation_y(yaw)),
                            Visibility::default(),
                            Swappable::sized(
                                ModelKind::Hut,
                                Vec3::new(
                                    (hut_r + 1.2) * 2.0,
                                    deck + 2.2 + hut_r * 0.9,
                                    (hut_r + 1.2) * 2.0,
                                ),
                            )
                            .variant(rng.r#gen()),
                            Obstacle {
                                radius: hut_r + 0.6,
                                below: 0.0,
                                above: deck + 2.2 + hut_r,
                            },
                        ))
                        .with_children(|p| {
                            for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                                let (x, z) = (sx * hut_r * 0.7, sz * hut_r * 0.7);
                                let g = ground(h.x + x, h.z + z).min(SEA_LEVEL - site.height - 0.5);
                                p.spawn((
                                    Mesh3d(sa.post.clone()),
                                    MeshMaterial3d(sa.wood.clone()),
                                    Transform::from_xyz(x, (g + deck) * 0.5, z)
                                        .with_scale(Vec3::new(0.16, deck - g, 0.16)),
                                ));
                            }
                            p.spawn((
                                Mesh3d(sa.disc.clone()),
                                MeshMaterial3d(sa.wood.clone()),
                                Transform::from_xyz(0.0, deck, 0.0).with_scale(Vec3::new(
                                    hut_r + 1.2,
                                    0.2,
                                    hut_r + 1.2,
                                )),
                            ));
                            p.spawn((
                                Mesh3d(sa.round_wall.clone()),
                                MeshMaterial3d(sa.thatch.clone()),
                                Transform::from_xyz(0.0, deck + 1.1, 0.0).with_scale(Vec3::new(
                                    hut_r * 0.8,
                                    2.2,
                                    hut_r * 0.8,
                                )),
                            ));
                            p.spawn((
                                Mesh3d(sa.cone_roof.clone()),
                                MeshMaterial3d(sa.thatch.clone()),
                                Transform::from_xyz(0.0, deck + 2.2 + hut_r * 0.45, 0.0)
                                    .with_scale(Vec3::new(hut_r + 0.4, hut_r * 0.9, hut_r + 0.4)),
                            ));
                        })
                        .id();
                commands.entity(root).add_child(hut);
                // Walkway to the platform, and to the next hut.
                let next = huts[(k + 1) % huts.len()];
                commands.entity(root).with_children(|p| {
                    plank(p, sa, Vec3::ZERO, *h, deck, 1.3);
                    if h.distance(next) < 34.0 {
                        plank(p, sa, *h, next, deck, 1.0);
                    }
                    if rng.gen_bool(0.5) {
                        torch(
                            p,
                            sa,
                            h + Vec3::new(0.0, deck, 0.0) + (h.normalize_or_zero() * (hut_r + 0.6)),
                            0.5,
                            k as f32,
                        );
                    }
                });
                people.push((h.with_y(deck), hut_r + 0.8));
            }
            // Canoes drawn up by the platform.
            commands.entity(root).with_children(|p| {
                for _ in 0..rng.gen_range(1..4) {
                    let a = rng.gen_range(0.0..std::f32::consts::TAU);
                    let r = rng.gen_range(7.0..12.0);
                    p.spawn((
                        Transform::from_xyz(
                            a.cos() * r,
                            SEA_LEVEL - site.height - 0.1,
                            a.sin() * r,
                        )
                        .with_rotation(Quat::from_rotation_y(rng.gen_range(0.0..6.0))),
                        Visibility::default(),
                        Swappable::sized(ModelKind::Canoe, Vec3::new(1.0, 0.7, 5.2))
                            .variant(rng.r#gen()),
                    ))
                    .with_children(|p| {
                        p.spawn((
                            Mesh3d(wet.canopy_mesh.clone()),
                            MeshMaterial3d(sa.wood.clone()),
                            Transform::from_xyz(0.0, 0.25, 0.0)
                                .with_scale(Vec3::new(0.5, 0.3, 2.6)),
                            NotShadowCaster,
                        ));
                    });
                }
            });
        }
        SiteKind::Town => {
            let r = site.radius;
            // Houses on rough rings around a square with a well and a tower.
            let n = rng.gen_range(10..17);
            commands.entity(root).with_children(|p| {
                for k in 0..n {
                    let ring = if k < n / 3 { 0.42 } else { 0.72 };
                    let a = k as f32 / n as f32 * std::f32::consts::TAU + rng.gen_range(-0.2..0.2);
                    let d = r * ring * rng.gen_range(0.85..1.1);
                    let at = Vec3::new(a.cos() * d, 0.0, a.sin() * d);
                    let (w, l, h) = (
                        rng.gen_range(4.0..7.0),
                        rng.gen_range(4.0..7.5),
                        rng.gen_range(3.0..5.0),
                    );
                    house(p, sa, &stone, at, -a, w, l, h, rng.gen_range(0..3));
                }
                // The tower.
                let tower_h = rng.gen_range(12.0..18.0);
                tower(p, sa, &stone, Vec3::ZERO, 3.0, tower_h);
                banner(p, sa, Vec3::new(0.0, tower_h + 3.6, 0.0), 4.0, 0);
                // The well, and a ring wall with gaps for the gates.
                p.spawn((
                    Mesh3d(sa.round_wall.clone()),
                    MeshMaterial3d(stone.clone()),
                    Transform::from_xyz(7.0, 0.5, 2.0).with_scale(Vec3::new(1.4, 1.0, 1.4)),
                ));
                let segs = 28;
                for k in 0..segs {
                    let a = k as f32 / segs as f32 * std::f32::consts::TAU;
                    if k % 14 == 0 {
                        // A gate: a gap in the wall with a torch either side.
                        for side in [-1.0, 1.0] {
                            let b = a + side * 1.2 / r;
                            torch(
                                p,
                                sa,
                                Vec3::new(
                                    b.cos() * (r * 0.92 - 1.5),
                                    0.0,
                                    b.sin() * (r * 0.92 - 1.5),
                                ),
                                0.6,
                                k as f32 + side,
                            );
                        }
                        continue;
                    }
                    let len = r * 0.92 * std::f32::consts::TAU / segs as f32 + 0.3;
                    p.spawn((
                        Mesh3d(sa.block.clone()),
                        MeshMaterial3d(stone.clone()),
                        Transform::from_xyz(a.cos() * r * 0.92, 1.4, a.sin() * r * 0.92)
                            .with_rotation(Quat::from_rotation_y(-a))
                            .with_scale(Vec3::new(1.0, 2.8, len)),
                    ));
                }
            });
            people.push((Vec3::ZERO, r * 0.8));
            people.push((Vec3::ZERO, r * 0.8));
        }
        SiteKind::Hold => {
            let r = site.radius;
            let n = rng.gen_range(3..6);
            commands.entity(root).with_children(|p| {
                let mut towers: Vec<Vec3> = Vec::new();
                for k in 0..n {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU + rng.gen_range(-0.3..0.3);
                    let d = r * rng.gen_range(0.5..0.85);
                    let at = Vec3::new(a.cos() * d, 0.0, a.sin() * d);
                    let (tr, th) = (rng.gen_range(2.4..4.0), rng.gen_range(10.0..22.0));
                    tower(p, sa, &stone, at, tr, th);
                    banner(p, sa, at + Vec3::Y * (th + tr * 1.2), 3.0, k);
                    towers.push(at);
                }
                // Walls between the towers; the great hall in the middle.
                for k in 0..towers.len() {
                    let (a, b) = (towers[k], towers[(k + 1) % towers.len()]);
                    let mid = (a + b) * 0.5;
                    let len = a.distance(b);
                    let yaw = (a.x - b.x).atan2(a.z - b.z);
                    p.spawn((
                        Mesh3d(sa.block.clone()),
                        MeshMaterial3d(stone.clone()),
                        Transform::from_translation(mid + Vec3::Y * 2.5)
                            .with_rotation(Quat::from_rotation_y(yaw))
                            .with_scale(Vec3::new(1.2, 5.0, len)),
                    ));
                }
                house(
                    p,
                    sa,
                    &stone,
                    Vec3::ZERO,
                    rng.gen_range(0.0..6.0),
                    10.0,
                    14.0,
                    6.0,
                    1,
                );
                for k in 0..3 {
                    let a = k as f32 * 2.1;
                    torch(
                        p,
                        sa,
                        Vec3::new(a.cos() * 9.0, 0.0, a.sin() * 9.0),
                        0.8,
                        k as f32,
                    );
                }
            });
            people.push((Vec3::ZERO, r * 0.7));
        }
    }

    // The people: a few per spot, wandering around it.
    let count = match site.kind {
        SiteKind::Stilts => rng.gen_range(3..6),
        SiteKind::Town => rng.gen_range(5..9),
        SiteKind::Hold => rng.gen_range(2..4),
        SiteKind::City => rng.gen_range(10..16),
    };
    for _ in 0..count {
        let (home, radius) = people[rng.gen_range(0..people.len())];
        let cloth = sa_cloth(&mut rng);
        let e = crate::components::spawn_villager(commands, meshes, materials, cloth, rng.r#gen());
        let a = rng.gen_range(0.0..std::f32::consts::TAU);
        let d = rng.gen_range(0.0..radius * 0.8);
        let at = home + Vec3::new(a.cos() * d, 0.0, a.sin() * d);
        commands.entity(e).insert((
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(a)),
            Villager {
                home,
                radius,
                target: at,
                pause: rng.gen_range(0.5..4.0),
                speed: rng.gen_range(1.0..1.8),
            },
            ChildOf(root),
        ));
    }
}

/// A city: an arcology at the heart, spires on a ring around it, ring roads
/// and avenues lit along their edges, trees on the avenues, craft on the
/// roads and in the air. The big shapes are alea forms (`forms.rs`): every
/// world draws its own.
#[allow(clippy::too_many_arguments)]
fn city(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    sa: &SettlementAssets,
    site: &Site,
    root: Entity,
    rng: &mut StdRng,
    people: &mut Vec<(Vec3, f32)>,
) {
    let r = site.radius;
    let road = materials.add(StandardMaterial {
        base_color: Color::srgb(0.10, 0.11, 0.13),
        perceptual_roughness: 0.6,
        metallic: 0.3,
        ..default()
    });
    let plaza = materials.add(StandardMaterial {
        base_color: Color::srgb(0.78, 0.80, 0.80),
        perceptual_roughness: 0.4,
        ..default()
    });
    let strip = materials.add(StandardMaterial {
        base_color: Color::srgb(0.4, 0.95, 1.0),
        emissive: LinearRgba::rgb(2.0, 7.0, 9.0),
        unlit: true,
        ..default()
    });
    let ring =
        |inner: f32, outer: f32| Mesh::from(Annulus::new(inner, outer).mesh().resolution(96));
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let roads = [r * 0.52, r * 0.88];
    let avenues = rng.gen_range(4..7);
    let turn = rng.gen_range(0.0..std::f32::consts::TAU);
    commands.entity(root).with_children(|p| {
        // The plaza the city stands on.
        p.spawn((
            Mesh3d(meshes.add(Circle::new(r * 0.97).mesh().resolution(96))),
            MeshMaterial3d(plaza.clone()),
            Transform::from_xyz(0.0, 0.04, 0.0).with_rotation(flat),
        ));
        // Ring roads with lit edges.
        for rr in roads {
            p.spawn((
                Mesh3d(meshes.add(ring(rr - 5.0, rr + 5.0))),
                MeshMaterial3d(road.clone()),
                Transform::from_xyz(0.0, 0.08, 0.0).with_rotation(flat),
            ));
            for edge in [rr - 5.0, rr + 5.0] {
                p.spawn((
                    Mesh3d(meshes.add(ring(edge - 0.18, edge + 0.18))),
                    MeshMaterial3d(strip.clone()),
                    Transform::from_xyz(0.0, 0.1, 0.0).with_rotation(flat),
                    NotShadowCaster,
                ));
            }
        }
        // Avenues from the heart to the edge.
        for k in 0..avenues {
            let a = turn + k as f32 / avenues as f32 * std::f32::consts::TAU;
            let (from, to) = (r * 0.3, r * 0.97);
            let mid = (from + to) * 0.5;
            let at = Vec3::new(a.cos() * mid, 0.07, a.sin() * mid);
            let yaw = Quat::from_rotation_y(-a);
            p.spawn((
                Mesh3d(sa.block.clone()),
                MeshMaterial3d(road.clone()),
                Transform::from_translation(at)
                    .with_rotation(yaw)
                    .with_scale(Vec3::new(to - from, 0.06, 8.0)),
            ));
            for side in [-4.2, 4.2] {
                p.spawn((
                    Mesh3d(sa.block.clone()),
                    MeshMaterial3d(strip.clone()),
                    Transform::from_translation(at + yaw * Vec3::new(0.0, 0.03, side))
                        .with_rotation(yaw)
                        .with_scale(Vec3::new(to - from, 0.05, 0.3)),
                    NotShadowCaster,
                ));
                // Trees along the avenue.
                for t in 0..4 {
                    let d = from + 14.0 + t as f32 * (to - from - 28.0) / 3.0;
                    let off = yaw * Vec3::new(0.0, 0.0, side * 2.2);
                    let s = rng.gen_range(0.7..1.1);
                    p.spawn((
                        Transform::from_translation(Vec3::new(a.cos() * d, 0.0, a.sin() * d) + off)
                            .with_scale(Vec3::splat(s)),
                        Visibility::default(),
                        FormSlot {
                            kind: FormKind::Tree,
                            pick: rng.r#gen(),
                            scale: s,
                            solid: false,
                        },
                    ));
                }
            }
        }
        // The heart: an arcology.
        p.spawn((
            Transform::from_xyz(0.0, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(rng.gen_range(0.0..6.0))),
            Visibility::default(),
            FormSlot {
                kind: FormKind::Arcology,
                pick: rng.r#gen(),
                scale: 1.0,
                solid: true,
            },
        ));
        // Spires between the roads, avoiding the avenues.
        let spires = rng.gen_range(6..12);
        for k in 0..spires {
            let a = turn
                + (k as f32 + 0.5) / spires as f32 * std::f32::consts::TAU
                + rng.gen_range(-0.12..0.12);
            let d = rng.gen_range(r * 0.64..r * 0.78);
            let s = rng.gen_range(0.6..1.2);
            p.spawn((
                Transform::from_xyz(a.cos() * d, 0.0, a.sin() * d)
                    .with_rotation(Quat::from_rotation_y(rng.gen_range(0.0..6.0)))
                    .with_scale(Vec3::splat(s)),
                Visibility::default(),
                FormSlot {
                    kind: FormKind::Tower,
                    pick: rng.r#gen(),
                    scale: s,
                    solid: true,
                },
            ));
        }
        // Craft on the ring roads, both ways…
        for (lane, rr) in roads.iter().enumerate() {
            for k in 0..rng.gen_range(5..10) {
                let dir = if (k + lane) % 2 == 0 { 1.0 } else { -1.0 };
                let lane_r = rr + 2.4 * dir;
                p.spawn((
                    Transform::default(),
                    Visibility::default(),
                    Traffic {
                        radius: lane_r,
                        speed: dir * rng.gen_range(14.0..24.0) / lane_r,
                        angle: rng.gen_range(0.0..std::f32::consts::TAU),
                        height: 0.9,
                        weave: 0.0,
                    },
                    FormSlot {
                        kind: FormKind::Vehicle,
                        pick: rng.r#gen(),
                        scale: 1.0,
                        solid: false,
                    },
                ));
            }
        }
        // …and in the air.
        for _ in 0..rng.gen_range(8..16) {
            let rr = rng.gen_range(r * 0.35..r * 1.3);
            p.spawn((
                Transform::default(),
                Visibility::default(),
                Traffic {
                    radius: rr,
                    speed: (if rng.gen_bool(0.5) { 1.0 } else { -1.0 }) * rng.gen_range(20.0..40.0)
                        / rr,
                    angle: rng.gen_range(0.0..std::f32::consts::TAU),
                    height: rng.gen_range(35.0..140.0),
                    weave: rng.gen_range(2.0..8.0),
                },
                FormSlot {
                    kind: FormKind::Vehicle,
                    pick: rng.r#gen(),
                    scale: rng.gen_range(1.0..1.6),
                    solid: false,
                },
            ));
        }
        // Lamps along the outer road.
        for k in 0..16 {
            let a = k as f32 / 16.0 * std::f32::consts::TAU;
            let d = roads[1] + 7.0;
            torch(
                p,
                sa,
                Vec3::new(a.cos() * d, 0.0, a.sin() * d),
                0.8,
                k as f32,
            );
        }
    });
    people.push((Vec3::ZERO, r * 0.5));
    people.push((Vec3::ZERO, r * 0.9));
}

/// Craft go round their circles, facing the way they go, banking in the air.
fn drive_traffic(time: Res<Time>, mut q: Query<(&mut Traffic, &mut Transform)>) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    for (mut c, mut tf) in &mut q {
        c.angle += c.speed * dt;
        let (s, co) = c.angle.sin_cos();
        let y = c.height + c.weave * (t * 0.3 + c.radius).sin();
        tf.translation = Vec3::new(co * c.radius, y, s * c.radius);
        // Tangent of the circle, in the direction of travel.
        let ahead = Vec3::new(-s, 0.0, co) * c.speed.signum();
        let yaw = (-ahead.x).atan2(-ahead.z);
        let bank = if c.weave > 0.0 {
            -0.35 * c.speed.signum()
        } else {
            0.0
        };
        tf.rotation = Quat::from_euler(EulerRot::YXZ, yaw, 0.0, bank);
    }
}

fn sa_cloth(rng: &mut StdRng) -> Color {
    let palette = [
        Color::srgb(0.62, 0.30, 0.22),
        Color::srgb(0.30, 0.42, 0.48),
        Color::srgb(0.72, 0.62, 0.30),
        Color::srgb(0.38, 0.48, 0.28),
        Color::srgb(0.55, 0.40, 0.58),
    ];
    palette[rng.gen_range(0..palette.len())]
}

fn plank(
    p: &mut ChildSpawnerCommands,
    sa: &SettlementAssets,
    a: Vec3,
    b: Vec3,
    y: f32,
    width: f32,
) {
    let mid = (a + b) * 0.5;
    let len = a.distance(b);
    let yaw = (a.x - b.x).atan2(a.z - b.z);
    p.spawn((
        Mesh3d(sa.block.clone()),
        MeshMaterial3d(sa.wood.clone()),
        Transform::from_translation(mid.with_y(y - 0.05))
            .with_rotation(Quat::from_rotation_y(yaw))
            .with_scale(Vec3::new(width, 0.12, len)),
    ));
}

/// A model variant picked from where a thing stands (no RNG needed).
fn variant_at(at: Vec3) -> u32 {
    (at.x * 73.1 + at.z * 31.7 + at.y * 7.3).abs() as u32
}

fn torch(p: &mut ChildSpawnerCommands, sa: &SettlementAssets, at: Vec3, size: f32, phase: f32) {
    p.spawn((
        Transform::from_translation(at),
        Visibility::default(),
        Swappable::sized(ModelKind::Torch, Vec3::new(0.7, 2.6, 0.7)).variant(variant_at(at)),
    ))
    .with_children(|p| {
        p.spawn((
            Mesh3d(sa.post.clone()),
            MeshMaterial3d(sa.wood.clone()),
            Transform::from_xyz(0.0, 1.0, 0.0).with_scale(Vec3::new(0.12, 2.0, 0.12)),
        ));
        p.spawn((
            Torch {
                phase,
                size: size * 0.4,
            },
            Mesh3d(sa.flame.clone()),
            MeshMaterial3d(sa.flame_mat.clone()),
            Transform::from_xyz(0.0, 2.2, 0.0).with_scale(Vec3::splat(size * 0.4)),
            NotShadowCaster,
        ));
    });
}

/// A round tower with a conical roof, `r` wide and `h` tall to the eaves.
fn tower(
    p: &mut ChildSpawnerCommands,
    sa: &SettlementAssets,
    stone: &Handle<StandardMaterial>,
    at: Vec3,
    r: f32,
    h: f32,
) {
    p.spawn((
        Transform::from_translation(at),
        Visibility::default(),
        Swappable::sized(
            ModelKind::Tower,
            Vec3::new(r * 2.0 + 1.2, h + r * 1.2, r * 2.0 + 1.2),
        )
        .variant(variant_at(at)),
        Obstacle {
            radius: r + 0.6,
            below: 0.0,
            above: h + r * 1.2,
        },
    ))
    .with_children(|p| {
        p.spawn((
            Mesh3d(sa.round_wall.clone()),
            MeshMaterial3d(stone.clone()),
            Transform::from_xyz(0.0, h * 0.5, 0.0).with_scale(Vec3::new(r, h, r)),
        ));
        p.spawn((
            Mesh3d(sa.cone_roof.clone()),
            MeshMaterial3d(sa.slate.clone()),
            Transform::from_xyz(0.0, h + r * 0.6, 0.0).with_scale(Vec3::new(
                r + 0.6,
                r * 1.2,
                r + 0.6,
            )),
        ));
    });
}

fn banner(p: &mut ChildSpawnerCommands, sa: &SettlementAssets, at: Vec3, height: f32, k: usize) {
    p.spawn((
        Mesh3d(sa.post.clone()),
        MeshMaterial3d(sa.wood.clone()),
        Transform::from_translation(at + Vec3::Y * height * 0.5)
            .with_scale(Vec3::new(0.08, height, 0.08)),
    ));
    p.spawn((
        Banner(k as f32 * 1.7),
        Mesh3d(sa.cloth.clone()),
        MeshMaterial3d(sa.banners[k % sa.banners.len()].clone()),
        Transform::from_translation(at + Vec3::Y * (height - 0.9))
            .with_scale(Vec3::new(1.0, 1.6, 2.4)),
        NotShadowCaster,
    ));
}

fn house(
    p: &mut ChildSpawnerCommands,
    sa: &SettlementAssets,
    stone: &Handle<StandardMaterial>,
    at: Vec3,
    yaw: f32,
    w: f32,
    l: f32,
    h: f32,
    roof: usize,
) {
    let roof_mat = if roof == 0 {
        sa.thatch.clone()
    } else {
        sa.slate.clone()
    };
    let roof_h = w.min(l) * 0.6;
    p.spawn((
        Transform::from_translation(at).with_rotation(Quat::from_rotation_y(yaw)),
        Visibility::default(),
        Swappable::sized(ModelKind::House, Vec3::new(w, h + roof_h, l)).variant(variant_at(at)),
        Obstacle {
            radius: w.max(l) * 0.6,
            below: 0.0,
            above: h + roof_h,
        },
    ))
    .with_children(|p| {
        p.spawn((
            Mesh3d(sa.block.clone()),
            MeshMaterial3d(stone.clone()),
            Transform::from_xyz(0.0, h * 0.5, 0.0).with_scale(Vec3::new(w, h, l)),
        ));
        p.spawn((
            Mesh3d(sa.pyramid_roof.clone()),
            MeshMaterial3d(roof_mat),
            Transform::from_xyz(0.0, h + roof_h * 0.5, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4))
                .with_scale(Vec3::new(w * 0.78, roof_h, l * 0.78)),
        ));
    });
}

/// Villagers stroll between spots around home and stand a while.
fn wander(time: Res<Time>, mut q: Query<(&mut Villager, &mut Transform)>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let t = time.elapsed_secs();
    for (mut v, mut tf) in &mut q {
        let to = (v.target - tf.translation).with_y(0.0);
        if to.length() < 0.3 {
            v.pause -= dt;
            if v.pause <= 0.0 {
                // Somewhere else near home; the pick is a hash of time so it
                // needs no RNG component.
                let a = (t * 1.31 + tf.translation.x * 0.7 + tf.translation.z * 1.3).sin()
                    * std::f32::consts::PI;
                let d = v.radius * (0.2 + 0.7 * ((t * 0.77 + tf.translation.x).cos() * 0.5 + 0.5));
                v.target = v.home + Vec3::new(a.cos() * d, 0.0, a.sin() * d);
                v.pause = 1.0 + 4.0 * ((t * 0.37).sin() * 0.5 + 0.5);
            }
            continue;
        }
        let dir = to.normalize();
        let step = (v.speed * dt).min(to.length());
        tf.translation += dir * step;
        let yaw = (-dir.x).atan2(-dir.z);
        tf.rotation = tf
            .rotation
            .slerp(Quat::from_rotation_y(yaw), 1.0 - (-6.0 * dt).exp());
    }
}

fn flicker(
    time: Res<Time>,
    sa: Res<SettlementAssets>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut torches: Query<(&Torch, &mut Transform)>,
    mut banners: Query<(&Banner, &mut Transform), Without<Torch>>,
) {
    let t = time.elapsed_secs();
    for (torch, mut tf) in &mut torches {
        // Lamps breathe and the orb hovers a little over its pylon.
        let f = 1.0 + 0.12 * (t * 2.0 + torch.phase).sin();
        let s = torch.size * f;
        tf.scale = Vec3::splat(s);
        tf.translation.y = 2.3 + 0.08 * (t * 1.3 + torch.phase).sin();
    }
    if let Some(m) = mats.get_mut(&sa.flame_mat) {
        // A slow electric pulse rather than a flicker.
        let k = 1.0 + 0.25 * (t * 2.3).sin() + 0.05 * (t * 31.0).sin();
        m.emissive = LinearRgba::rgb(3.0 * k, 9.0 * k, 12.0 * k);
    }
    for (b, mut tf) in &mut banners {
        let a = (t * 6.0 + b.0).sin() * 0.3 + (t * 11.0 + b.0 * 2.0).sin() * 0.1;
        tf.rotation = Quat::from_rotation_y(a) * Quat::from_rotation_x(0.1 * (t * 4.0 + b.0).sin());
    }
}

pub struct SettlementPlugin;

impl Plugin for SettlementPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .run_system_once(setup_assets)
            .expect("settlement assets need Assets<Mesh> and Assets<StandardMaterial>");
        app.add_systems(
            Update,
            (wander, flicker, drive_traffic).in_set(crate::GameSet::Simulation),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn villagers_have_a_gait() {
        // The figure spawned for a villager carries the rig that animates it.
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>();
        let e = app
            .world_mut()
            .run_system_once(
                |mut commands: Commands,
                 mut meshes: ResMut<Assets<Mesh>>,
                 mut materials: ResMut<Assets<StandardMaterial>>| {
                    crate::components::spawn_villager(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        Color::WHITE,
                        0,
                    )
                },
            )
            .unwrap();
        assert!(app.world().get::<crate::rig::Gait>(e).is_some());
        assert!(app.world().get::<crate::rig::Rig>(e).is_some());
    }
}
