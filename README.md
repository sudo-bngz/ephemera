# EPHEMERA

*An arcade racer on the back of a giant mosquito.*

Judith was raised by the marsh tribe but born of no clan. The tribe does not name a
child until she has ridden the wind — so she has one season to find a wild
**Ephemera**, make it hers, and race it at the Gathering.

Built in Rust on [Bevy 0.17](https://bevyengine.org). No external assets: the
creature and the rider are assembled from primitives, and the **endless world is drawn
and scored at runtime by [alea](https://github.com/sudo-bngz/alea)**, a language for
generative art. Every land is an edition of an alea work — its relief, its ground, its
ambience — and so is every monolith, orb, veil and sigil, picture and hum alike.

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
| Sound | `M` mute · `--mute` start muted |
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

Sound needs nothing extra on macOS. On Linux, Bevy's audio needs ALSA's headers
(`libasound2-dev`).

To check or test without a GPU (CI, SSH box):

```sh
cargo clippy
cargo test   # headless: every phase, streaming, alea pictures and sounds, terrain, models
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
├── terrain.rs     The world's shape: regions of alea editions, blended; climate; flattening
├── world.rs       Chunk streaming on the compute pool; ground meshes + painted textures, props, landmarks
├── art.rs         alea pictures: editions rendered off-thread, cached, animated
├── audio.rs       alea sound: ambience by region, landmark hums in space, wings, music, chimes
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

assets/alea/       the alea works: land/ (biomes), sound/ (music, wings, chimes), landmarks
assets/models/     optional GLB models + meshy prompts
tools/meshy-gen/   generates those models with the Meshy API
```

### The endless world, drawn by alea

The world is a sea of 512 m regions. Each region is an edition of one of the land
works in [`assets/alea/land/`](assets/alea/land/) (marsh, fungal bloom, bone dunes,
crystal flats, field of monoliths), chosen by a climate map so similar lands cluster.
The game reads a land work's first `render image` as the **relief** (the picture is
the land: black is the lake bottom, 30 % grey the shoreline, white the peaks), its
second as the **ground painting**, and its `render sound` as the region's
**ambience**. Relief and painting are blended between neighbouring regions by
distance, so editions meet without seams.

The ground streams in 128 m chunks around whoever the camera follows: each chunk's
heights and its 96 × 96 painted texture are sampled from the alea programs on the
compute pool (~70 ms a chunk, six in flight), nearest first, dropped behind. Lakes
form wherever the relief dips below the water line; flight, AI riders and gates follow
the ground. Everything is a function of the seed and a position, so the world is
seamless, endless and reproducible, and `N` draws a new one.

Drop another `.alea` file into `assets/alea/land/` and it becomes a land of the world.

### alea in the game: pictures

The landmarks are painted at runtime by four works in `assets/alea/`:

| Work | Becomes |
|---|---|
| `monolith.alea` | the carved, glowing faces of obelisks floating over the marsh (animated) |
| `veil.alea` | aurora curtains hanging in the sky (animated, additive) |
| `sigil.alea` | circles of light in the mud that nobody in the tribe admits to drawing |
| `orb.alea` | the marbled, veined skin of spheres drifting through the air (animated) |

Each landmark shows an edition: the work, and a seed derived from the world seed and
where it stands. Editions render on background threads, are cached on disk, and are
animated by swapping frames on a material shared by every landmark showing that
edition.

### alea in the game: sound

There is no audio file in the repository. Every loop and chime is a `render sound`,
rendered by alea when first needed (about a second for twenty seconds of stereo) and
cached as WAV:

- each region's **ambience**, crossfaded by distance as you fly from land to land;
- each landmark's **hum** — the same work that paints it — placed in space on it;
- the Ephemera's **wings** (`sound/wings.alea`), a loop pitched up with speed;
- **music** for the title, the camp and the race (`sound/title|camp|race.alea`),
  a new edition per world;
- the **chimes** of the rite (`sound/events.alea`, one render per event).

[`assets/alea/README.md`](assets/alea/README.md) has the conventions, and the two
things alea does to every sound that a game has to work around.

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

API keys need a paid Meshy plan. Without one, the same command prints a step-by-step
guide to make each model by hand in Meshy's web app (the free plan works): the prompt
to paste, the triangle count and the file name to save as.

Prompts live in `assets/models/meshy-prompts.txt`. Each model is a preview task
(geometry, remeshed to its triangle budget) then a refine task (PBR textures),
downloaded as GLB. Models run in parallel; task ids are recorded so an interrupted
run resumes without paying twice, and a task Meshy fails is started over on the next
run.

## Roadmap

- Gamepad support — `InputAxes` is already the single point to extend.
- Rigged wings for a custom Ephemera model.
- Audio ducking and a mixer; HRTF for the hums.
- Ghost replay of your best lap; tribe leaderboard.
- A fourth phase: the naming ceremony.

## License

MIT
