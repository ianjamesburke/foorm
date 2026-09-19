//! Scenes: each gives incoming events a shape. A scene owns only animation
//! state; it never touches the network or the terminal directly.
//!
//! The kick is the anchor of every scene: same place, same motion, every hit,
//! with at most one cell of analog wobble so it reads as played, not looped.
//! Its size follows the hit's level, so an inaudible kick is invisible too.
//! Ambient motion follows each voice's level through `Sensitivity`: silence
//! is still, and every layer nooise plays has something on screen.

use std::f32::consts::TAU;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::osc::{Event, Voice};

pub trait Scene {
    fn name(&self) -> &'static str;
    fn on_event(&mut self, event: &Event);
    /// Replace the sensitivity table; the tune panel calls this on every edit.
    fn tune(&mut self, sensitivity: Sensitivity);
    fn tick(&mut self, dt: f32);
    fn render(&self, area: Rect, buf: &mut Buffer);
}

pub fn all() -> Vec<Box<dyn Scene>> {
    vec![
        Box::new(Fluid::default()),
        Box::new(System::default()),
        Box::new(Binary::default()),
        Box::new(Tide::default()),
        Box::new(Rain::default()),
        Box::new(Estuary::default()),
        Box::new(Loom::default()),
        Box::new(Reef::default()),
        Box::new(City::default()),
        Box::new(Atlas::default()),
    ]
}

const GRADIENT: &[char] = &[' ', '·', '∙', '•', '●', '◉', '⬤'];
const HUES: [f32; 5] = [205.0, 270.0, 325.0, 158.0, 38.0];

/// RMS at which a voice counts as fully driven. Calibrated from nooise's
/// built-in songs (`song_level_profile` in nooise's `src/fluid/osc.rs`,
/// 2026-09-18, songs 1-16): roughly each voice's p90 while active. Lead was
/// silent in every profiled song and carries a guess. The future per-scene
/// tune panel (`t`) edits this table live and prints it on quit so a tuned
/// session can be pasted back here; nothing else in a scene hard-codes a level.
#[derive(Clone, Copy, PartialEq)]
pub struct Sensitivity {
    /// Indexed by `Voice`.
    voices: [f32; Voice::ALL.len()],
    master: f32,
}

impl Default for Sensitivity {
    fn default() -> Self {
        Self {
            voices: [
                0.06,  // pad
                0.002, // perc
                0.2,   // bass
                0.05,  // kick
                0.04,  // tonal
                0.012, // clap
                0.05,  // arp
                0.05,  // lead (uncalibrated)
            ],
            master: 0.08,
        }
    }
}

impl Sensitivity {
    /// Editable rows: one per voice, then master.
    pub const ROWS: usize = Voice::ALL.len() + 1;

