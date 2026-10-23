# Custom models (meshy.ai or any glTF source)

Drop a `.glb` file here with one of the names below and the game uses it on
the next run. No code change, no rebuild. A missing file keeps the built-in
placeholder, so you can swap models one at a time.

| File            | Replaces                                  | Placeholder size        |
|-----------------|-------------------------------------------|-------------------------|
| `ephemera.glb`  | the creature (player + 3 rivals)          | ~7 m long, faces -Z     |
| `judith.glb`    | Judith on foot and in the saddle          | ~1.9 m tall             |
| `reed_tall.glb` | tall, *solid* reed clumps (obstacles)     | unit: 1 m radius × 1 m  |
| `reed.glb`      | short reed clumps (soft, fly-through)     | unit: 1 m radius × 1 m  |
| `gate.glb`      | race gates                                | unit: 1 m tall          |
| `mesa.glb`      | distant rock formations on the horizon    | unit: 1 m               |

"Unit" models are stretched per instance: each reed clump scales the model
to its own radius and height, each gate to the gate height. So model those
around 1 m and let the game size them.

## Fitting a model: `models.cfg`

Generated models come in arbitrary sizes and orientations. Adjust them in
`models.cfg`, one line per model:

```text
ephemera   scale=6.0  yaw=180  y=-1.0
judith     scale=1.8  yaw=180
reed_tall  scale=1.0
```

- `scale`: uniform scale applied on top of the per-instance size.
- `yaw`: degrees around the vertical axis. Meshy, like most tools, exports
  facing +Z, while Bevy's forward is -Z, hence the 180 default for the
  creature and Judith.
- `y`: vertical offset, multiplied by the instance height for unit models.

The game logs `model override: ephemera.glb (...)` at startup for each model
it picks up.

## Generating with meshy.ai

**Automatically:** `tools/meshy-gen` drives the Meshy API for every slot, using the
prompts in `meshy-prompts.txt`:

```sh
export MESHY_API_KEY=msy_...
cargo run --release --manifest-path tools/meshy-gen/Cargo.toml
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
