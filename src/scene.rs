//! Scenes: each gives incoming events a shape. A scene owns only animation
//! state; it never touches the network or the terminal directly.
//!
//! The kick is the anchor of every scene: same place, same motion, every hit,
//! with at most one cell of analog wobble so it reads as played, not looped.

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
    ]
}

const GRADIENT: &[char] = &[' ', '·', '∙', '•', '●', '◉', '⬤'];
const HUES: [f32; 5] = [205.0, 270.0, 325.0, 158.0, 38.0];

/// One cell of wobble per hit, in cells, derived from the hit count so the
/// same hit lands the same way in every scene.
fn wobble(hit: u64) -> (f32, f32) {
    let x = ((hit * 7) % 3) as f32 - 1.0;
    let y = ((hit * 5) % 3) as f32 - 1.0;
    (x, y)
}

/// Shared kick and chord bookkeeping every scene carries.
#[derive(Default)]
struct Pulse {
    hits: u64,
    /// (age in seconds, wobble in cells) per live hit
    live: Vec<(f32, (f32, f32))>,
    hue: f32,
    beat: f32,
}

impl Pulse {
    const LIFETIME: f32 = 2.0;

    fn on_event(&mut self, event: &Event) {
        match event {
            Event::Kick => {
                self.hits += 1;
                self.live.push((0.0, wobble(self.hits)));
            }
            Event::Chord(c) => self.hue = HUES[(*c).rem_euclid(HUES.len() as i32) as usize],
            Event::Beat(b) => self.beat = *b,
            Event::Unknown(_) => {}
        }
    }

    fn tick(&mut self, dt: f32) {
        for hit in &mut self.live {
            hit.0 += dt;
        }
        self.live.retain(|hit| hit.0 < Self::LIFETIME);
    }

    fn fade(age: f32) -> f32 {
        (1.0 - age / Self::LIFETIME).max(0.0)
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
/// upward through it; the chord tints the whole surface.
#[derive(Default)]
pub struct Fluid {
    pulse: Pulse,
    t: f32,
}

impl Scene for Fluid {
    fn name(&self) -> &'static str {
        "fluid"
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.t += dt;
        self.pulse.tick(dt);
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let z = self.t * 0.4;
        for y in 0..area.height {
            for x in 0..area.width {
                let nx = x as f32 / w;
                let ny = y as f32 / h;
                let mut v = (nx * 6.0 + z).sin() * (ny * 5.0 - z * 0.7).cos()
                    + ((nx * 3.3 - ny * 4.1) + z * 1.3).sin() * 0.7
                    + ((nx + ny) * 7.5 + (z * 0.9).sin() * 2.0).cos() * 0.5;
                for &(age, (wx, wy)) in &self.pulse.live {
                    let cx = 0.5 + wx / w;
                    let cy = 1.0 + wy / h;
                    let dx = (nx - cx) * 2.0; // cells are ~2:1
                    let dy = ny - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let front = age * 0.6;
                    let ring = (-((dist - front) * 9.0).powi(2)).exp();
                    v += ring * Pulse::fade(age) * 3.0;
                }
                let v = (v / 3.0).tanh() * 0.5 + 0.5;
                paint(buf, area, x, y, v, self.pulse.hue + (v - 0.5) * 40.0);
            }
        }
    }
}

// ------------------------------------------------------------ Grid

/// A coarse grid of blocks. The kick lights the bottom-centre block and the
/// flash spreads outward one block per frame; the beat walks a cursor.
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
        for y in 0..area.height {
            for x in 0..area.width {
                let col = (x / Self::CELL_W).min(cols - 1);
                let row = (y / Self::CELL_H).min(rows - 1);
                let mut v = if col == beat_col { 0.18 } else { 0.06 };
                for &(age, (wx, wy)) in &self.pulse.live {
                    let origin_col = (cols / 2) as f32 + wx;
                    let origin_row = (rows - 1) as f32 + wy;
                    let d = (col as f32 - origin_col)
                        .abs()
                        .max((row as f32 - origin_row).abs());
                    let front = age * 12.0;
                    let ring = (-((d - front) * 0.8).powi(2)).exp();
                    v += ring * Pulse::fade(age);
                }
                let border = x % Self::CELL_W == 0 || y % Self::CELL_H == 0;
                paint(
                    buf,
                    area,
                    x,
                    y,
                    if border { v * 0.3 } else { v },
                    self.pulse.hue,
                );
            }
        }
    }
}

// ------------------------------------------------------------ Orbit

/// Particles circling the centre. The kick shoves them outward from the
/// bottom, and they settle back on their orbits.
pub struct Orbit {
    pulse: Pulse,
    t: f32,
    /// (angle, radius, radial velocity) per particle
    particles: Vec<(f32, f32, f32)>,
}

impl Default for Orbit {
    fn default() -> Self {
        let particles = (0..96)
            .map(|i| {
                let f = i as f32 / 96.0;
                (
                    f * std::f32::consts::TAU,
                    0.15 + (f * 7.0).fract() * 0.3,
                    0.0,
                )
            })
            .collect();
        Self {
            pulse: Pulse::default(),
            t: 0.0,
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
        if let Event::Kick = event {
            let (wx, _) = wobble(self.pulse.hits);
            for p in &mut self.particles {
                // push hardest on the particles nearest the bottom
                let bottom = (p.0 + wx * 0.05).sin().max(0.0);
                p.2 += 0.6 * bottom;
            }
        }
    }
    fn tick(&mut self, dt: f32) {
        self.t += dt;
        self.pulse.tick(dt);
        for p in &mut self.particles {
            p.0 += dt * (0.3 + p.1);
            p.1 += p.2 * dt;
            p.2 -= (p.1 - 0.3) * 4.0 * dt; // spring back toward orbit
            p.2 *= 1.0 - 2.5 * dt;
        }
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        for y in 0..area.height {
            for x in 0..area.width {
                paint(buf, area, x, y, 0.0, self.pulse.hue);
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
                self.pulse.hue + vel * 60.0,
            );
        }
        let (wx, _) = self
            .pulse
            .live
            .last()
            .map(|hit| hit.1)
            .unwrap_or((0.0, 0.0));
        let glow = self
            .pulse
            .live
            .last()
            .map(|hit| Pulse::fade(hit.0))
            .unwrap_or(0.0);
        if let Some(bottom) = area.height.checked_sub(1) {
            let kx = ((w / 2.0) + wx).clamp(0.0, w - 1.0) as u16;
            paint(buf, area, kx, bottom, glow, self.pulse.hue);
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

    #[test]
    fn every_scene_renders_any_size_after_hits() {
        for (w, h) in [(0, 0), (1, 1), (3, 2), (46, 10), (200, 60)] {
            for mut scene in all() {
                for event in [Event::Kick, Event::Chord(3), Event::Beat(7.5), Event::Kick] {
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
}
