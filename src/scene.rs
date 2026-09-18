//! Scenes: each gives incoming events a shape. A scene owns only animation
//! state; it never touches the network or the terminal directly.
//!
//! The kick is the anchor of every scene: same place, same motion, every hit,
//! with at most one cell of analog wobble so it reads as played, not looped.
//! Its size follows the hit's level, so an inaudible kick is invisible too.
//! Ambient motion follows the master level: silence is still.

use std::f32::consts::TAU;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::osc::Event;

pub trait Scene {
    fn name(&self) -> &'static str;
    fn on_event(&mut self, event: &Event);
    fn tick(&mut self, dt: f32);
    fn render(&self, area: Rect, buf: &mut Buffer);
}

pub fn all() -> Vec<Box<dyn Scene>> {
    vec![
        Box::new(Fluid::default()),
        Box::new(Grid::default()),
        Box::new(Orbit::default()),
        Box::new(Tide::default()),
        Box::new(Rain::default()),
    ]
}

const GRADIENT: &[char] = &[' ', '·', '∙', '•', '●', '◉', '⬤'];
const HUES: [f32; 5] = [205.0, 270.0, 325.0, 158.0, 38.0];

/// Master RMS that counts as fully driven; nooise's mix sits well under 1.0.
const FULL_LEVEL: f32 = 0.2;

/// One cell of wobble per hit, in cells, derived from the hit count so the
/// same hit lands the same way in every scene.
fn wobble(hit: u64) -> (f32, f32) {
    let x = ((hit * 7) % 3) as f32 - 1.0;
    let y = ((hit * 5) % 3) as f32 - 1.0;
    (x, y)
}

/// One live kick hit.
#[derive(Clone, Copy)]
struct Hit {
    age: f32,
    level: f32,
    wobble: (f32, f32),
}

/// A voiced chord's tint, enveloped the way the pad layer it mirrors is: it
/// swells in over `attack`, and once the next chord lands it fades out over
/// `release`. What the eye sees is the weighted blend of every live layer.
struct ChordLayer {
    hue: f32,
    age: f32,
    attack: f32,
    release: f32,
    released_at: Option<f32>,
}

impl ChordLayer {
    fn weight(&self) -> f32 {
        let swell = (self.age / self.attack.max(1e-3)).min(1.0);
        let decay = self
            .released_at
            .map(|at| 1.0 - (self.age - at) / self.release.max(1e-3))
            .unwrap_or(1.0);
        (swell * decay).clamp(0.0, 1.0)
    }
}

/// Shared kick, chord, and level bookkeeping every scene carries.
#[derive(Default)]
struct Pulse {
    hits: u64,
    live: Vec<Hit>,
    chords: Vec<ChordLayer>,
    beat: f32,
    /// Master RMS as last reported.
    level: f32,
    /// `level` followed with a fast rise and slow fall, 0..1 against
    /// `FULL_LEVEL`: what ambient motion runs on.
    drive: f32,
}

impl Pulse {
    const LIFETIME: f32 = 2.0;

    fn on_event(&mut self, event: &Event) {
        match event {
            Event::Kick(level) => {
                self.hits += 1;
                self.live.push(Hit {
                    age: 0.0,
                    level: level.clamp(0.0, 1.0),
                    wobble: wobble(self.hits),
                });
            }
            Event::Chord {
                index,
                attack,
                release,
            } => {
                for layer in &mut self.chords {
                    layer.released_at.get_or_insert(layer.age);
                }
                self.chords.push(ChordLayer {
                    hue: HUES[(*index).rem_euclid(HUES.len() as i32) as usize],
                    age: 0.0,
                    attack: *attack,
                    release: *release,
                    released_at: None,
                });
            }
            Event::Beat(b) => self.beat = *b,
            Event::Level(l) => self.level = *l,
            Event::Unknown(_) => {}
        }
    }

    fn tick(&mut self, dt: f32) {
        for hit in &mut self.live {
            hit.age += dt;
        }
        self.live.retain(|hit| hit.age < Self::LIFETIME);
        for layer in &mut self.chords {
            layer.age += dt;
        }
        self.chords
            .retain(|layer| layer.released_at.is_none() || layer.weight() > 0.0);
        let target = (self.level / FULL_LEVEL).clamp(0.0, 1.0);
        let rate = if target > self.drive { 12.0 } else { 2.0 };
        self.drive += (target - self.drive) * (rate * dt).min(1.0);
    }

    fn fade(age: f32) -> f32 {
        (1.0 - age / Self::LIFETIME).max(0.0)
    }

