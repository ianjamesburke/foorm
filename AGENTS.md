# foorm

## Purpose

Standalone ASCII visualizer fed by OSC over UDP. nooise is the primary
producer; foorm never links against it and never reads its state directly.
The OSC address vocabulary is the only contract between them.

## Ownership

- `src/main.rs` — CLI (`--listen ADDR`), wires intake to the app.
- `src/osc.rs` — UDP intake thread; decodes packets into typed `Event`s. New
  producer addresses are mapped here and nowhere else.
- `src/scene.rs` — `Scene` trait and the scenes that give events a shape.
  Scenes own animation state only; no I/O.
- `src/app.rs` — terminal loop, frame pacing, settings overlay, keys.

## Local Contracts

- Unknown addresses are counted and surfaced in settings, never dropped
  silently.
- A scene must render any terminal size without panicking.
- Heavy rendering belongs here, never in nooise: foorm exists so the producer
  stays light.

## Verification

- `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets` with zero
  warnings before every commit.
