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

Ten scenes, in switch order:

- `fluid`: pad flow, bass floor, melodic shimmer, kick waves.
- `system`: pad sun; bass, perc, clap, tonal, arp, lead and kick planets; melodic moons.
- `binary`: pad rotation, bass separation, perc/clap shake, tonal/arp/lead particle rings, kick shove.
- `tide`: bass waterline, pad swell, tonal/arp/lead ripples, kick heave.
- `rain`: perc/clap and tonal/arp/lead drive falling drops; kick splashes the bottom.
- `estuary`: pad aurora, bass dunes, perc rain, clap lightning, tonal moon, arp birds, lead ribbon; kick ground ripple.
- `loom`: pad curtains, bass ribs, perc beads, clap crossbars, tonal waves, arp stairs, lead braid; kick lifts the foot.
- `reef`: pad kelp, bass coral, perc bubbles, clap fans, tonal anemone, arp fish, lead ray; kick seabed ripple.
- `city`: pad searchlights, bass skyline, perc traffic, clap beacons, tonal clock tower, arp windows, lead airship; kick street ripple.
- `atlas`: pad shells, bass diamond, perc rosette, clap burst, tonal harmonic rings, arp square chase, lead spiral; paired voices light bridges, kick pulses below.

Switch with `1`-`9` and `0` (atlas), `n`/`p`, or the arrow keys.
`Tab` or `s` opens the settings overlay (listen address, scene, beat,
per-voice levels, message counts, last message). `q` or `Ctrl+C` quits.

Gestures: hold nooise's `z` Bloom, `x` Lift, `c` Submerge, `v` Echo, or
`b` Thin and the picture follows the sound: Bloom glows outward, Lift
brightens and sweeps the floor away, Submerge darkens and sinks to blue,
Echo leaves trails, Thin drops cells to a lattice. Amounts come from nooise,
so the picture rises and returns exactly with the audio. Holding a second
gesture crossfades from the first instead of stacking. The settings overlay
shows each gesture's amount.

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
- The five layered scenes give each voice a distinct shape and its own clock.
  Master adds background shimmer; kick drive swells the bottom-centre foot.
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