    /// Circular weighted mean of every live chord layer's hue.
    fn hue(&self) -> f32 {
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for layer in &self.chords {
            let w = layer.weight();
            x += w * layer.hue.to_radians().cos();
            y += w * layer.hue.to_radians().sin();
        }
        if x == 0.0 && y == 0.0 {
            return HUES[0];
        }
        y.atan2(x).to_degrees().rem_euclid(360.0)
    }

    /// The latest hit, for scenes that draw the anchor itself.
    fn anchor(&self) -> Option<&Hit> {
        self.live.last()
    }
}

fn paint(buf: &mut Buffer, area: Rect, x: u16, y: u16, v: f32, hue: f32) {
    if x >= area.width || y >= area.height {
        return;
    }
    let v = v.clamp(0.0, 1.0);
    let idx = ((v * (GRADIENT.len() - 1) as f32).round() as usize).min(GRADIENT.len() - 1);
    buf[(area.x + x, area.y + y)]
        .set_char(GRADIENT[idx])
        .set_style(Style::default().fg(hsv(hue, 0.7, 0.12 + v * 0.88)));
}

// ------------------------------------------------------------ Fluid

/// A liquid field. The kick sits at the bottom centre and shoves a wave
/// upward through it; the chord tints the whole surface; the level sets how
/// fast the liquid moves, so silence is a still pool.
#[derive(Default)]
pub struct Fluid {
    pulse: Pulse,
    phase: f32,
}

impl Scene for Fluid {
    fn name(&self) -> &'static str {
        "fluid"
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        self.phase += dt * 0.5 * self.pulse.drive;
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let z = self.phase;
        let hue = self.pulse.hue();
        let amp = 0.25 + 0.75 * self.pulse.drive;
        for y in 0..area.height {
            for x in 0..area.width {
                let nx = x as f32 / w;
                let ny = y as f32 / h;
                let mut v = ((nx * 6.0 + z).sin() * (ny * 5.0 - z * 0.7).cos()
                    + ((nx * 3.3 - ny * 4.1) + z * 1.3).sin() * 0.7
                    + ((nx + ny) * 7.5 + (z * 0.9).sin() * 2.0).cos() * 0.5)
                    * amp;
                for hit in &self.pulse.live {
                    let (wx, wy) = hit.wobble;
                    let cx = 0.5 + wx / w;
                    let cy = 1.0 + wy / h;
                    let dx = (nx - cx) * 2.0; // cells are ~2:1
                    let dy = ny - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let front = hit.age * 0.6;
                    let ring = (-((dist - front) * 9.0).powi(2)).exp();
                    v += ring * Pulse::fade(hit.age) * 3.0 * hit.level;
                }
                let v = (v / 3.0).tanh() * 0.5 + 0.5;
                paint(buf, area, x, y, v, hue + (v - 0.5) * 40.0);
            }
        }
    }
}

// ------------------------------------------------------------ Grid

/// A coarse grid of blocks. The kick lights the bottom-centre block and the
/// flash spreads outward one block per frame; the beat walks a cursor whose
/// brightness follows the level.
#[derive(Default)]
pub struct Grid {
    pulse: Pulse,
}

impl Grid {
    const CELL_W: u16 = 4;
    const CELL_H: u16 = 2;
}

impl Scene for Grid {
    fn name(&self) -> &'static str {
        "grid"
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let cols = (area.width / Self::CELL_W).max(1);
        let rows = (area.height / Self::CELL_H).max(1);
        let beat_col = (self.pulse.beat.floor() as u64 % cols as u64) as u16;
        let hue = self.pulse.hue();
        for y in 0..area.height {
            for x in 0..area.width {
                let col = (x / Self::CELL_W).min(cols - 1);
                let row = (y / Self::CELL_H).min(rows - 1);
                let mut v = 0.06;
                if col == beat_col {
                    v += 0.3 * self.pulse.drive;
                }
                for hit in &self.pulse.live {
                    let (wx, wy) = hit.wobble;
                    let origin_col = (cols / 2) as f32 + wx;
                    let origin_row = (rows - 1) as f32 + wy;
                    let d = (col as f32 - origin_col)
                        .abs()
                        .max((row as f32 - origin_row).abs());
                    let front = hit.age * 12.0;
                    let ring = (-((d - front) * 0.8).powi(2)).exp();
                    v += ring * Pulse::fade(hit.age) * hit.level;
                }
                let border = x % Self::CELL_W == 0 || y % Self::CELL_H == 0;
                paint(buf, area, x, y, if border { v * 0.3 } else { v }, hue);
            }
        }
    }
}

