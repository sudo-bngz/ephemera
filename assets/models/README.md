# Custom models (meshy.ai or any glTF source)

Drop a `.glb` file here with one of the names below and the game uses it on
the next run. No code change, no rebuild. A missing file keeps the built-in
placeholder, so you can swap models one at a time.

Every kind of thing in the world has a **slot**. The first seven are fitted
by hand (below); everything else is **auto-fitted**: the model's bounding
box is scaled to the size the world spawns it at and its bottom put on the
ground, so any export fits.

| Slot (file)     | Replaces                                  | Fitted to                     |
|-----------------|-------------------------------------------|-------------------------------|
| `ephemera.glb`  | the creature (player + 3 rivals)          | hand: ~7 m long, faces -Z     |
| `judith.glb`    | Judith on foot and in the saddle          | hand: ~1.9 m tall             |
| `villager.glb`  | the clans: villagers, the rivals' riders  | hand: ~1.9 m tall             |
| `reed_tall.glb` | tall, *solid* reed clumps (obstacles)     | unit: 1 m radius × 1 m        |
| `reed.glb`      | short reed clumps (soft, fly-through)     | unit: 1 m radius × 1 m        |
| `gate.glb`      | race gates                                | unit: 1 m tall                |
| `mesa.glb`      | distant rock formations on the horizon    | unit: 1 m                     |
| `tree.glb`      | bulb trees (groves, lower peaks)          | auto: 7–16 m tall             |
| `mushroom.glb`  | the giant mushrooms of the fungal land    | auto: 9–25 m tall             |
| `boulder.glb`   | boulders and scree                        | auto: 2–10 m                  |
| `spire.glb`     | rock needles on the peaks                 | auto: 9–26 m tall             |
| `crystal.glb`   | crystal shard clusters                    | auto: 5–15 m tall             |
| `rib.glb`       | bone arches you fly through               | auto: 12–36 m span            |
| `hut.glb`       | stilt huts over the water                 | auto: ~7 × 7 m, 5 m tall      |
| `house.glb`     | stone houses in the towns                 | auto: 4–7 m, 5–8 m tall       |
| `tower.glb`     | town towers and the holds' towers         | auto: 10–22 m tall            |
| `torch.glb`     | standing torches (the flame is yours)     | auto: 2.6 m tall              |
| `canoe.glb`     | canoes by the stilt villages              | auto: 5.2 m long              |
| `wader.glb`     | the tall marsh birds                      | auto: 3.7 m tall, faces -Z    |
| `grazer.glb`    | the herd animals                          | auto: 2.1 m tall, 3.5 m long  |
| `goat.glb`      | mountain goats                            | auto: as the grazer, smaller  |
| `kite.glb`      | the flying rays                           | auto: 6 m span                |
| `leaper.glb`    | the lake fish                             | auto: 2.3 m long              |

**Variants.** A slot holds any number of files: `tree.glb`, `tree_2.glb`,
`tree_3.glb`… Every tree picks one at random (by its seed, so the same world
gives the same forest), so three tree models make a forest that doesn't
repeat. The same goes for every slot: several houses, several villagers,
several boulders. `meshy-prompts.txt` ships with a few variants for the
slots that gain the most from it.

"Unit" models are stretched per instance: each reed clump scales the model
to its own radius and height, each gate to the gate height. So model those
around 1 m and let the game size them.

A swapped animal keeps the game's behaviour (it still roams and flees) but
loses the placeholder's moving legs — unless its GLB carries animation
clips, which the game plays (`Idle`, `Walk`, `Run`, `Fly`, `Swim` by name).

## Fitting a model: `models.cfg`

The hand-fitted models come in arbitrary sizes and orientations. Adjust them
in `models.cfg`, one line per model (or per variant, `tree_2 …`):

```text
ephemera   scale=6.0  yaw=180  y=-1.0
judith     scale=1.8  yaw=180
reed_tall  scale=1.0
```

- `scale`: uniform scale applied on top of the per-instance size (for
  auto-fitted slots, a nudge on the fit: `1.2` makes every tree a fifth
  bigger than the box the world asked for).
- `yaw`: degrees around the vertical axis. Meshy, like most tools, exports
  facing +Z, while Bevy's forward is -Z, hence the 180 default for the
  creature, the characters and the animals.
- `y`: vertical offset, multiplied by the instance height for unit models.
- `auto`: `0` turns the auto fit off for a slot, `1` on.

The game logs `model: tree_2.glb → tree (...)` at startup for each file it
picks up, and `… model: bottom … top … saddle …` once it has measured it.

## Generating with meshy.ai

**Automatically:** `tools/meshy-gen` drives the Meshy API for every section
of `meshy-prompts.txt` — about thirty models and variants as shipped, each a
preview + refine task (and rigging + animations for the characters), so
check your credits; name the ones you want to make only those:

