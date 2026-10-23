# alea works

The landmarks of the endless world are painted at runtime by these works, written in
[alea](https://github.com/sudo-bngz/alea). Each landmark shows an *edition*: a seed
drawn from the world seed and the landmark's position.

| Work | In the game | Notes |
|---|---|---|
| `monolith.alea` | faces of floating obelisks | tall (1:3), animated loop |
| `veil.alea` | aurora curtains in the sky | painted on black, drawn additively |
| `sigil.alea` | glowing circles in the mud | painted on black, drawn additively |
| `orb.alea` | skins of drifting spheres | longitude wraps: computed on cos/sin of x |

They're ordinary alea works, so you can open them in the
[playground](https://alea.bngz.io/playground.html) or render them:

```sh
alea monolith.alea -n 6
```

Change one and run the game: its editions are repainted (the cache is keyed on the
work's text). Keep `range:` on video renders. Measuring the colour range over the
whole animation costs far more than drawing the frames, which matters when the world
paints dozens of editions while you fly.