// ------------------------------------------------------------ Orbit

/// Particles circling the centre at a speed set by the level. The kick
/// shoves them outward from the bottom, and they settle back on their orbits.
pub struct Orbit {
    pulse: Pulse,
    /// (angle, radius, radial velocity) per particle
    particles: Vec<(f32, f32, f32)>,
}

impl Default for Orbit {
    fn default() -> Self {
        let particles = (0..96)
            .map(|i| {
                let f = i as f32 / 96.0;
                (f * TAU, 0.15 + (f * 7.0).fract() * 0.3, 0.0)
            })
            .collect();
        Self {
            pulse: Pulse::default(),
            particles,
        }
    }
}

impl Scene for Orbit {
    fn name(&self) -> &'static str {
        "orbit"
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
        if let Event::Kick(level) = event {
            let (wx, _) = wobble(self.pulse.hits);
            for p in &mut self.particles {
                // push hardest on the particles nearest the bottom
                let bottom = (p.0 + wx * 0.05).sin().max(0.0);
                p.2 += 0.6 * bottom * level.clamp(0.0, 1.0);
            }
        }
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        let speed = self.pulse.drive;
        for p in &mut self.particles {
            p.0 += dt * (0.3 + p.1) * speed;
            p.1 += p.2 * dt;
            p.2 -= (p.1 - 0.3) * 4.0 * dt; // spring back toward orbit
            p.2 *= 1.0 - 2.5 * dt;
        }
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let hue = self.pulse.hue();
        for y in 0..area.height {
            for x in 0..area.width {
                paint(buf, area, x, y, 0.0, hue);
            }
        }
        for &(angle, radius, vel) in &self.particles {
            let x = 0.5 + angle.cos() * radius;
            let y = 0.5 + angle.sin() * radius;
            if !(0.0..1.0).contains(&x) || !(0.0..1.0).contains(&y) {
                continue;
            }
            let v = (0.45 + vel.abs() * 1.5).clamp(0.0, 1.0);
            paint(
                buf,
                area,
                (x * w) as u16,
                (y * h) as u16,
                v,
                hue + vel * 60.0,
            );
        }
        if let (Some(hit), Some(bottom)) = (self.pulse.anchor(), area.height.checked_sub(1)) {
            let kx = ((w / 2.0) + hit.wobble.0).clamp(0.0, w - 1.0) as u16;
            paint(buf, area, kx, bottom, Pulse::fade(hit.age) * hit.level, hue);
        }
    }
}

// ------------------------------------------------------------ Tide

/// A water line across the screen. The level sets how high it sits and how
/// much it swells; the kick heaves a hump up from the bottom centre that
/// travels outward along the surface.
#[derive(Default)]
pub struct Tide {
    pulse: Pulse,
    phase: f32,
}

impl Scene for Tide {
    fn name(&self) -> &'static str {
        "tide"
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        self.phase += dt * 1.2 * self.pulse.drive;
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let hue = self.pulse.hue();
        let drive = self.pulse.drive;
        for x in 0..area.width {
            let nx = x as f32 / w;
            // surface height in 0..1 from the bottom
            let mut surface = 0.08
                + 0.25 * drive
                + drive
                    * 0.06
                    * ((nx * 9.0 + self.phase).sin() + (nx * 4.0 - self.phase * 0.6).cos());
            for hit in &self.pulse.live {
                let (wx, wy) = hit.wobble;
                let dx = (nx - 0.5 - wx / w) * 2.0;
                let front = hit.age * 0.9;
                let hump = (-((dx.abs() - front) * 6.0).powi(2)).exp();
                surface += hump * Pulse::fade(hit.age) * hit.level * (0.45 + wy * 0.02);
            }
            let surface = surface.clamp(0.0, 1.0);
            for y in 0..area.height {
                let depth = surface - (1.0 - y as f32 / h);
                let v = if depth < 0.0 {
                    0.0
                } else {
                    (0.9 - depth * 1.2).max(0.15)
                };
                paint(buf, area, x, y, v, hue - depth.max(0.0) * 30.0);
            }
        }
    }
}

// ------------------------------------------------------------ Rain

/// Drops fall at a rate set by the level. Each kick throws a splash along the
/// bottom edge, outward from the centre.
pub struct Rain {
    pulse: Pulse,
    /// (x in 0..1, y in 0..1, fall speed) per drop
    drops: Vec<(f32, f32, f32)>,
    /// Drops owed by the level since the last spawn.
    due: f32,
    seed: u64,
}

