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
`system` (the pad is the sun, every other voice a planet on its own orbit
with trails and moons; the kick sends a shockwave up through it), `binary`
(two stars circling each other, each dragging a ring of particles the kick
shoves off), `tide` (a water line the kick heaves a hump along), `rain`
(drops falling at the level's rate; the kick splashes the bottom edge).
Switch with `1`-`5`, `n`/`p`, or the arrow keys. `Tab` or `s` opens the
settings overlay (listen address, scene, beat, per-voice levels, message
counts, last message). `q` or `Ctrl+C` quits.

Tuning: `t` opens the tune panel, one row per voice plus master, showing the
full-drive level being edited, the level nooise reports right now, and the
drive that pair yields. `↑`/`↓` pick a row, `←`/`→` scale it by 1.25, `r`
resets the row. Edits reach every scene at once. On quit foorm prints the
table as Rust when it differs from the default, ready to paste into
`Sensitivity::default` in `src/scene.rs`.

What every scene shares:

- The kick lands at the bottom centre with at most one cell of wobble, sized
  by the hit's level, so a kick at zero volume draws nothing.
- Colour follows the chord the way the pad does: the new chord's tint swells
  in over its attack while the old one fades over its release.
- Every voice drives something. Pad sets the flow and the sun, bass the
  floor, water height, or star separation, perc and clap the rain rate and
  particle shake, tonal, arp and lead the shimmer and the moons. Silence is still, whatever the tempo is doing.
- Sensitivity is one table (`Sensitivity` in `src/scene.rs`), calibrated
  from nooise's built-in songs. Recalibrate with nooise's
  `song_level_profile` test when the mix changes.

## Address vocabulary

foorm consumes the nooise contract documented in nooise's `src/fluid/osc.rs`:

- `/nooise/beat` `f32` beat position
- `/nooise/level` `f32` master output RMS
- `/nooise/voice/<voice>/level` `f32` per-voice RMS as it enters the mix
  (`pad`, `perc`, `bass`, `kick`, `tonal`, `clap`, `arp`, `lead`)
- `/nooise/chord` `i32 f32 f32` chord index, pad attack and release seconds
- `/nooise/voice/kick` `f32` kick level

Anything else is counted as unknown and shown in settings.
