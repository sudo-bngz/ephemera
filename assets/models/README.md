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

### Things to know

- **Wing animation**: the placeholder creature flaps its wings procedurally.
  A Meshy model is a single static mesh, so the swapped Ephemera keeps its
  flight motion (banking, bobbing, pitching) but its wings won't beat. Meshy's
  auto-rigging targets humanoids and quadrupeds, not insects. Rigging the
  wings in Blender would restore the flapping. Judith is humanoid, so
  Meshy's rigging and animation clips do work for her.
- **Gate highlight**: the "next gate" glow recolours the placeholder meshes,
  so a custom `gate.glb` won't light up. Something to wire up next.
- **Licence**: on Meshy's free plan, models are CC BY 4.0, so commercial use
  is allowed **with attribution**. Paid plans give a private licence. If you
  ship the game with free-plan models, add a credit.
