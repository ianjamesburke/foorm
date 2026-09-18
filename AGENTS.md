# foorm

## Purpose

Standalone ASCII visualizer fed by OSC over UDP. nooise is the primary
producer; foorm never links against it and never reads its state directly.
The OSC address vocabulary is the only contract between them.

## Ownership

- `src/main.rs` — CLI (`--listen ADDR`), wires intake to the app.
- `src/osc.rs` — UDP intake thread; decodes packets into typed `Event`s. New
  producer addresses are mapped here and nowhere else.
- `src/scene.rs` — `Scene` trait, `all()` (the switchable scene list, in key
  order), the shared `Pulse` bookkeeping (kick hits with level, chord layers
  with attack/release envelopes, master level followed as `drive`), and the
  scenes. Scenes own animation state only; no I/O. Every scene receives every
  event even while hidden, so switching never shows a cold scene.
- `src/app.rs` — terminal loop, frame pacing, settings overlay, keys.

## Local Contracts

- Unknown addresses are counted and surfaced in settings, never dropped
  silently.
- A scene must render any terminal size without panicking
  (`every_scene_renders_any_size_after_hits` enforces it).
- The kick is the anchor: bottom centre, same motion every hit, wobble of at
  most one cell from `wobble(hit)` so all scenes agree, scaled by the hit's
  level so an inaudible kick is invisible.
- Colour comes from `Pulse::hue()`, never from a chord index directly: each
  chord is a layer that swells over its attack and fades over its release,
  mirroring the pad.
- Ambient motion scales with per-voice drives from `Pulse::voice`, `rhythm`,
  and `melody`, falling back to the master `drive`; silence is still and the
  beat alone never animates anything.
- Every level threshold lives in `Sensitivity`, calibrated from nooise's
  `song_level_profile` output and dated in its doc comment. Scenes never
  hard-code an RMS. This table is where per-scene user settings will attach.
- Heavy rendering belongs here, never in nooise: foorm exists so the producer
  stays light.

## Verification

- `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets` with zero
  warnings before every commit.