impl Default for Rain {
    fn default() -> Self {
        Self {
            pulse: Pulse::default(),
            drops: Vec::new(),
            due: 0.0,
            seed: 0x9E37_79B9_7F4A_7C15,
        }
    }
}

impl Rain {
    /// Next value in 0..1 from a splitmix64 step: deterministic, no crate.
    fn next_unit(&mut self) -> f32 {
        self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) as f32 / u64::MAX as f32
    }
}

impl Scene for Rain {
    fn name(&self) -> &'static str {
        "rain"
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        self.due += dt * 40.0 * self.pulse.drive;
        while self.due >= 1.0 && self.drops.len() < 400 {
            self.due -= 1.0;
            let x = self.next_unit();
            let speed = 0.4 + self.next_unit() * 0.6;
            self.drops.push((x, 0.0, speed));
        }
        for drop in &mut self.drops {
            drop.1 += drop.2 * dt;
        }
        self.drops.retain(|drop| drop.1 < 1.0);
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let hue = self.pulse.hue();
        for y in 0..area.height {
            for x in 0..area.width {
                paint(buf, area, x, y, 0.0, hue);
            }
        }
        for &(x, y, speed) in &self.drops {
            paint(
                buf,
                area,
                (x * w) as u16,
                (y * h) as u16,
                0.3 + speed * 0.5,
                hue,
            );
        }
        let Some(bottom) = area.height.checked_sub(1) else {
            return;
        };
        for x in 0..area.width {
            let mut v = 0.0;
            for hit in &self.pulse.live {
                let (wx, wy) = hit.wobble;
                let d = (x as f32 - (w / 2.0 + wx)).abs() / w * 2.0;
                let front = hit.age * 0.8;
                let ring = (-((d - front) * 8.0).powi(2)).exp();
                v += ring * Pulse::fade(hit.age) * hit.level;
                // the splash lifts a cell up on the wobble's second axis
                if wy > 0.0
                    && v > 0.5
                    && let Some(above) = bottom.checked_sub(1)
                {
                    paint(buf, area, x, above, v * 0.4, hue);
                }
            }
            paint(buf, area, x, bottom, v, hue);
        }
    }
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::Rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(index: i32, attack: f32, release: f32) -> Event {
        Event::Chord {
            index,
            attack,
            release,
        }
    }

    #[test]
    fn every_scene_renders_any_size_after_hits() {
        for (w, h) in [(0, 0), (1, 1), (3, 2), (46, 10), (200, 60)] {
            for mut scene in all() {
                for event in [
                    Event::Level(0.3),
                    Event::Kick(0.9),
                    chord(3, 0.5, 0.5),
                    Event::Beat(7.5),
                    Event::Kick(0.0),
                ] {
                    scene.on_event(&event);
                }
                scene.tick(0.3);
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                scene.render(area, &mut buf);
            }
        }
    }

    #[test]
    fn wobble_stays_within_one_cell_and_repeats_per_hit() {
        for hit in 0..50 {
            let (x, y) = wobble(hit);
            assert!(x.abs() <= 1.0 && y.abs() <= 1.0);
            assert_eq!(wobble(hit), wobble(hit));
        }
    }

    #[test]
    fn hue_crosses_over_the_attack_and_old_chord_lingers_for_the_release() {
        let mut pulse = Pulse::default();
        pulse.on_event(&chord(0, 1.0, 4.0));
        pulse.tick(2.0);
        assert!((pulse.hue() - HUES[0]).abs() < 0.5);

        pulse.on_event(&chord(1, 4.0, 2.0));
        pulse.tick(1.0);
        // a quarter into a 4 s attack: still much closer to the old hue
        let quarter = pulse.hue();
        let weights: Vec<f32> = pulse.chords.iter().map(ChordLayer::weight).collect();
        assert!(
            (quarter - HUES[0]).abs() < (quarter - HUES[1]).abs(),
            "hue {quarter} weights {weights:?}"
        );
        pulse.tick(3.0);
        // old layer released 4 s ago against its own 4 s release: gone
        assert_eq!(pulse.chords.len(), 1);
        assert!((pulse.hue() - HUES[1]).abs() < 0.5);
    }

    #[test]
    fn drive_follows_level_and_silence_settles_to_zero() {
        let mut pulse = Pulse::default();
        pulse.on_event(&Event::Level(FULL_LEVEL));
        pulse.tick(0.5);
        assert!(pulse.drive > 0.9);
        pulse.on_event(&Event::Level(0.0));
        for _ in 0..100 {
            pulse.tick(0.1);
        }
        assert!(pulse.drive < 0.01);
    }
}
