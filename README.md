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

Three scenes: `fluid` (a liquid surface the kick pushes a wave through),
`grid` (blocks that flash outward from the kick), `orbit` (particles the kick
shoves off their rings). The kick always lands at the bottom centre with at
most one cell of wobble. Switch with `1`-`3`, `n`/`p`, or the arrow keys.
`Tab` or `s` opens the settings overlay (listen address, scene, beat, message
counts, last message). `q` or `Ctrl+C` quits.

## Address vocabulary

foorm consumes the nooise contract documented in nooise's `src/fluid/osc.rs`:

- `/nooise/beat` `f32`
- `/nooise/chord` `i32`
- `/nooise/voice/kick`

Anything else is counted as unknown and shown in settings.
