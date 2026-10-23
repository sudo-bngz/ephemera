# EPHEMERA

*Science fiction on the back of a giant mosquito.*

On Khera, a marsh moon under a violet gas giant, the settlers live in clans — an old
way of living, kept by choice — in spires of grown ceramic, among machines older than
their names. A child of the clans is not named until she has ridden an **Ephemera**,
the moon's wild flyers. Judith was found as an infant at the foot of the veils, the
auroras that hum over the marsh; raised by the clans, born of none. This season she
will stalk one, bond with it, and race it at the Gathering.

The look is science fiction, not fantasy: the clans' organisation is tribal, nothing
they make is. See [`assets/models/meshy-prompts.txt`](assets/models/meshy-prompts.txt)
for the art direction.

**Starting.** The game opens on a menu — *Begin the rite*, *Into the veils*, *Race*,
*Controls*, *Quit* — over the title world. A journey opens with a few story cards
(ENTER for the next, ESC to skip), and every phase starts with a briefing — what to
do, and the keys for it — that holds the game until ENTER. `ESC` pauses (resume,
controls, restart, menu, quit) and `TAB` shows the controls anywhere.

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

### Explore: the endless world

`F4` (or `--explore`) is free flight through a world with no edge. Every visit draws a
new world: its terrain, its light and every landmark in it, the way an alea work
draws a new edition at every run. `N` draws another; `--seed 0x2a` brings one back.
The HUD names the land below, the edition of the nearest landmark, and how many
editions you've found.

The world is lived in. Seven lands — marsh, fungal bloom, bone dunes, crystal flats, a
field of monoliths, wooded groves and a **mountain range** that climbs above the fog —
with **rivers** cut across all of them. The clans have built **stilt habitats** in the
shallows, **ceramic towns** on flattened ground and **holds** of spires on the mountain
shoulders, with people walking between the domes under plasma lamps. Herds of
**grazers** drift over the grass, **waders** stalk the pools and take off when you
come close, flocks of **kites** ride the air over the high lands, and **leapers**
break the surface of the lakes.

**The maze of realities.** Every world is an edition of the same work, and the
veils — the aurora curtains — are the ways between editions. A reality is a seed
and a *depth*; its veils are sealed until the reality has been *read* (three of
its editions found), then flying through one shifts the world: the next reality
is drawn in the background for the ground you stand on, and over four seconds
the relief morphs from one into the other, the old world's props dissolve, the
fog closes in in the veil's colour, the sky turns, and the new world grows in
around you. One veil in three leads back up towards the reality you came from;
the rest lead deeper, and depth leaks into everything — the land works get it
as a parameter (`work Marsh(depth = 0)`) and drift, the sky's hues turn, the fog
thickens. The HUD shows the reality, its depth and whether its veils are open.
(This is the mechanism; the story that drives it is still being written.)

**On foot.** Press `F` in flight and the Ephemera comes down; Judith slides off its
back and the camera settles over her shoulder. She walks, runs, jumps, wades and swims
through the world at her own height — under the mushrooms, along the walkways, into
the towns — with `WASD` relative to the camera and the mouse (or the arrows) turning
it. The creature waits where you left it, or comes when whistled: `F` far from it
calls it over, `F` beside it puts her back in the saddle.

## Controls

| | |
|---|---|
| Stalk | `W/S` walk · `A/D` turn · `C` crouch · `Shift` sprint · `Space` throw net |
| Camp | `1`–`4` pick what to do with the dawn · `Enter` ride to the Gathering |
| Flight | `W/S` throttle · `A/D` bank · `Q` dive / `E` climb · `Shift` boost |
| Anywhere | `Enter` confirm · `Esc` pause menu · `Tab` controls · menus: `↑/↓` + `Enter` or the mouse |
| Explore, flying | flight controls · `F` land and dismount · `N` new world |
| Explore, on foot | `WASD` walk (camera-relative) · mouse / arrows look · `Shift` run · `Space` jump · `C` crouch · `F` mount (beside the Ephemera) or whistle it over |
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
├── terrain.rs     The world's shape: regions of alea editions, blended; climate; rivers; settlement sites
├── world.rs       Chunk streaming on the compute pool; ground meshes + painted textures, props, landmarks
├── settlement.rs  Stilt villages, walled towns, mountain holds; villagers, torches, banners
├── fauna.rs       Waders, grazers, kites, leapers: jointed animals that roam and flee
├── art.rs         alea pictures: editions rendered off-thread, cached, animated
├── audio.rs       alea sound: ambience by region, landmark hums in space, wings, music, chimes
├── sky.rs         Sky dome, sun, clouds, distant mesas, per-mood fog palette
├── models.rs      Optional GLB overrides (meshy.ai etc.) swapped in for placeholders
├── creature.rs    The Ephemera built from primitives; wing/leg/body animation
├── rig.rs         Rigs static models at load (wings, humanoid); poses; walk/crouch/ride
├── camera.rs      Chase camera: lag, speed-FOV, roll, shake; orbit camera on foot
├── hud.rs         One persistent HUD; phases write into the `Hud` resource
├── title.rs       Title screen
├── capture.rs     Phase I  — awareness model, cover, net throw, spook & flee
├── taming.rs      Phase II — dawn loop, four actions, escape condition
├── racing.rs      Phase III — flight model, track spline, gates, AI, speed lines, results
├── explore.rs     Free flight through an endless world
├── reality.rs     The maze of realities: portals in the veils, the shift that morphs one world into the next
└── onfoot.rs      Landing, dismounting, the third-person walker, whistling, remounting

