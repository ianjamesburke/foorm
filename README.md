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

Each kick hit spawns a ring. `Tab` or `s` opens the settings overlay (listen
address, scene, beat, message counts, last message). `q` quits.

## Address vocabulary

foorm consumes the nooise contract documented in nooise's `src/fluid/osc.rs`:

- `/nooise/beat` `f32`
- `/nooise/chord` `i32`
- `/nooise/voice/kick`

Anything else is counted as unknown and shown in settings.