    pub fn label(row: usize) -> &'static str {
        Voice::ALL.get(row).map(|v| v.name()).unwrap_or("master")
    }

    pub fn get(&self, row: usize) -> f32 {
        self.voices.get(row).copied().unwrap_or(self.master)
    }

    pub fn set(&mut self, row: usize, full: f32) {
        let full = full.clamp(1e-4, 1.0);
        match self.voices.get_mut(row) {
            Some(v) => *v = full,
            None => self.master = full,
        }
    }

    /// 0..1 drive for a level: square-root law so quiet material still
    /// registers, saturating at the calibrated full level.
    pub fn drive(full: f32, level: f32) -> f32 {
        (level / full.max(1e-6)).clamp(0.0, 1.0).sqrt()
    }

    /// The table as it would be written in `Default`, for pasting back.
    pub fn literal(&self) -> String {
        let voices = Voice::ALL
            .iter()
            .map(|v| format!("{:.4}, // {}", self.voices[*v as usize], v.name()))
            .collect::<Vec<_>>()
            .join("\n        ");
        format!(
            "Sensitivity {{\n    voices: [\n        {voices}\n    ],\n    master: {:.4},\n}}",
            self.master
        )
    }
}

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
    sensitivity: Sensitivity,
    /// Master RMS as last reported.
    level: f32,
    /// Per-voice RMS as last reported.
    voice_levels: [f32; Voice::ALL.len()],
    /// `level` followed with a fast rise and slow fall, 0..1: what ambient
    /// motion runs on.
    drive: f32,
    /// Per-voice followed drives, same law as `drive`.
    voice_drives: [f32; Voice::ALL.len()],
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
            Event::VoiceLevel(voice, l) => self.voice_levels[*voice as usize] = *l,
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
        Self::follow(
            &mut self.drive,
            Sensitivity::drive(self.sensitivity.master, self.level),
            dt,
        );
        for (i, drive) in self.voice_drives.iter_mut().enumerate() {
            let target = Sensitivity::drive(self.sensitivity.voices[i], self.voice_levels[i]);
            Self::follow(drive, target, dt);
        }
    }

    /// Fast rise, slow fall.
    fn follow(drive: &mut f32, target: f32, dt: f32) {
        let rate = if target > *drive { 12.0 } else { 2.0 };
        *drive += (target - *drive) * (rate * dt).min(1.0);
    }

    fn voice(&self, voice: Voice) -> f32 {
        self.voice_drives[voice as usize]
    }

    /// Strongest of the transient voices: what a rhythm cursor should follow.
    fn rhythm(&self) -> f32 {
        [Voice::Perc, Voice::Clap]
            .into_iter()
            .map(|v| self.voice(v))
            .fold(0.0, f32::max)
    }

    /// Strongest of the melodic voices above the pad.
    fn melody(&self) -> f32 {
        [Voice::Tonal, Voice::Arp, Voice::Lead]
            .into_iter()
            .map(|v| self.voice(v))
            .fold(0.0, f32::max)
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

/// A shaded disc centred at (cx, cy) in cells with radius r in rows; cells
/// are ~2:1 so the disc is twice as wide as it is tall in cells.
fn disc(buf: &mut Buffer, area: Rect, cx: f32, cy: f32, r: f32, v: f32, hue: f32) {
    let r = r.max(0.3);
    let (x0, x1) = (
        (cx - r * 2.0 - 1.0).max(0.0) as u16,
        (cx + r * 2.0 + 1.0) as u16,
    );
    let (y0, y1) = ((cy - r - 1.0).max(0.0) as u16, (cy + r + 1.0) as u16);
    for y in y0..=y1.min(area.height.saturating_sub(1)) {
        for x in x0..=x1.min(area.width.saturating_sub(1)) {
            let dx = (x as f32 - cx) / 2.0;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt() / r;
            if d <= 1.0 {
                // lit from the upper left, dark at the limb
                let lit = 1.0 - 0.55 * d * d + 0.15 * (-dx - dy) / r;
                paint(buf, area, x, y, v * lit.clamp(0.2, 1.0), hue);
            }
        }
    }
}

/// Deterministic star per cell: most cells are empty; a few carry a faint
/// star that twinkles with `shimmer`.
fn starfield(buf: &mut Buffer, area: Rect, hue: f32, shimmer: f32, t: f32) {
    for y in 0..area.height {
        for x in 0..area.width {
            let mut z = (x as u64) << 32 | y as u64 | 0xA5A5;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            let r = (z ^ (z >> 31)) % 1000;
            let v = if r < 12 {
                0.12 + 0.25 * shimmer * ((t * 3.0 + r as f32).sin() * 0.5 + 0.5)
            } else {
                0.0
            };
            paint(buf, area, x, y, v, hue);
        }
    }
}

/// A ring expanding from the kick's anchor at the bottom centre, in
/// normalised screen space; 0..1 per cell.
fn kick_ring(hits: &[Hit], nx: f32, ny: f32, w: f32, h: f32) -> f32 {
    let mut v = 0.0;
    for hit in hits {
        let (wx, wy) = hit.wobble;
        let dx = (nx - 0.5 - wx / w) * 2.0;
        let dy = ny - 1.0 - wy / h;
        let dist = (dx * dx + dy * dy).sqrt();
        let front = hit.age * 0.6;
        let ring = (-((dist - front) * 9.0).powi(2)).exp();
        v += ring * Pulse::fade(hit.age) * hit.level;
    }
    v
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
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.pulse.sensitivity = sensitivity;
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        self.phase += dt * 0.6 * self.pulse.voice(Voice::Pad).max(self.pulse.drive * 0.5);
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let z = self.phase;
        let hue = self.pulse.hue();
        let pad = self.pulse.voice(Voice::Pad);
        let bass = self.pulse.voice(Voice::Bass);
        let melody = self.pulse.melody();
        let amp = 0.2 + 0.8 * pad.max(self.pulse.drive * 0.6);
        for y in 0..area.height {
            for x in 0..area.width {
                let nx = x as f32 / w;
                let ny = y as f32 / h;
                let mut v = ((nx * 6.0 + z).sin() * (ny * 5.0 - z * 0.7).cos()
                    + ((nx * 3.3 - ny * 4.1) + z * 1.3).sin() * 0.7
                    + ((nx + ny) * 7.5 + (z * 0.9).sin() * 2.0).cos() * 0.5)
                    * amp
                    // melody: fine shimmer across the surface
                    + ((nx * 23.0 + z * 4.0).sin() * (ny * 17.0 - z * 3.0).cos()) * melody * 0.8
                    // bass: the floor swells up from the bottom
                    + (ny * ny) * bass * 2.5;
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

// ------------------------------------------------------------ System

/// A solar system. The pad is the sun, swelling with its level; every other
/// voice is a planet on its own orbit, sized by its drive and hurried by it,
/// the melodic ones carrying a moon. Planets leave trails, so a voice that
/// just played is still visible as a fading arc. The kick sends a shockwave
/// up from the bottom centre through the whole field.
pub struct System {
    pulse: Pulse,
    t: f32,
    planets: Vec<Planet>,
}

struct Planet {
    voice: Voice,
    orbit: f32,
    tilt: f32,
    angle: f32,
    moon: bool,
}

impl Default for System {
    fn default() -> Self {
        let bodies = [
            (Voice::Bass, false),
            (Voice::Tonal, true),
            (Voice::Perc, false),
            (Voice::Arp, true),
            (Voice::Clap, false),
            (Voice::Lead, true),
            (Voice::Kick, false),
        ];
        let planets = bodies
            .into_iter()
            .enumerate()
            .map(|(i, (voice, moon))| Planet {
                voice,
                orbit: 0.12 + i as f32 * 0.065,
                tilt: 0.55 + (i % 3) as f32 * 0.08,
                angle: i as f32 * 2.1,
                moon,
            })
            .collect();
        Self {
            pulse: Pulse::default(),
            t: 0.0,
            planets,
        }
    }
}

impl System {
    const TRAIL: usize = 28;
}

impl Scene for System {
    fn name(&self) -> &'static str {
        "system"
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.pulse.sensitivity = sensitivity;
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        self.t += dt;
        for p in &mut self.planets {
            let drive = self.pulse.voice(p.voice);
            // inner planets are quicker; a playing voice hurries its planet
            p.angle += dt * (0.12 + 0.9 * drive) / p.orbit.sqrt();
        }
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let hue = self.pulse.hue();
        let pad = self.pulse.voice(Voice::Pad);
        let melody = self.pulse.melody();
        starfield(buf, area, hue, melody, self.t);
        // kick shockwave under the bodies, so it never hides them
        if !self.pulse.live.is_empty() {
            for y in 0..area.height {
                for x in 0..area.width {
                    let v = kick_ring(&self.pulse.live, x as f32 / w, y as f32 / h, w, h);
                    if v > 0.1 {
                        paint(buf, area, x, y, v * 0.6, hue - 15.0);
                    }
                }
            }
        }
        let (cx, cy) = (w / 2.0, h / 2.0);
        // orbits as faint dotted ellipses
        for p in &self.planets {
            for k in 0..64 {
                let a = k as f32 / 64.0 * TAU;
                let x = cx + a.cos() * p.orbit * h * 2.0;
                let y = cy + a.sin() * p.orbit * h * p.tilt;
                if x >= 0.0 && y >= 0.0 {
                    paint(buf, area, x as u16, y as u16, 0.1, hue);
                }
            }
        }
        // the sun
        disc(
            buf,
            area,
            cx,
            cy,
            2.0 + 2.5 * pad.max(self.pulse.drive * 0.5),
            0.55 + 0.45 * pad,
            hue + 20.0,
        );
        for (i, p) in self.planets.iter().enumerate() {
            let drive = self.pulse.voice(p.voice);
            let x = cx + p.angle.cos() * p.orbit * h * 2.0;
            let y = cy + p.angle.sin() * p.orbit * h * p.tilt;
            let phue = hue + (i as f32 - 3.0) * 22.0;
            // trail: recent angles, fading back
            for k in 1..Self::TRAIL {
                let a = p.angle - k as f32 * 0.05;
                let tx = cx + a.cos() * p.orbit * h * 2.0;
                let ty = cy + a.sin() * p.orbit * h * p.tilt;
                if tx >= 0.0 && ty >= 0.0 {
                    let v = drive * (1.0 - k as f32 / Self::TRAIL as f32) * 0.5;
                    paint(buf, area, tx as u16, ty as u16, v, phue);
                }
            }
            disc(
                buf,
                area,
                x,
                y,
                0.9 + 2.0 * drive,
                0.35 + 0.65 * drive,
                phue,
            );
            if p.moon && drive > 0.05 {
                let ma = p.angle * 5.0;
                let mx = x + ma.cos() * (3.0 + 3.0 * drive) * 2.0;
                let my = y + ma.sin() * (3.0 + 3.0 * drive) * 0.6;
                if mx >= 0.0 && my >= 0.0 {
                    paint(
                        buf,
                        area,
                        mx as u16,
                        my as u16,
                        0.4 + 0.6 * drive,
                        phue + 30.0,
                    );
                }
            }
        }
    }
}

// ------------------------------------------------------------ Binary

/// Two stars circling each other, each dragging a ring of particles. Bass
/// pulls the pair apart, the pad spins the system, melody brightens the
/// rings, rhythm shakes the particles. The kick shoves the particles nearest
/// the bottom outward and they spring back onto their rings.
pub struct Binary {
    pulse: Pulse,
    t: f32,
    /// Angle of the pair about the shared centre.
    angle: f32,
    /// (star index, angle, radius, radial velocity) per particle
    particles: Vec<(usize, f32, f32, f32)>,
}

impl Default for Binary {
    fn default() -> Self {
        let particles = (0..120)
            .map(|i| {
                let f = i as f32 / 120.0;
                (i % 2, f * TAU * 2.0, 0.06 + (f * 7.0).fract() * 0.12, 0.0)
            })
            .collect();
        Self {
            pulse: Pulse::default(),
            t: 0.0,
            angle: 0.0,
            particles,
        }
    }
}

impl Binary {
    /// Star centres in normalised height units around the screen centre.
    fn stars(&self) -> [(f32, f32); 2] {
        let sep = 0.14 + 0.16 * self.pulse.voice(Voice::Bass);
        let (c, s) = (self.angle.cos() * sep, self.angle.sin() * sep * 0.5);
        [(c, s), (-c, -s)]
    }
    fn rest(&self) -> f32 {
        0.09 + 0.05 * self.pulse.melody()
    }
}

impl Scene for Binary {
    fn name(&self) -> &'static str {
        "binary"
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.pulse.sensitivity = sensitivity;
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
        if let Event::Kick(level) = event {
            let (wx, _) = wobble(self.pulse.hits);
            for p in &mut self.particles {
                // push hardest on the particles nearest the bottom
                let bottom = (p.1 + wx * 0.05).sin().max(0.0);
                p.3 += 0.5 * bottom * level.clamp(0.0, 1.0);
            }
        }
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        self.t += dt;
        let pad = self.pulse.voice(Voice::Pad).max(self.pulse.drive * 0.6);
        self.angle += dt * (0.1 + 0.5 * pad);
        let rest = self.rest();
        let shake = self.pulse.rhythm();
        for p in &mut self.particles {
            p.1 += dt * (1.2 + 3.0 * pad) * (0.1 / p.2.max(0.03));
            p.2 += p.3 * dt;
            p.3 -= (p.2 - rest - (p.1 * 7.0).sin() * 0.03 * shake) * 6.0 * dt;
            p.3 *= 1.0 - 3.0 * dt;
        }
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let hue = self.pulse.hue();
        let pad = self.pulse.voice(Voice::Pad);
        let melody = self.pulse.melody();
        starfield(buf, area, hue, melody, self.t);
        let stars = self.stars();
        let to_cells = |(ux, uy): (f32, f32)| (w / 2.0 + ux * h * 2.0, h / 2.0 + uy * h);
        for &(star, angle, radius, vel) in &self.particles {
            let (sx, sy) = stars[star];
            let (x, y) = to_cells((sx + angle.cos() * radius, sy + angle.sin() * radius * 0.6));
            if x < 0.0 || y < 0.0 || x >= w || y >= h {
                continue;
            }
            let v = (0.25 + 0.5 * melody + vel.abs() * 2.0).clamp(0.0, 1.0);
            let shue = hue + if star == 0 { 25.0 } else { -25.0 };
            paint(buf, area, x as u16, y as u16, v, shue + vel * 80.0);
        }
        for (i, &s) in stars.iter().enumerate() {
            let (x, y) = to_cells(s);
            let shue = hue + if i == 0 { 25.0 } else { -25.0 };
            disc(buf, area, x, y, 0.8 + 1.4 * pad, 0.6 + 0.4 * pad, shue);
        }
        if let (Some(hit), Some(bottom)) = (self.pulse.anchor(), area.height.checked_sub(1)) {
            let kx = ((w / 2.0) + hit.wobble.0).clamp(0.0, w - 1.0) as u16;
            paint(buf, area, kx, bottom, Pulse::fade(hit.age) * hit.level, hue);
            for x in 0..area.width {
                let v = kick_ring(&self.pulse.live, x as f32 / w, 1.0, w, h);
                paint(
                    buf,
                    area,
                    x,
                    bottom,
                    v.max(if x == kx { hit.level } else { 0.0 }),
                    hue,
                );
            }
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
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.pulse.sensitivity = sensitivity;
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        self.phase += dt * 1.2 * self.pulse.voice(Voice::Pad).max(self.pulse.drive * 0.5);
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let hue = self.pulse.hue();
        let pad = self.pulse.voice(Voice::Pad);
        let bass = self.pulse.voice(Voice::Bass);
        let melody = self.pulse.melody();
        let height = 0.08 + 0.35 * bass.max(self.pulse.drive * 0.5);
        for x in 0..area.width {
            let nx = x as f32 / w;
            // surface height in 0..1 from the bottom
            let mut surface = height
                + pad
                    * 0.08
                    * ((nx * 9.0 + self.phase).sin() + (nx * 4.0 - self.phase * 0.6).cos())
                + melody * 0.03 * (nx * 31.0 + self.phase * 5.0).sin();
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
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.pulse.sensitivity = sensitivity;
    }
    fn on_event(&mut self, event: &Event) {
        self.pulse.on_event(event);
    }
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        let rate = self
            .pulse
            .rhythm()
            .max(self.pulse.melody())
            .max(self.pulse.drive * 0.4);
        self.due += dt * 50.0 * rate;
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

// ------------------------------------------------------------ Layer drawing

/// Independent clocks keep a solo voice from moving another voice's shapes.
#[derive(Default)]
struct Layers {
    pulse: Pulse,
    phases: [f32; Voice::ALL.len()],
    ambient: f32,
}

impl Layers {
    fn tick(&mut self, dt: f32) {
        self.pulse.tick(dt);
        for voice in Voice::ALL {
            self.phases[voice as usize] += dt * self.pulse.voice(voice);
        }
        self.ambient += dt * self.pulse.drive;
    }

    fn phase(&self, voice: Voice) -> f32 {
        self.phases[voice as usize]
    }

    fn background(&self, area: Rect, buf: &mut Buffer) {
        starfield(buf, area, self.pulse.hue(), self.pulse.drive, self.ambient);
    }

    /// All five scenes share the same hit trajectory, independent of the mix.
    /// The sustained kick meter is a separate, stationary foot at the anchor.
    fn kick(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let hue = self.pulse.hue();
        for y in 0..area.height {
            for x in 0..area.width {
                let v = kick_ring(&self.pulse.live, x as f32 / w, y as f32 / h, w, h);
                Ink::new(v, 0.0, '~').draw(buf, area, x, y, hue);
            }
        }
        let drive = self.pulse.voice(Voice::Kick);
        if drive > 0.0 {
            disc(
                buf,
                area,
                w * 0.5,
                h - 1.0,
                (h * 0.16 + 1.0) * drive,
                drive,
                hue,
            );
        }
        for hit in &self.pulse.live {
            let v = hit.level * Pulse::fade(hit.age);
            if v > 0.0 {
                disc(
                    buf,
                    area,
                    w * 0.5 + hit.wobble.0,
                    h - 1.0 + hit.wobble.1,
                    (1.0 + h * 0.12 * Pulse::fade(hit.age)) * hit.level,
                    v,
                    hue,
                );
            }
        }
    }
}

/// Brightest layer wins at crossings; a silent layer cannot erase another.
#[derive(Default)]
struct Ink {
    value: f32,
    offset: f32,
    glyph: Option<char>,
}

impl Ink {
    fn new(value: f32, offset: f32, glyph: char) -> Self {
        Self {
            value,
            offset,
            glyph: Some(glyph),
        }
    }

    fn layer(&mut self, value: f32, offset: f32, glyph: char) {
        if value > self.value {
            *self = Self::new(value, offset, glyph);
        }
    }

    fn draw(&self, buf: &mut Buffer, area: Rect, x: u16, y: u16, hue: f32) {
        // Match paint's first visible glyph, a raster limit, not an RMS gate.
        if (self.value.clamp(0.0, 1.0) * (GRADIENT.len() - 1) as f32).round() == 0.0 {
            return;
        }
        paint(buf, area, x, y, self.value, hue + self.offset);
        if x < area.width
            && y < area.height
            && let Some(glyph) = self.glyph
        {
            buf[(area.x + x, area.y + y)].set_char(glyph);
        }
    }
}

/// Soft coverage for a stroke of a given half-width in the caller's units.
fn ridge(distance: f32, width: f32) -> f32 {
    (1.0 - distance.abs() / width.max(f32::EPSILON)).max(0.0)
}

fn field(area: Rect, buf: &mut Buffer, hue: f32, sample: impl Fn(f32, f32) -> Ink) {
    for y in 0..area.height {
        for x in 0..area.width {
            sample(x as f32, y as f32).draw(buf, area, x, y, hue);
        }
    }
}

// ------------------------------------------------------------ Estuary

/// Aurora, dunes, weather, moon, birds and a comet occupy separate depths.
#[derive(Default)]
pub struct Estuary {
    layers: Layers,
}

impl Scene for Estuary {
    fn name(&self) -> &'static str {
        "estuary"
    }
    fn on_event(&mut self, event: &Event) {
        self.layers.pulse.on_event(event);
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.layers.pulse.sensitivity = sensitivity;
    }
    fn tick(&mut self, dt: f32) {
        self.layers.tick(dt);
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let l = &self.layers;
        let p = &l.pulse;
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        l.background(area, buf);
        field(area, buf, p.hue(), |x, y| {
            let (nx, ny) = (x / w, y / h);
            let mut ink = Ink::default();
            let pad = p.voice(Voice::Pad);
            let curtain = 0.16 + 0.07 * (nx * 9.0 + l.phase(Voice::Pad)).sin();
            let folds = 0.65 + 0.35 * (nx * 45.0 - l.phase(Voice::Pad)).sin().abs();
            ink.layer(
                pad * ridge(ny - curtain, 0.08 + pad * 0.1) * folds,
                -30.0,
                '|',
            );

            let bass = p.voice(Voice::Bass);
            for depth in 0..3 {
                let d = depth as f32;
                let ground = 0.78 + d * 0.075 - bass * 0.12
                    + (nx * (7.0 + d * 3.0) + l.phase(Voice::Bass) * 0.7 + d).sin() * 0.045;
                if ny >= ground {
                    let contour = ((ny - ground) * h).rem_euclid(3.0);
                    ink.layer(
                        bass * (0.4 + d * 0.15),
                        35.0 + d * 12.0,
                        if contour < 1.0 { '=' } else { ':' },
                    );
                }
            }

            // Weather occupies opposite banks, so clap and perc remain distinct.
            if nx > 0.57 && ny < 0.76 {
                let streak = (nx * 22.0 + ny * 3.0).rem_euclid(1.0);
                let fall = (ny * 6.0 - l.phase(Voice::Perc) * 3.0 + nx * 4.0).rem_euclid(1.0);
                ink.layer(
                    p.voice(Voice::Perc) * ridge(streak - 0.5, 0.16) * ridge(fall - 0.5, 0.4),
                    -65.0,
                    '/',
                );
            }
            if (0.22..0.72).contains(&ny) {
                let zig = ((ny * 18.0).floor() + l.phase(Voice::Clap) * 2.0).sin();
                for branch in [0.13, 0.28] {
                    ink.layer(
                        p.voice(Voice::Clap)
                            * ridge(
                                nx - branch - zig * 0.035,
                                0.018 + 0.016 * p.voice(Voice::Clap),
                            ),
                        100.0,
                        '#',
                    );
                }
            }

            let tonal = p.voice(Voice::Tonal);
            let radius = h.min(w / 2.0) * (0.055 + tonal * 0.065);
            let dx = (x - w * 0.76) / 2.0;
            let dy = y - h * 0.24;
            let distance = dx.hypot(dy);
            ink.layer(tonal * ridge(distance, radius), 65.0, 'O');
            ink.layer(tonal * 0.6 * ridge(distance - radius, 1.1), 65.0, '+');

            let arp = p.voice(Voice::Arp);
            for bird in 0..4 {
                let b = bird as f32;
                let bx = (0.12 + b * 0.23 + l.phase(Voice::Arp) * 0.035).rem_euclid(1.0);
                let dx = (nx - bx).abs();
                if dx < 0.055 {
                    let wing =
                        0.48 + b.sin() * 0.055 - dx * (1.0 + l.phase(Voice::Arp).sin() * 0.4);
                    ink.layer(
                        arp * ridge(ny - wing, 0.028 + arp * 0.025),
                        -100.0,
                        if nx < bx { '\\' } else { '/' },
                    );
                }
            }
            let lead = p.voice(Voice::Lead);
            let ribbon = 0.34 + 0.055 * (nx * 9.0 - l.phase(Voice::Lead) * 1.7).sin();
            let segments = 0.55 + 0.45 * (nx * 17.0 - l.phase(Voice::Lead) * 3.0).cos().abs();
            ink.layer(
                lead * ridge(ny - ribbon, 0.025 + lead * 0.035) * segments,
                140.0,
                '>',
            );
            ink
        });
        l.kick(area, buf);
    }
}

/// Labels are clipped to the scene rectangle, including offset rectangles.
fn caption(buf: &mut Buffer, area: Rect, at: (u16, u16), text: &str, hue: f32) {
    for (i, c) in text.chars().enumerate() {
        let x = usize::from(at.0) + i;
        if x >= usize::from(area.width) || at.1 >= area.height {
            break;
        }
        buf[(area.x + x as u16, area.y + at.1)]
            .set_char(c)
            .set_style(Style::default().fg(hsv(hue, 0.5, 0.65)));
    }
}

const INSTRUMENTS: [Voice; 7] = [
    Voice::Pad,
    Voice::Bass,
    Voice::Perc,
    Voice::Clap,
    Voice::Tonal,
    Voice::Arp,
    Voice::Lead,
];

// ------------------------------------------------------------ Loom

/// Seven labelled warp ribbons, each with its own weave and clock.
#[derive(Default)]
pub struct Loom {
    layers: Layers,
}

impl Scene for Loom {
    fn name(&self) -> &'static str {
        "loom"
    }
    fn on_event(&mut self, event: &Event) {
        self.layers.pulse.on_event(event);
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.layers.pulse.sensitivity = sensitivity;
    }
    fn tick(&mut self, dt: f32) {
        self.layers.tick(dt);
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let l = &self.layers;
        let p = &l.pulse;
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let lane = w / INSTRUMENTS.len() as f32;
        l.background(area, buf);
        field(area, buf, p.hue(), |x, y| {
            let mut ink = Ink::default();
            if y < 3.0 || y >= h - 2.0 {
                return ink;
            }
            let i = ((x / lane) as usize).min(INSTRUMENTS.len() - 1);
            let voice = INSTRUMENTS[i];
            let d = p.voice(voice);
            let t = l.phase(voice);
            let u = (x / lane).fract() * 2.0 - 1.0;
            let v = y / h;
            let a = v * TAU * 2.0 - t * 1.4;
            let offset = i as f32 * 32.0 - 90.0;
            let (coverage, glyph) = match voice {
                Voice::Pad => (
                    ridge(u - a.sin() * 0.25, 0.4 + d * 0.4)
                        * (0.65 + 0.35 * (u * 10.0 + t).cos().abs()),
                    '|',
                ),
                Voice::Bass => (
                    ridge(u, 0.3 + d * 0.6) * (0.5 + 0.5 * (v * 30.0 - t).sin().abs()),
                    '=',
                ),
                Voice::Perc => {
                    let bead = (v * 6.0 - t).rem_euclid(1.0) * 2.0 - 1.0;
                    ((1.0 - (u.abs() + bead.abs()) / (0.5 + d)).max(0.0), 'o')
                }
                Voice::Clap => {
                    let bar = (v * 5.0 - t * 1.7).rem_euclid(1.0);
                    (ridge(bar - 0.5, 0.2 + d * 0.2) * ridge(u, 1.2), '#')
                }
                Voice::Tonal => (
                    ridge(u - a.sin() * 0.55, 0.3 + d * 0.3)
                        .max(ridge(u + a.sin() * 0.55, 0.16) * 0.55),
                    '~',
                ),
                Voice::Arp => {
                    let step = ((v * 9.0 - t * 2.0).floor()).rem_euclid(4.0) / 3.0;
                    (ridge(u - (step - 0.5), 0.3 + d * 0.25), '+')
                }
                Voice::Lead => (
                    ridge(u - a.cos() * 0.5, 0.22 + d * 0.3)
                        .max(ridge(u + a.cos() * 0.5, 0.22 + d * 0.3)),
                    '/',
                ),
                Voice::Kick => (0.0, ' '),
            };
            ink.layer(d * coverage, offset, glyph);
            ink
        });
        for (i, voice) in INSTRUMENTS.iter().enumerate() {
            let x = (lane * (i as f32 + 0.5) - voice.name().len() as f32 / 2.0).max(0.0) as u16;
            caption(
                buf,
                area,
                (x, 1),
                voice.name(),
                p.hue() + i as f32 * 32.0 - 90.0,
            );
        }
        l.kick(area, buf);
    }
}

// ------------------------------------------------------------ Reef

/// A living reef with large silhouettes at different depths.
#[derive(Default)]
pub struct Reef {
    layers: Layers,
}

impl Scene for Reef {
    fn name(&self) -> &'static str {
        "reef"
    }
    fn on_event(&mut self, event: &Event) {
        self.layers.pulse.on_event(event);
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.layers.pulse.sensitivity = sensitivity;
    }
    fn tick(&mut self, dt: f32) {
        self.layers.tick(dt);
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let l = &self.layers;
        let p = &l.pulse;
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let scale = h.min(w / 2.0);
        l.background(area, buf);
        field(area, buf, p.hue(), |x, y| {
            let (nx, ny) = (x / w, y / h);
            let mut ink = Ink::default();
            let pad = p.voice(Voice::Pad);
            for stalk in 0..3 {
                let s = stalk as f32;
                if ny > 0.22 + s * 0.09 && ny < 0.94 {
                    let stem = 0.055
                        + s * 0.085
                        + (ny * 9.0 + l.phase(Voice::Pad) + s).sin() * 0.035 * (1.0 - ny);
                    let leaves = (ny * 22.0 + s * 2.0).sin().powi(2);
                    ink.layer(pad * ridge(nx - stem, 0.012 + leaves * 0.042), -55.0, '|');
                }
            }
            let bass = p.voice(Voice::Bass);
            if ny > 0.71 {
                for root in [0.32, 0.55, 0.78] {
                    let growth = (0.96 - ny).max(0.0);
                    for branch in [-1.0, 0.0, 1.0] {
                        let stem = root
                            + branch * growth * 0.42
                            + (ny * 24.0 + l.phase(Voice::Bass)).sin() * 0.014;
                        ink.layer(bass * ridge(nx - stem, 0.018 + bass * 0.014), 55.0, '#');
                    }
                }
            }
            let perc = p.voice(Voice::Perc);
            for bubble in 0..5 {
                let b = bubble as f32;
                let cx = w * (0.83 + (b * 2.3).sin() * 0.09);
                let cy = h * (0.92 - (b * 0.19 + l.phase(Voice::Perc) * 0.13).rem_euclid(0.85));
                let distance = ((x - cx) / 2.0).hypot(y - cy);
                let radius = scale * (0.025 + perc * 0.035);
                ink.layer(perc * ridge(distance - radius, 0.8), -100.0, 'o');
            }
            let clap = p.voice(Voice::Clap);
            let dx = (x - w * 0.77) / 2.0;
            let dy = y - h * 0.73;
            let distance = dx.hypot(dy);
            let angle = dy.atan2(dx);
            if dy < 0.0 {
                let ribs = 0.4 + 0.6 * (angle * 9.0 + l.phase(Voice::Clap) * 2.0).cos().abs();
                ink.layer(
                    clap * ridge(distance, scale * (0.1 + clap * 0.16)) * ribs,
                    110.0,
                    '/',
                );
            }
            let tonal = p.voice(Voice::Tonal);
            let dx = (x - w * 0.41) / 2.0;
            let dy = y - h * 0.69;
            let angle = dy.atan2(dx);
            let petal = 0.75 + 0.25 * (angle * 7.0 + l.phase(Voice::Tonal)).cos();
            let radius = scale * (0.06 + tonal * 0.1) * petal;
            ink.layer(tonal * ridge(dx.hypot(dy), radius), 10.0, '@');
            ink.layer(tonal * 0.7 * ridge(dx.hypot(dy) - radius, 0.7), 10.0, '+');

            let arp = p.voice(Voice::Arp);
            for fish in 0..3 {
                let f = fish as f32;
                let cx = (0.31 + f * 0.18 + l.phase(Voice::Arp) * 0.045).rem_euclid(0.65) + 0.15;
                let cy = 0.38 + f * 0.105;
                let dx = (nx - cx) / (0.035 + arp * 0.055);
                let dy = (ny - cy) / (0.025 + arp * 0.055);
                ink.layer(
                    arp * (1.0 - dx.abs() * 0.55 - dy.abs()).max(0.0),
                    -145.0,
                    '>',
                );
                if (-1.7..-0.7).contains(&dx) {
                    ink.layer(arp * ridge(dy, (dx + 0.7).abs()), -145.0, '<');
                }
            }
            let lead = p.voice(Voice::Lead);
            let cx = 0.5 + l.phase(Voice::Lead).sin() * 0.07;
            let dx = (nx - cx) / (0.12 + lead * 0.17);
            let wing = 0.19 + dx.abs() * 0.06 * (l.phase(Voice::Lead) * 1.6).cos();
            let body = ridge(ny - wing, (0.035 + lead * 0.09) * (1.0 - dx.abs()).max(0.0));
            if dx.abs() < 1.0 {
                ink.layer(lead * body, 155.0, '=');
            }
            if (0.19..0.38).contains(&ny) {
                ink.layer(
                    lead * ridge(
                        nx - cx - (ny * 20.0 + l.phase(Voice::Lead)).sin() * 0.015,
                        0.012,
                    ),
                    155.0,
                    '~',
                );
            }
            ink
        });
        l.kick(area, buf);
    }
}

// ------------------------------------------------------------ City

/// A night skyline: architecture, traffic and signals have separate owners.
#[derive(Default)]
pub struct City {
    layers: Layers,
}

impl Scene for City {
    fn name(&self) -> &'static str {
        "city"
    }
    fn on_event(&mut self, event: &Event) {
        self.layers.pulse.on_event(event);
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.layers.pulse.sensitivity = sensitivity;
    }
    fn tick(&mut self, dt: f32) {
        self.layers.tick(dt);
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let l = &self.layers;
        let p = &l.pulse;
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let scale = h.min(w / 2.0);
        l.background(area, buf);
        field(area, buf, p.hue(), |x, y| {
            let (nx, ny) = (x / w, y / h);
            let mut ink = Ink::default();
            let pad = p.voice(Voice::Pad);
            if ny < 0.73 {
                for origin in [0.08, 0.92] {
                    let reach = 0.73 - ny;
                    let sweep = (l.phase(Voice::Pad) * 0.6 + origin * 5.0).sin() * 0.4;
                    let beam = ridge(nx - origin - reach * sweep, reach * (0.09 + pad * 0.12));
                    ink.layer(pad * beam * 0.7, -50.0, ':');
                }
            }
            let bass = p.voice(Voice::Bass);
            if (0.3..0.68).contains(&nx) && ny < 0.76 {
                let block = ((nx - 0.3) * 24.0).floor();
                let roof = 0.71 - bass * (0.23 + 0.21 * (block * 2.4).sin().abs())
                    + (l.phase(Voice::Bass) + block).sin() * bass * 0.025;
                if ny > roof {
                    let edge = ((nx - 0.3) * 24.0).fract();
                    ink.layer(
                        bass * (0.6 + 0.3 * edge),
                        45.0,
                        if edge < 0.16 {
                            '|'
                        } else if (y as u16).is_multiple_of(3) {
                            '='
                        } else {
                            '#'
                        },
                    );
                }
            }
            let perc = p.voice(Voice::Perc);
            for lane in 0..3 {
                let lane = lane as f32;
                let direction = if lane == 1.0 { -1.0 } else { 1.0 };
                let car = (nx * 5.0 - l.phase(Voice::Perc) * direction * 1.8 + lane * 0.6)
                    .rem_euclid(1.0);
                let road = ridge(ny - (0.8 + lane * 0.065), 0.032);
                ink.layer(perc * ridge(car - 0.5, 0.3) * road, -90.0, '=');
                if car > 0.62 {
                    ink.layer(perc * road, -90.0, '>');
                }
            }
            let clap = p.voice(Voice::Clap);
            for tower in [0.08, 0.92] {
                let dx = (x - w * tower) / 2.0;
                let dy = y - h * 0.4;
                let r = scale * (0.04 + clap * 0.08);
                let rays = (dy.atan2(dx) * 4.0 + l.phase(Voice::Clap) * 2.0)
                    .cos()
                    .abs();
                ink.layer(
                    clap * ridge(dx.hypot(dy) - r, 1.0) * (0.5 + rays * 0.5),
                    115.0,
                    '*',
                );
                if (0.4..0.74).contains(&ny) {
                    ink.layer(clap * ridge(nx - tower, 0.017), 115.0, '!');
                }
            }
            let tonal = p.voice(Voice::Tonal);
            let dx = (x - w * 0.21) / 2.0;
            let dy = y - h * 0.42;
            let r = scale * (0.065 + tonal * 0.08);
            let distance = dx.hypot(dy);
            ink.layer(tonal * ridge(distance - r, 1.0), 0.0, 'O');
            if distance < r {
                let a = l.phase(Voice::Tonal) * 1.5;
                let hand = ridge(dx * a.sin() - dy * a.cos(), 0.85);
                ink.layer(tonal * hand, 0.0, '+');
            }
            if (0.48..0.75).contains(&ny) {
                ink.layer(tonal * ridge(nx - 0.21, 0.06), 0.0, '|');
            }
            let arp = p.voice(Voice::Arp);
            if (0.69..0.83).contains(&nx) && (0.28..0.75).contains(&ny) {
                let col = ((nx - 0.69) * w / 3.0).floor();
                let row = (ny * h / 2.0).floor();
                let chase = 0.35 + 0.65 * (row - col - l.phase(Voice::Arp) * 4.0).cos().powi(2);
                let window = if (x as u16).is_multiple_of(3) {
                    '|'
                } else {
                    '+'
                };
                ink.layer(arp * chase, -140.0, window);
            }
            let lead = p.voice(Voice::Lead);
            let cx = 0.5 + l.phase(Voice::Lead).sin() * 0.12;
            let dx = (nx - cx) / (0.13 + lead * 0.08);
            let dy = (ny - 0.16) / (0.045 + lead * 0.08);
            ink.layer(lead * ridge(dx.hypot(dy), 1.0), 155.0, '@');
            if nx < cx && nx > cx - 0.3 {
                let tail = 0.16 + (nx * 30.0 - l.phase(Voice::Lead) * 3.0).sin() * 0.035;
                ink.layer(lead * ridge(ny - tail, 0.03), 155.0, '~');
            }
            ink
        });
        l.kick(area, buf);
    }
}

// ------------------------------------------------------------ Atlas

/// Seven named constellations. A bridge lights only while both ends play.
#[derive(Default)]
pub struct Atlas {
    layers: Layers,
}

const CLUSTERS: [(Voice, f32, f32); 7] = [
    (Voice::Pad, 0.5, 0.16),
    (Voice::Bass, 0.5, 0.75),
    (Voice::Perc, 0.12, 0.58),
    (Voice::Clap, 0.88, 0.58),
    (Voice::Tonal, 0.16, 0.22),
    (Voice::Arp, 0.84, 0.22),
    (Voice::Lead, 0.5, 0.45),
];

/// Distance to a segment and progress along it, in aspect-corrected units.
fn segment(at: (f32, f32), from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let length_squared = dx * dx + dy * dy;
    let t = (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length_squared.max(f32::EPSILON))
        .clamp(0.0, 1.0);
    ((at.0 - from.0 - t * dx).hypot(at.1 - from.1 - t * dy), t)
}

impl Scene for Atlas {
    fn name(&self) -> &'static str {
        "atlas"
    }
    fn on_event(&mut self, event: &Event) {
        self.layers.pulse.on_event(event);
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.layers.pulse.sensitivity = sensitivity;
    }
    fn tick(&mut self, dt: f32) {
        self.layers.tick(dt);
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let l = &self.layers;
        let p = &l.pulse;
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        let scale = h.min(w / 2.0);
        l.background(area, buf);
        field(area, buf, p.hue(), |x, y| {
            let mut ink = Ink::default();
            for (a, b) in [
                (0, 4),
                (0, 5),
                (0, 6),
                (1, 2),
                (1, 3),
                (1, 6),
                (2, 4),
                (3, 5),
                (2, 6),
                (3, 6),
            ] {
                let (va, ax, ay) = CLUSTERS[a];
                let (vb, bx, by) = CLUSTERS[b];
                let together = p.voice(va).min(p.voice(vb));
                let (distance, along) =
                    segment((x / 2.0, y), (ax * w / 2.0, ay * h), (bx * w / 2.0, by * h));
                let travel = 0.4 + 0.6 * (along * 18.0 - l.phase(va) - l.phase(vb)).cos().powi(2);
                ink.layer(
                    together * ridge(distance, 0.75) * travel * 0.65,
                    (a + b) as f32 * 16.0 - 90.0,
                    ':',
                );
            }
            for (i, &(voice, cx, cy)) in CLUSTERS.iter().enumerate() {
                let d = p.voice(voice);
                let t = l.phase(voice);
                let dx = (x - w * cx) / 2.0;
                let dy = y - h * cy;
                let distance = dx.hypot(dy);
                let angle = dy.atan2(dx);
                let radius = scale * (0.065 + d * 0.055);
                let offset = i as f32 * 32.0 - 90.0;
                let (shape, glyph) = match voice {
                    Voice::Pad => {
                        let shell = radius * (0.8 + 0.15 * (angle * 3.0 + t).sin());
                        (
                            ridge(distance - shell, 0.9).max(ridge(distance, radius) * 0.6),
                            'O',
                        )
                    }
                    Voice::Bass => (ridge(dx.abs() + dy.abs(), radius * 1.35), '#'),
                    Voice::Perc => {
                        let petals = 0.7 + 0.3 * (angle * 5.0 - t * 2.0).cos();
                        (ridge(distance - radius * petals, 0.85), 'o')
                    }
                    Voice::Clap => {
                        let spokes = (angle * 4.0 + t).cos().abs().powi(3);
                        (ridge(distance, radius * 1.4) * spokes, '*')
                    }
                    Voice::Tonal => (
                        ridge(dx.hypot(dy * 1.6) - radius, 0.8)
                            .max(ridge((dx * 1.6).hypot(dy) - radius, 0.8)),
                        '~',
                    ),
                    Voice::Arp => {
                        let square = dx.abs().max(dy.abs());
                        let chase = 0.55 + 0.45 * (angle * 3.0 - t * 3.0).cos().abs();
                        (ridge(square - radius * 0.75, 0.85) * chase, '+')
                    }
                    Voice::Lead => {
                        let spiral = (angle * 2.0 - distance / radius * 7.0 + t * 2.0)
                            .sin()
                            .abs();
                        (ridge(distance, radius * 1.3) * (0.3 + 0.7 * spiral), '@')
                    }
                    Voice::Kick => (0.0, ' '),
                };
                ink.layer(d * shape, offset, glyph);
                // Each cluster carries a broad, moving satellite on its own clock.
                let orbit = radius * 1.35;
                let sx = orbit * (t + i as f32).cos();
                let sy = orbit * (t + i as f32).sin();
                ink.layer(d * ridge((dx - sx).hypot(dy - sy), 0.8 + d), offset, '+');
            }
            ink
        });
        for (i, &(voice, cx, cy)) in CLUSTERS.iter().enumerate() {
            let (x, y) = if matches!(voice, Voice::Pad | Voice::Bass | Voice::Lead) {
                ((cx * w + scale * 0.48) as u16, (cy * h) as u16)
            } else {
                (
                    (cx * w - voice.name().len() as f32 * 0.5).max(0.0) as u16,
                    (cy * h - scale * 0.16 - 1.0).max(0.0) as u16,
                )
            };
            caption(
                buf,
                area,
                (x, y),
                voice.name(),
                p.hue() + i as f32 * 32.0 - 90.0,
            );
        }
        l.kick(area, buf);
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
        for (w, h) in [
            (0, 0),
            (0, 20),
            (20, 0),
            (1, 1),
            (3, 2),
            (46, 10),
            (60, 20),
            (120, 40),
            (200, 60),
        ] {
            for mut scene in all() {
                for event in [
                    Event::Level(0.3),
                    Event::VoiceLevel(Voice::Pad, 0.05),
                    Event::VoiceLevel(Voice::Bass, 0.2),
                    Event::Kick(0.9),
                    chord(3, 0.5, 0.5),
                    Event::Beat(7.5),
                    Event::Kick(0.0),
                ] {
                    scene.on_event(&event);
                }
                for voice in Voice::ALL {
                    scene.on_event(&Event::VoiceLevel(
                        voice,
                        Sensitivity::default().get(voice as usize),
                    ));
                }
                scene.tick(0.3);
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                scene.render(area, &mut buf);
            }
        }
    }

    fn frame(scene: &dyn Scene, area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        scene.render(area, &mut buf);
        buf
    }

    fn play(scene: &mut dyn Scene, voices: &[Voice]) {
        let sensitivity = Sensitivity::default();
        for &voice in voices {
            scene.on_event(&Event::VoiceLevel(
                voice,
                sensitivity.get(voice as usize) * 0.4225,
            ));
        }
        scene.tick(0.5);
    }

    #[test]
    fn new_scenes_show_each_voice_alone_and_in_the_mix() {
        for (w, h) in [(60, 20), (120, 40)] {
            let area = Rect::new(3, 2, w, h);
            for index in 5..all().len() {
                for voice in Voice::ALL {
                    let mut solo = all().remove(index);
                    let silent = frame(solo.as_ref(), area);
                    play(solo.as_mut(), &[voice]);
                    let active = frame(solo.as_ref(), area);
                    let changed_rows = (area.y..area.bottom())
                        .filter(|&y| {
                            (area.x..area.right())
                                .any(|x| active[(x, y)].symbol() != silent[(x, y)].symbol())
                        })
                        .count();
                    assert!(
                        changed_rows >= 3,
                        "{} {voice:?} too small at {w}x{h}: {changed_rows} rows",
                        solo.name()
                    );

                    let mut mix = all().remove(index);
                    play(mix.as_mut(), &Voice::ALL);
                    let full = frame(mix.as_ref(), area);
                    let mut muted = all().remove(index);
                    let others: Vec<_> = Voice::ALL.into_iter().filter(|&v| v != voice).collect();
                    play(muted.as_mut(), &others);
                    let without = frame(muted.as_ref(), area);
                    let changed = full
                        .content
                        .iter()
                        .zip(&without.content)
                        .filter(|(a, b)| a.symbol() != b.symbol())
                        .count();
                    assert!(
                        changed >= 3,
                        "{} {voice:?} hidden in mix at {w}x{h}: {changed} cells",
                        mix.name()
                    );
                }
            }
        }
    }

    #[test]
    fn new_scenes_ignore_beat_and_settle_after_silence() {
        let area = Rect::new(0, 0, 60, 20);
        for mut scene in all().into_iter().skip(5) {
            let initial = frame(scene.as_ref(), area);
            scene.on_event(&Event::Beat(42.0));
            scene.tick(1.0);
            assert_eq!(
                initial,
                frame(scene.as_ref(), area),
                "{} moves on beat",
                scene.name()
            );
            play(scene.as_mut(), &Voice::ALL);
            scene.on_event(&Event::Kick(0.7));
            scene.on_event(&Event::Level(0.1));
            scene.tick(0.2);
            for voice in Voice::ALL {
                scene.on_event(&Event::VoiceLevel(voice, 0.0));
            }
            scene.on_event(&Event::Level(0.0));
            scene.tick(3.0);
            let settled = frame(scene.as_ref(), area);
            scene.tick(1.0);
            assert_eq!(
                settled,
                frame(scene.as_ref(), area),
                "{} moves in silence",
                scene.name()
            );
        }
    }

    #[test]
    fn new_scenes_apply_live_tuning_and_master_only_motion() {
        let area = Rect::new(0, 0, 60, 20);
        for index in 5..all().len() {
            let mut tuned = all().remove(index);
            let mut control = all().remove(index);
            play(tuned.as_mut(), &Voice::ALL);
            play(control.as_mut(), &Voice::ALL);
            let mut sensitivity = Sensitivity::default();
            for row in 0..Sensitivity::ROWS {
                sensitivity.set(row, 1.0);
            }
            tuned.tune(sensitivity);
            tuned.tick(0.5);
            control.tick(0.5);
            assert_ne!(
                frame(tuned.as_ref(), area),
                frame(control.as_ref(), area),
                "{} ignores tuning",
                tuned.name()
            );
        }
        for mut scene in all().into_iter().skip(5) {
            scene.on_event(&Event::Level(Sensitivity::default().master));
            scene.tick(0.5);
            let before = frame(scene.as_ref(), area);
            scene.tick(0.5);
            assert_ne!(
                before,
                frame(scene.as_ref(), area),
                "{} ignores master",
                scene.name()
            );
        }
    }

    #[test]
    fn new_scenes_share_kick_motion_and_hide_zero_hits() {
        let area = Rect::new(0, 0, 60, 20);
        for age in [0.0, 0.2, 0.8, 1.5, 2.1] {
            let mut expected = None;
            for mut scene in all().into_iter().skip(5) {
                let silent = frame(scene.as_ref(), area);
                scene.on_event(&Event::Kick(0.0));
                assert_eq!(silent, frame(scene.as_ref(), area));
                scene.on_event(&Event::Kick(0.7));
                scene.tick(age);
                let mut actual = frame(scene.as_ref(), area);
                for (cell, before) in actual.content.iter_mut().zip(&silent.content) {
                    if cell == before {
                        *cell = ratatui::buffer::Cell::default();
                    }
                }
                if let Some(ref expected) = expected {
                    assert_eq!(expected, &actual, "{} differs at {age}", scene.name());
                }
                expected = Some(actual);
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
        pulse.on_event(&Event::Level(pulse.sensitivity.master));
        pulse.on_event(&Event::VoiceLevel(Voice::Bass, 0.2));
        pulse.tick(0.5);
        assert!(pulse.drive > 0.9);
        assert!(pulse.voice(Voice::Bass) > 0.9);
        assert_eq!(pulse.voice(Voice::Pad), 0.0);
        pulse.on_event(&Event::Level(0.0));
        pulse.on_event(&Event::VoiceLevel(Voice::Bass, 0.0));
        for _ in 0..100 {
            pulse.tick(0.1);
        }
        assert!(pulse.drive < 0.01);
        assert!(pulse.voice(Voice::Bass) < 0.01);
    }

    #[test]
    fn sensitivity_rows_cover_every_voice_and_master() {
        let mut s = Sensitivity::default();
        assert_eq!(Sensitivity::label(Sensitivity::ROWS - 1), "master");
        s.set(0, 0.5);
        s.set(Sensitivity::ROWS - 1, 0.25);
        assert_eq!(s.get(0), 0.5);
        assert_eq!(s.master, 0.25);
        assert!(s.literal().contains("0.5000, // pad"));
        assert!(s.literal().contains("master: 0.2500"));
        let mut scene = System::default();
        scene.tune(s);
        assert!(scene.pulse.sensitivity == s);
    }

    #[test]
    fn quiet_material_still_registers() {
        // a voice at a tenth of its calibrated level shows at about a third
        let d = Sensitivity::drive(0.06, 0.006);
        assert!((d - 0.316).abs() < 0.01);
    }
}
