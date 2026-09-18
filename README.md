# foorm

nooise makes the signal. foorm gives it a shape.

A full-screen ASCII visualizer that listens for OSC and draws what it hears.
The primary producer is [nooise](https://github.com/ianjamesburke/nooise);
anything that speaks OSC can drive it.

## Run

```sh
cargo run
```

Listens on `127.0.0.1:9000` by default (`--listen ADDR` to change). In another
terminal:

```sh
nooise --osc 127.0.0.1:9000
```

Five scenes: `fluid` (a liquid surface the kick pushes a wave through),
`grid` (blocks that flash outward from the kick), `orbit` (particles the kick
shoves off their rings), `tide` (a water line the kick heaves a hump along),
`rain` (drops falling at the level's rate; the kick splashes the bottom edge).
Switch with `1`-`5`, `n`/`p`, or the arrow keys. `Tab` or `s` opens the
settings overlay (listen address, scene, beat, level, message counts, last
message). `q` or `Ctrl+C` quits.

What every scene shares:

- The kick lands at the bottom centre with at most one cell of wobble, sized
  by the hit's level, so a kick at zero volume draws nothing.
- Colour follows the chord the way the pad does: the new chord's tint swells
  in over its attack while the old one fades over its release.
- Ambient motion follows the master level. Silence is still, whatever the
  tempo is doing.

## Address vocabulary

foorm consumes the nooise contract documented in nooise's `src/fluid/osc.rs`:

- `/nooise/beat` `f32` beat position
- `/nooise/level` `f32` master output RMS
- `/nooise/chord` `i32 f32 f32` chord index, pad attack and release seconds
- `/nooise/voice/kick` `f32` kick level

Anything else is counted as unknown and shown in settings.
