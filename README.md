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

Thirteen scenes, in switch order:

- `fluid`: pad flow, bass floor, melodic shimmer, kick waves.
- `system`: pad sun; bass, perc, clap, tonal, arp, lead and kick planets; melodic moons.
- `binary`: pad rotation, bass separation, perc/clap shake, tonal/arp/lead particle rings, kick shove.
- `tide`: bass waterline, pad swell, tonal/arp/lead ripples, kick heave.
- `rain`: perc/clap and tonal/arp/lead drive falling drops; kick splashes the bottom.
- `estuary`: striped pad sun, breathing bass grid, perc floor glints, clap horizon, tonal/arp/lead sky bands; kick shockwave.
- `loom`: pad swell, bass horizontal bands, perc/clap oblique grain, tonal/arp/lead crossing waves; scalar interference with a kick ripple.
- `reef`: pad upper pool, bass lower pool, perc/clap side lobes, tonal/arp upper corners, lead core; merging metaballs lifted by kick.
- `city`: pad sky, bass skyline, arp dithered windows, perc street, clap horizon flicker, tonal/lead beams; kick street pulse.
- `atlas`: pad shells, bass diamond, perc rosette, clap burst, tonal harmonic rings, arp square chase, lead spiral; paired voices light bridges, kick pulses below.
- `veil`: pad haze, bass lower glow, perc/clap diagonal curtains, tonal/arp/lead drifting ribbons; half-block gradients and kick rings.
- `ion`: pad/bass broad folds, perc/clap oblique folds, tonal/arp/lead finer rotating folds; Bayer-dithered plasma and kick rings.
- `echo`: pad outer curl, bass perimeter, perc outer ripple, clap middle ripple, tonal inner halo, arp core ring, lead middle curl; retained trails and fading kick rings.

Switch with `1`-`9` and `0` (atlas), `n`/`p`, or the arrow keys.
Use `n`/`p` or arrows to reach veil, ion and echo after atlas.
`Tab` or `s` opens the settings overlay (listen address, scene, beat,
per-voice levels, message counts, last message). `q` or `Ctrl+C` quits.

Gestures: hold `z` Bloom, `x` Lift, `c` Submerge, `v` Echo, or `b` Thin,
the same keys as nooise. Bloom glows outward, Lift brightens and sweeps the
floor away, Submerge darkens and sinks to blue, Echo leaves trails, Thin
drops cells to a lattice. A held key rises at nooise's speed and returns in
50 ms on release; in a terminal that reports no key releases a press toggles
the hold instead, and the settings overlay says so. Gestures held in nooise
arrive over OSC too and the larger of the two amounts wins, so one hand on
either keyboard drives the picture. Holding a second gesture crossfades from
the first instead of stacking. The settings overlay shows each gesture's
amount and marks a local hold with `*`.

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
- Estuary through echo give each voice its own layer and clock. The seven
  neon field scenes sample every cell, with 2:1 cell aspect and subdued CRT
  scanlines. Master adds moving haze; kick drive lights the bottom centre.
  Silence is still, whatever the tempo is doing.
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