```sh
export MESHY_API_KEY=msy_...
cargo run --release --manifest-path tools/meshy-gen/Cargo.toml                      # everything missing
cargo run --release --manifest-path tools/meshy-gen/Cargo.toml -- tree tree_2 house  # just these
```

**By hand**, in the web app:

1. Use **Text to 3D** (or Image to 3D if you have concept art).
2. **Remesh** before downloading: it's free and lets you pick a face count.
   Budgets that keep the game fast:
   - `ephemera`: 15-30k triangles (there are 4 on screen)
   - `judith`: 10-20k
   - `reed_tall`, `reed`: **1-3k max**. The race places ~450 clumps.
   - `gate`: 5-10k (11 gates)
   - `mesa`: 2-5k (46 on the horizon)
3. Download as **GLB**. Bevy loads glTF/GLB natively; it can't read FBX or OBJ.
4. Copy the file here with the exact name from the table above.

### Prompts that fit the game's look

> **ephemera**: giant alien mosquito mount, elongated iridescent dark-violet
> chitin body, long thin legs, four translucent veined wings spread flat,
> glowing teal compound eyes, long proboscis, a small woven saddle on the
> thorax, stylized game asset, clean silhouette

> **judith**: young woman tribal rider, lean, leather wrap clothing in ochre
> and rust, hair in a high bun, net slung across her back, bare forearms with
> painted marks, stylized low-poly game character, T-pose

> **reed_tall**: clump of tall alien marsh reeds, violet stalks with
> bioluminescent tips, bundled at the base, stylized, game asset

> **reed**: clump of short green alien marsh grass, dense, stylized, game asset

> **gate**: tribal racing gate, two carved bone-white totems joined by a woven
> arch, hanging feathers and glowing glass beads, stylized game asset

> **mesa**: weathered alien rock spire, layered sediment, flat top, stylized,
> low-poly

### Animation

There are two ways a model moves, and the game picks by what the file
contains:

**1. It brings its own clips (best, humanoids).** Meshy's *Animate* rigs a
humanoid and lets you add actions from its animation library; the GLB then
carries a skeleton and one clip per action. The game plays those clips as
they are, cross-fading as Judith idles, walks, runs, crouches, jumps, swims
or sits in the saddle. Clips are matched to actions by name (`Idle`,
`Walking`, `Running`, `Crouch…`, `Jump`, `Swimming`, `Sitting`…), and the
terminal lists what it found: `Judith model brings 7 clip(s): …`.

`tools/meshy-gen` does this for you when a section in `meshy-prompts.txt`
has an `animate =` line (see `[judith]`): after the model, it calls Meshy's
rigging API, looks the actions up in the animation library, and replaces
`judith.glb` with the rigged, animated one (the static model is kept as
`judith.static.glb`). An existing `judith.glb` without clips is rigged on
the next run — no need to regenerate the model. By hand: open the model in
Meshy → **Animate** → rig it → add the actions → download the GLB with all
of them as `judith.glb`.

**2. The game rigs it (the Ephemera).** Meshy only rigs humanoids, so a
static `ephemera.glb` is rigged at load: the game reads the mesh's shape,
places a skeleton, weights every vertex to the bones by where it sits, and
turns it into a GPU-skinned mesh. The terminal says
`rigged Ephemera model: 7 bones, … vertices`. (A static `judith.glb` gets
the same treatment, with a humanoid skeleton — it works, but her own clips
look far better.)

The game also measures every model: its lowest point keeps the creature on
its legs (not its belly) and Judith's feet on the floor, and the top of the
body at its middle is the **saddle** where she sits — so the `y=` offset in
`models.cfg` only needs to be roughly right.

- **The Ephemera**: the wide, thin parts above the body's middle are the
  wings (they beat, in a figure-eight); the parts hanging below are the legs
  (they dangle at rest and tuck back in flight); the abdomen bobs, the head
  looks around.
- **Judith** (and the riders): in a T-pose (which is why the prompt asks for
  one) her arms stand clear of the body, so arms, legs, head and spine are
  found reliably. She walks, crouches, rides astride, leans into the bank.

If the wings are cut in the wrong place, tune the Ephemera's line in
`models.cfg` (fractions of the model's size):

```text
ephemera  scale=6.0  yaw=180  y=-1.0  wing_hinge=0.2  wing_min_y=0.45  leg_max_y=0.3
```

- `wing_hinge`: where the wings start, as a fraction of the half wingspan
  from the centre line. Raise it if part of the body flaps; lower it if the
  wing roots stay still.
- `wing_min_y`: wings are above this fraction of the height.
- `leg_max_y`: legs are below this fraction of the height.
- **Gate highlight**: the "next gate" glow recolours the placeholder meshes,
  so a custom `gate.glb` won't light up. Something to wire up next.
- **Licence**: on Meshy's free plan, models are CC BY 4.0, so commercial use
  is allowed **with attribution**. Paid plans give a private licence. If you
  ship the game with free-plan models, add a credit.
