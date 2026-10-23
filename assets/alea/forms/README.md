# `forms/` — buildings, trees and craft sculpted by alea

Every work in these folders becomes a 3D shape in the game. Each world draws
four editions of every work (seeded from the world and the reality's depth),
on background threads; the cities, groves and lone spires then pick among them.
Drop a new `.alea` file into a folder and it joins the pool at the next start.

| Folder | Read as | Used for |
|---|---|---|
| `tower/` | lathe | the spires of a city, and lone spires in the wild |
| `tree/` | lathe | trees on the avenues and in the groves (they sway) |
| `arcology/` | relief | the huge building at the heart of every city |
| `vehicle/` | relief, mirrored underneath | ground craft on the ring roads, flyers in the city sky |

## What the game reads

1. The **first `render sculpture(shape, side:, relief:)`** is the shape.
   `side` is the size of the footprint in metres, `relief` the height in metres
   that a field value of 1 reaches (multiplied by `side` for lathes — see below).
2. The **first `render image`** is the paint (the colour texture).
3. The **second `render image`**, if there is one, is the glow (emissive): black
   is unlit, anything brighter shines.

A work may take a `depth` parameter (`work Spire(depth = 0)`); the deeper the
reality, the higher the number it gets, so a form can drift as you go down.

## Lathe (towers, trees)

The shape turns round a vertical axis. `x` goes once round (0 → 1), `y` runs
from the **tip (0) to the foot (1)**. The field is the **radius**, 0–1, as a
fraction of half the side. The height of the form is `relief × side`. Radii
under 0.01 are left open, so a field can thin to nothing at the tip.

## Relief (arcologies, vehicles)

The field is a height map seen **from above**: `x`, `y` across the footprint,
the field is the height 0–1 (times `relief`). Cells where the field is under
0.01 are left out, so the footprint can be any shape. Vehicles have their nose
at `y = 0` and are mirrored underneath to give them a belly.

Arcologies and towers are solid (Judith and the Ephemera collide with them);
trees and craft are not.

## Rules of thumb

- Whole numbers where alea wants them: `posterize` levels and the frequency of
  anything periodic in `x` must be whole, so pick them with `choose { 6 | 8 }`,
  never `~between`.
- Keep a lathe's field above 0 at the foot, or the form stands on a needle.
- `cargo test every_form_draws` sculpts three editions of every work and checks
  their sizes are sane for their kind.
