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
  with attack/release envelopes, per-voice and master drives), shared layer
  clocks and drawing helpers, and the ten scenes. Scenes own animation state
  only; no I/O. Every scene receives every event even while hidden, so
  switching never shows a cold scene.
- `src/gesture.rs` — `Gestures`: nooise's live gestures (`/nooise/gesture/<name>`)
  as a grade over the finished frame, one look per gesture (Bloom glow, Lift
  brighten and sweep the floor, Submerge darken and sink to blue, Echo
  trails, Thin lattice). Two sources, the larger wins: local `z x c v b`
  holds with nooise's rise and return times (`press`/`release`, or `toggle`
  where the terminal reports no releases), and nooise's mirrored amounts,
  already enveloped. Two held gestures crossfade by relative amount; they
  never stack.
- `src/app.rs` — terminal loop, frame pacing, settings overlay, tune panel
  (`t`; owns the live `Sensitivity`, pushes edits via `Scene::tune`, prints
  the table on quit when changed), keys. Negotiates key-release reporting
  once at start (`supports_keyboard_enhancement`); gesture keys hold when it
  is there and toggle when it is not, never guessing a release.

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
  hard-code an RMS. The tune panel edits it live; a tuned table is pasted
  back into `Sensitivity::default`, never persisted elsewhere. This table is
  where per-scene user settings will attach.
- Gestures are a grade, not a scene: scenes never read gesture amounts, so a
  held key looks the same on every scene and a new scene inherits it for free.
- Heavy rendering belongs here, never in nooise: foorm exists so the producer
  stays light.

## Verification

- `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets` with zero
  warnings before every commit.
