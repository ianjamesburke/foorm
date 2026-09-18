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
  order), the shared `Pulse` kick/chord bookkeeping, and the scenes. Scenes
  own animation state only; no I/O. Every scene receives every event even
  while hidden, so switching never shows a cold scene.
- `src/app.rs` — terminal loop, frame pacing, settings overlay, keys.

## Local Contracts

- Unknown addresses are counted and surfaced in settings, never dropped
  silently.
- A scene must render any terminal size without panicking
  (`every_scene_renders_any_size_after_hits` enforces it).
- The kick is the anchor: bottom centre, same motion every hit, wobble of at
  most one cell from `wobble(hit)` so all scenes agree.
- Heavy rendering belongs here, never in nooise: foorm exists so the producer
  stays light.

## Verification

- `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets` with zero
  warnings before every commit.
