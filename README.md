# EPHEMERA

*An arcade racer on the back of a giant mosquito.*

Judith was raised by the marsh tribe but born of no clan. The tribe does not name a
child until she has ridden the wind — so she has one season to find a wild
**Ephemera**, make it hers, and race it at the Gathering.

Built in Rust on [Bevy 0.17](https://bevyengine.org). No external assets: the
creature, the rider and an **endless procedural world** are assembled at runtime, and
its landmarks are painted live by [alea](https://github.com/sudo-bngz/alea), a
language for generative art: every monolith, orb, veil and sigil is an edition of its
own.

## The three phases

| Phase | What you do | What it tests |
|---|---|---|
| **I. The Stalk** (stealth) | Creep through a dusk marsh towards a resting Ephemera. Sound (sprinting, splashing through pools) and sight (standing in the open) raise its *awareness*; crouching still in the reeds lets it bleed off. Get in net range while it's calm. | Patience, reading cover |
| **II. The Tether** (resource management) | Seven dawns at camp. Each dawn: forage nectar, feed & groom, flight training, or rest. Juggle the nectar pouch, Judith's energy, and the creature's hunger and trust. Starve it or lose its trust and it snaps the tether — back to the stalk. | Planning, trade-offs |
| **III. The Gathering** (racing) | Three laps through gates against three tribal riders. Low chase camera, FOV that widens with speed, banking, boost, skim bonuses over water and reed beds that stop you dead. The stats you built at the tether are what you fly with. | Speed, reflexes |

Low trust at race time means the creature *bucks* — short bursts where it takes the
controls away from you.

### Explore: the endless marsh

`F4` (or `--explore`) is free flight through a world with no edge. Every visit draws a
new world: its terrain, its light and every landmark in it, the way an alea work
draws a new edition at every run. `N` draws another; `--seed 0x2a` brings one back.
The HUD names the biome below, the edition of the nearest landmark, and how many
editions you've found.

## Controls

| | |
|---|---|
| On foot | `W/S` walk · `A/D` turn · `C` crouch · `Shift` sprint · `Space` throw net |
| Camp | `1`–`4` pick what to do with the dawn · `Enter` ride to the Gathering |
| Flight | `W/S` throttle · `A/D` bank · `Q` dive / `E` climb · `Shift` boost |
| Anywhere | `Enter` confirm · `Esc` back to title / quit |
| Explore | flight controls · `N` new world |
| Shortcuts | `F1` stalk · `F2` tether · `F3` race (with a pre-tamed mount) · `F4` explore |

Jump straight to a phase from the command line:

```sh
cargo run --release -- --race                  # or --capture, --taming
cargo run --release -- --explore               # a new world
cargo run --release -- --explore --seed 0x2a   # that world again
```

## Running on macOS

Requires a Rust toolchain (`rustup`, stable). No other system dependencies.

```sh
git clone https://github.com/sudo-bngz/ephemera
cd ephemera
cargo run --release
```

The first build takes a few minutes (it's Bevy). For fast iteration while hacking on
the game, dynamic linking cuts rebuilds to a couple of seconds:

```sh
cargo run --features dev
```

Debug builds keep dependencies optimised (`profile.dev.package."*"` is `opt-level = 3`),
so even `cargo run` without `--release` is playable.

To check or test without a GPU (CI, SSH box):

```sh
cargo clippy
cargo test   # headless: every phase, streaming, alea rendering, terrain, models
```

## Architecture

The whole game is one `GamePlugin` on top of `DefaultPlugins`. Every phase is a
`GamePhase` state; a phase spawns its own scene on `OnEnter`, tags everything
`PhaseScoped`, and `OnExit` despawns it. The only things that cross phases are two
resources: `EphemeraStats` (what taming produced, what racing consumes) and `RunLog`.

```
src/
├── main.rs        App, GameSet ordering, headless smoke test
├── state.rs       GamePhase, EphemeraStats, RunLog, PhaseScoped cleanup
├── input.rs       InputAxes — one sampled input resource for all phases
├── components.rs  Shared markers (Judith, Ephemera, Velocity, CameraTarget)
├── terrain.rs     The world's shape: seeded noise, five biomes, lakes (pure, unit-tested)
├── world.rs       Chunk streaming; terrain meshes, water, reeds, biome props, landmarks
├── art.rs         alea at runtime: editions rendered off-thread, cached, animated
├── sky.rs         Sky dome, sun, clouds, distant mesas, per-mood fog palette
├── models.rs      Optional GLB overrides (meshy.ai etc.) swapped in for placeholders
├── creature.rs    The Ephemera built from primitives; wing/leg/body animation
├── camera.rs      Chase camera: lag, speed-FOV, roll, shake
├── hud.rs         One persistent HUD; phases write into the `Hud` resource
├── title.rs       Title screen
├── capture.rs     Phase I  — awareness model, cover, net throw, spook & flee
├── taming.rs      Phase II — dawn loop, four actions, escape condition
├── racing.rs      Phase III — flight model, track spline, gates, AI, speed lines, results
└── explore.rs     Free flight through an endless world

assets/alea/       the alea works that paint the landmarks
assets/models/     optional GLB models + meshy prompts
tools/meshy-gen/   generates those models with the Meshy API
```

### The endless world

The world is one seeded height function (`terrain.rs`) cut into 128 m chunks that
stream around whoever the camera follows (`world.rs`): nearest first, two per frame,
dropped once they're out of range. Every chunk is a pure function of the seed and
its coordinates, so the world is seamless, endless and reproducible.

- **Biomes** blend into each other: the *reed marsh*, the *fungal bloom* (glowing
  mushrooms to fly under), the *bone dunes* (rib arches to thread), the *crystal
  flats* (a terraced plateau of shards) and, rarely, the *field of monoliths*.
- **Lakes** form wherever the terrain dips below the water line; fly low over one
  for a speed push.
- **Flight is terrain-relative**: altitude limits, AI riders and gates follow the
  ground. The race keeps its loop, and its line is kept clear of random obstacles.
- A chunk builds in under a millisecond; reed clumps are single merged meshes shared
  between clumps, so they batch on the GPU.

### alea in the game

The landmarks are painted at runtime by four works in [`assets/alea/`](assets/alea/),
through the alea library (pinned in `Cargo.toml`):

| Work | Becomes |
|---|---|
| `monolith.alea` | the carved, glowing faces of obelisks floating over the marsh (animated) |
| `veil.alea` | aurora curtains hanging in the sky (animated, additive) |
| `sigil.alea` | circles of light in the mud that nobody in the tribe admits to drawing |
| `orb.alea` | the marbled, veined skin of spheres drifting through the air (animated) |

Each landmark shows an edition: the work, and a seed derived from the world seed and
where it stands. The same world always shows the same editions. Editions render on
background threads, are cached on disk (`~/Library/Caches/ephemera/alea` on macOS),
and are animated by swapping frames on a material shared by every landmark showing
that edition. Edit a work, run again, and the world repaints. A work with an error
is logged and its landmarks stay dark: the game doesn't stop for art.

### Per-frame ordering

`GameSet::Input → Simulation → Camera → Hud`, chained in `Update`. Input is sampled
once, the simulation integrates from it, the camera follows the *result* of the
simulation, and the HUD reads the final state — so what you see is never a frame stale.
That ordering is most of what makes the controls feel responsive.

### The flight model

Player and AI riders share one `Rider` component and one `integrate` system; the only
difference is who fills in their `FlightInput` (the player system copies `InputAxes`,
the AI system steers along the track spline with a look-ahead point and rubber-banding).
Tuning the feel happens in exactly one place, and the rivals obey the same physics.

- Longitudinal: thrust minus quadratic drag tuned so terminal velocity equals the
  creature's top speed; boost multiplies both.
- Yaw rate tightens at low speed and opens at high speed; visual roll follows steering.
- Altitude is soft-clamped; dragging through the mud bleeds speed; skimming low over a
  pool gives a small push.
- Short reeds are soft: you plough through with a little drag and they whip aside.
- Tall reeds are solid but never a wall: you slide along them, lose speed once per
  impact (more if head-on) and flutter upwards to hop over. Pushes from overlapping
  clumps are summed, so two clumps can never pin you between them.

### Speed sensation

- Chase camera sits lower and closer as speed rises; FOV widens by up to 30°.
- Camera rolls with the bank, shakes on boost, gate pass, collisions and bucking.
- Speed-line streaks stream past the camera above ~55% of top speed.
- Reed beds line the racing line so the ground visibly rushes by at low altitude, and
  bend away as you tear past.
- Fog tied to a gradient sky dome: distance fades into the horizon colour, so speed
  reads against a world with depth instead of a flat backdrop.
- The creature's wing-beat frequency and amplitude scale with speed.

## Custom 3D models (meshy.ai)

Everything is built from primitives, but any of the creature, Judith, reeds, gates and
horizon rocks can be replaced by dropping a GLB into `assets/models/`. No code change
needed. See [`assets/models/README.md`](assets/models/README.md) for file names, poly
budgets and how to fit a model with `models.cfg`.

To generate them all with the [Meshy API](https://www.meshy.ai/developers):

```sh
export MESHY_API_KEY=msy_...
cargo run --release --manifest-path tools/meshy-gen/Cargo.toml              # all missing
cargo run --release --manifest-path tools/meshy-gen/Cargo.toml -- ephemera  # just one
cargo run --release --manifest-path tools/meshy-gen/Cargo.toml -- --dry-run # show prompts
```

Prompts live in `assets/models/meshy-prompts.txt`. Each model is a preview task
(geometry, remeshed to its triangle budget) then a refine task (PBR textures),
downloaded as GLB. Models run in parallel; task ids are recorded so an interrupted
run resumes without paying twice, and a task Meshy fails is started over on the next
run.

## Roadmap

- Audio (`bevy_audio`): wing drone pitched by speed, gate chimes, marsh ambience.
- Gamepad support — `InputAxes` is already the single point to extend.
- Rigged wings for a custom Ephemera model.
- alea's `render sound` as the world's soundtrack: each landmark hums its edition.
- Ghost replay of your best lap; tribe leaderboard.
- A fourth phase: the naming ceremony.

## License

MIT