assets/alea/       the alea works: land/ (biomes), sound/ (music, wings, chimes), landmarks
assets/models/     optional GLB models + meshy prompts
tools/meshy-gen/   generates those models with the Meshy API
```

### The endless world, drawn by alea

The world is a sea of 512 m regions. Each region is an edition of one of the land
works in [`assets/alea/land/`](assets/alea/land/) (marsh, fungal bloom, bone dunes,
crystal flats, field of monoliths, grove, peaks), chosen by a climate map so similar
lands cluster. The game reads a land work's first `render image` as the **relief**
(the picture is the land: black is the lake bottom, 30 % grey the shoreline, white
the peaks; the render's `range` says how high white is, so `peaks.alea` with
`range: [0, 5]` rises to 280 m), its second as the **ground painting**, and its
`render sound` as the region's **ambience**. Relief and painting are blended between
neighbouring regions by distance, so editions meet without seams.

Two things run over the lands without regard for their borders. **Rivers** follow the
zero lines of one slow noise across the whole world, carved into whatever relief they
cross: a reed-lined channel in the lowlands, a gorge in the hills. **Settlement sites**
are drawn on a 448 m grid from the seed; the tribe flattens the ground for a town or a
hold and drives stilts where the land is wet, and the chunks around build the houses,
the walkways, the torches and the people.

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

### Animation

Everything moves. The Ephemera beats its wings in a figure-eight, tucks its
legs at speed and bobs. Judith walks, creeps when crouched, rides astride and
leans into the bank. The rivals carry riders of their own. The procedural
placeholders have jointed limbs, and models loaded from `assets/models/` are
**rigged by the game at load** (`rig.rs`): it reads the mesh's shape, places a
skeleton, weights each vertex to the bones by position, and skins the mesh on
the GPU. One pose system drives both. Around them, wind gusts roll across the
reed beds, the water moves in waves that repeat exactly every chunk, mushroom
caps breathe, gills and crystals throb, spores wander, moths circle and gate
flags snap.

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
cargo run --release --manifest-path tools/meshy-gen/Cargo.toml -- judith    # rig + animate an existing judith.glb
```

API keys need a paid Meshy plan. Without one, the same command prints a step-by-step
guide to make each model by hand in Meshy's web app (the free plan works): the prompt
to paste, the triangle count and the file name to save as.

Prompts live in `assets/models/meshy-prompts.txt`. Each model is a preview task
(geometry, remeshed to its triangle budget) then a refine task (PBR textures),
downloaded as GLB. Models run in parallel; task ids are recorded so an interrupted
run resumes without paying twice, and a task Meshy fails is started over on the next
run.

Judith's section has an `animate =` line: after her model, the tool rigs her with
Meshy's rigging API and fetches her actions (idle, walking, running, crouch, jump,
swimming, sitting) from Meshy's animation library, so `judith.glb` arrives with a
skeleton and a clip for each, which the game plays and cross-fades. Meshy only rigs
humanoids; the Ephemera is rigged by the game itself at load (`rig.rs`).

## Roadmap

- Gamepad support — `InputAxes` is already the single point to extend.
- Audio ducking and a mixer; HRTF for the hums.
- Ghost replay of your best lap; clan leaderboard.
- A fourth phase: the naming ceremony.

## License

MIT
