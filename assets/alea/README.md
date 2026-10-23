# alea works

Everything you see and hear in the endless world is drawn by these works, written in
[alea](https://github.com/sudo-bngz/alea) and rendered by the game at runtime through
the alea library. Each thing in the world is an *edition*: a work plus a seed drawn
from the world seed and where the thing stands. The same world always gives back the
same editions.

They're ordinary alea works: open them in the
[playground](https://alea.bngz.io/playground.html), or render them yourself:

```sh
alea land/marsh.alea -n 6       # six editions: relief, ground painting, ambience
alea sound/race.alea            # the race music
```

Change one and run the game: its editions are repainted and re-rendered (the cache is
keyed on the work's text). A work with an error is logged and its part of the world
falls back (a flat plain, a dark landmark, silence); the game never stops for art.

## `land/` — the lands

Every 512 m region of the world is an edition of one land work; a climate map decides
which, so similar lands cluster. Drop a new `.alea` file here and it joins the world.

| Work | The land | Props it grows |
|---|---|---|
| `marsh.alea` | flat, wet, pocked with pools | reeds, spores |
| `fungal.alea` | rolling hills, one dark lake | giant glowing mushrooms |
| `bones.alea` | long pale dunes | rib arches to fly through |
| `crystal.alea` | a terraced plateau | crystal shards |
| `monoliths.alea` | a flooded basin with a rose window in its floor | monoliths, sigils |
| `grove.alea` | rolling wooded hills with sunken glades | bulb trees, long grass, herds |
| `peaks.alea` | a mountain range, snow on the heights (`range: [0, 5]`) | scree, spires, goats, kites |

The game reads a land work as:

1. **First `render image`: the relief.** The picture is the land. Black is the lake
   bottom (−18 m), 30 % grey the shoreline, white the peaks (+42 m). Give it
   `range: [0, 1]` to be literal; without it, alea's measured range is used, as for
   the image. The range is also the picture's scale of heights: a work that renders
   `range: [0, 5]` climbs five times as high (a value of 5 is +282 m) — that is how
   `peaks.alea` makes mountains.
2. **Second `render image` (optional): the ground painting**, its field and palette.
   Without it the relief's palette paints the ground.
3. **`render sound`: the ambience**, looped, crossfaded with the neighbouring lands'
   as you fly.

The region's canvas (0–1) spans 1024 m around its centre; neighbours overlap and blend.
The game then cuts its own rivers through the relief and flattens the ground under
the tribe's towns; a land work need not know about either.

A land work may declare a `depth` parameter — `work Marsh(depth = 0) { … }` — and
the game gives it the reality's depth (how many veils below the origin it lies,
see `reality.rs`): deeper editions can bend, swell, discolour. `marsh.alea` and
`grove.alea` do; the others ignore depth and only the sky drifts over them.
Keep the relief and painting fields cheap: they are sampled ~20 000 times per chunk.
Noise, `clouds`, `stripes`, `warp`, `posterize`, `smoothstep` cost about a microsecond
a point; a `merge` of twenty shapes costs twenty. (`cargo test chunk_cost -- --ignored
--nocapture` prints the cost.)

## Landmarks

| Work | In the game |
|---|---|
| `monolith.alea` | faces of floating obelisks, animated |
| `veil.alea` | aurora curtains in the sky (painted on black, drawn additively) |
| `sigil.alea` | glowing circles in the mud (painted on black, drawn additively) |
| `orb.alea` | skins of drifting spheres (longitude wraps: computed on cos/sin of x) |

Each has a picture (`render image` or `render video`, sampled over its loop) and a
**`render sound`: its hum**, played in space on the landmark when you're within
170 m. Keep `range:` on video renders: measuring the colour range over the whole
animation costs far more than drawing the frames.

## `sound/` — the rest of what you hear

| Work | Plays |
|---|---|
| `wings.alea` | the Ephemera's hum, looped under the player; the game pitches it up with speed |
| `title.alea` | the title and results screens |
| `camp.alea` | the taming camp |
| `race.alea` | the race |
| `events.alea` | the chimes, one `render sound` each: gate, net thrown, caught, spooked, bump, buck, boost |

Two things alea does to every sound that matter here:

- It **fades in and out over half a second**. A loop is joined by adding its fade-out
  onto its fade-in, so there is no dip; a one-shot has its leading silence trimmed,
  so put the first note after `t = 0.5 / duration`.
- A **drone follows its field stretched to the range it takes**, so a slowly moving
  field dips to silence. A constant field holds (`drone(0.85, …)`); for a hum that
  must never fall silent, add a constant drone under the moving one.

Sounds render on background threads and are cached as WAV in
`~/Library/Caches/ephemera/alea` (macOS) or `~/.cache/ephemera/alea`. Twenty seconds of
stereo with a synth, a texture and a room takes about a second on one core.
`EPHEMERA_NO_ART_CACHE=1` disables the cache; `--mute` starts muted, `M` toggles.
