//! Scenes: each gives incoming events a shape. A scene owns only animation
//! state; it never touches the network or the terminal directly.
//!
//! The kick is the anchor of every scene: same place, same motion, every hit,
//! with at most one cell of analog wobble so it reads as played, not looped.
//! Its size follows the hit's level, so an inaudible kick is invisible too.
//! Ambient motion follows each voice's level through `Sensitivity`: silence
//! is still, and every layer nooise plays has something on screen.

use std::cell::RefCell;
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
    /// Prepare size-dependent animation state, including scenes not on screen.
    fn resize(&mut self, _area: Rect) {}
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
        Box::new(Veil::default()),
        Box::new(Ion::default()),
        Box::new(Echo::default()),
    ]
}

pub(crate) const GRADIENT: &[char] = &[' ', '·', '∙', '•', '●', '◉', '⬤'];
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
            Event::Unknown(_) | Event::Gesture(..) => {}
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

    /// Atlas uses the same scalar anchor and trajectory as the neon fields.
    fn kick(&self, area: Rect, buf: &mut Buffer) {
        let space = FieldSpace::new(area);
        let hue = self.pulse.hue();
        for y in 0..area.height {
            for x in 0..area.width {
                let value = self.kick_field(space, x as f32 / space.w, y as f32 / space.h);
                Ink::new(value, 0.0, '~').draw(buf, area, x, y, hue);
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

/// Additive field light with energy-weighted hue offsets.
/// Soft compression retains detail where several voices overlap.
#[derive(Clone, Copy, Default)]
struct Glow {
    energy: f32,
    offset: f32,
}

impl Glow {
    fn add(&mut self, energy: f32, offset: f32) {
        self.energy += energy;
        self.offset += energy * offset;
    }

    fn value(self) -> f32 {
        self.energy / (0.5 + self.energy)
    }

    fn tint(self) -> f32 {
        self.offset / self.energy.max(f32::EPSILON)
    }
}

const SHADES: [char; 5] = [' ', '░', '▒', '▓', '█'];

/// Ordered 4x4 thresholds at bin centres, stable in local scene coordinates.
fn bayer(x: u16, y: u16) -> f32 {
    const MATRIX: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (MATRIX[usize::from(y % 4)][usize::from(x % 4)] as f32 + 0.5) / 16.0
}

fn scanline(y: u16) -> f32 {
    if y.is_multiple_of(2) { 1.0 } else { 0.86 }
}

fn shade(buf: &mut Buffer, area: Rect, x: u16, y: u16, v: f32, hue: f32) {
    let v = v.clamp(0.0, 1.0);
    let index = (v * 4.0).round() as usize;
    buf[(area.x + x, area.y + y)]
        .set_char(SHADES[index])
        .set_style(
            Style::default()
                .fg(hsv(hue, 0.85, v * scanline(y)))
                .bg(Color::Black),
        );
}

/// Screen coordinates in rows, corrected for cells twice as tall as wide.
#[derive(Clone, Copy)]
struct FieldSpace {
    w: f32,
    h: f32,
    scale: f32,
}

impl FieldSpace {
    fn new(area: Rect) -> Self {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        Self {
            w,
            h,
            scale: h.min(w * 0.5),
        }
    }

    fn distance(self, x: f32, y: f32, cx: f32, cy: f32) -> f32 {
        let dx = (x - cx) * self.w * 0.5;
        let dy = (y - cy) * self.h;
        (dx * dx + dy * dy).sqrt() / self.scale
    }
}

impl Layers {
    /// Common scalar kick: expanding circular front and a sustained foot.
    /// The centre varies only by the shared one-cell wobble of each hit.
    fn kick_field(&self, space: FieldSpace, x: f32, y: f32) -> f32 {
        let mut value = self.pulse.voice(Voice::Kick) * ridge(space.distance(x, y, 0.5, 1.0), 0.28);
        for hit in &self.pulse.live {
            let distance = space.distance(
                x,
                y,
                0.5 + hit.wobble.0 / space.w,
                1.0 + hit.wobble.1 / space.h,
            );
            value += hit.level * Pulse::fade(hit.age) * ridge(distance - hit.age * 0.6, 0.10);
        }
        value
    }

    fn atmosphere(&self, x: f32, y: f32) -> Glow {
        let mut glow = Glow::default();
        glow.add(
            self.pulse.drive * 0.18 * (0.6 + 0.4 * (x * 5.0 + y * 3.0 - self.ambient * 0.2).sin()),
            60.0,
        );
        glow
    }
}

fn neon_field(area: Rect, buf: &mut Buffer, layers: &Layers, sample: impl Fn(f32, f32) -> Glow) {
    draw_field(area, buf, layers, sample, shade);
}

fn dithered_shade(buf: &mut Buffer, area: Rect, x: u16, y: u16, v: f32, hue: f32) {
    shade(buf, area, x, y, v, hue);
    let index = (v.clamp(0.0, 1.0) * 4.0 + bayer(x, y)).floor() as usize;
    buf[(area.x + x, area.y + y)].set_char(SHADES[index.min(4)]);
}

type Shader = fn(&mut Buffer, Rect, u16, u16, f32, f32);

fn draw_field(
    area: Rect,
    buf: &mut Buffer,
    layers: &Layers,
    sample: impl Fn(f32, f32) -> Glow,
    shader: Shader,
) {
    let space = FieldSpace::new(area);
    let hue = layers.pulse.hue() - 120.0;
    for y in 0..area.height {
        for x in 0..area.width {
            let (nx, ny) = (x as f32 / space.w, y as f32 / space.h);
            let mut glow = sample(nx, ny);
            glow.add(layers.kick_field(space, nx, ny) * 1.5, 180.0);
            shader(buf, area, x, y, glow.value(), hue + glow.tint());
        }
    }
}

// ------------------------------------------------------------ Estuary

/// A striped sun above a breathing grid and broad spectral sky bands.
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
        let space = FieldSpace::new(area);
        let sun_y = 0.38 + 0.06 * (l.phase(Voice::Pad) * 0.18).sin();
        neon_field(area, buf, l, |x, y| {
            let mut glow = l.atmosphere(x, y);
            let sun = space.distance(x, y, 0.5, sun_y);
            let stripes = 0.22 + 0.78 * ridge((y * 19.0).fract() - 0.5, 0.35);
            glow.add(p.voice(Voice::Pad) * ridge(sun, 0.43) * 2.8 * stripes, 90.0);
            let depth = ((y - 0.57) / 0.43).max(0.0);
            if depth > 0.0 {
                let bass = p.voice(Voice::Bass);
                let perspective = 1.0 / (depth + 0.16);
                let columns = ridge(((x - 0.5) * perspective * 5.0).sin(), 0.18);
                let rows = ridge(
                    (perspective * (2.0 + bass * 0.7) - l.phase(Voice::Bass) * 0.45).sin(),
                    0.22,
                );
                glow.add(bass * (0.12 + columns.max(rows)) * depth.sqrt(), 180.0);
                glow.add(
                    p.voice(Voice::Perc)
                        * ridge(
                            (x * 15.0 + depth * 8.0 - l.phase(Voice::Perc) * 1.8).sin(),
                            0.32,
                        )
                        * depth,
                    0.0,
                );
            }
            glow.add(
                p.voice(Voice::Clap)
                    * ridge(
                        y - 0.55 - (x * 12.0 + l.phase(Voice::Clap) * 0.6).sin() * 0.035,
                        0.095,
                    ),
                180.0,
            );
            glow.add(
                p.voice(Voice::Tonal)
                    * ridge(
                        y - 0.13 - (x * 4.0 + l.phase(Voice::Tonal) * 0.25).sin() * 0.05,
                        0.15,
                    ),
                20.0,
            );
            glow.add(
                p.voice(Voice::Arp)
                    * ridge(
                        y - 0.29 - (x * 8.0 - l.phase(Voice::Arp) * 0.45).sin() * 0.04,
                        0.10,
                    )
                    * (0.6 + 0.4 * (x * 24.0 - l.phase(Voice::Arp)).cos()),
                180.0,
            );
            glow.add(
                p.voice(Voice::Lead)
                    * ridge(
                        y - 0.42 + (x - 0.5) * 0.18 - (l.phase(Voice::Lead) * 0.3).sin() * 0.06,
                        0.085,
                    ),
                120.0,
            );
            glow
        });
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

/// Slow interference bands, each voice with its own direction and wavelength.
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
        let space = FieldSpace::new(area);
        // Directions, wavelengths and broad envelopes make solos recognisable.
        let waves = [
            (0.4, 1.0, 6.0, 0.22, 0.44, 90.0),
            (0.0, 1.0, 13.0, 0.84, 0.32, 180.0),
            (1.0, 0.3, 25.0, 0.64, 0.35, 0.0),
            (-0.7, 1.0, 21.0, 0.45, 0.28, 180.0),
            (1.0, 0.7, 11.0, 0.16, 0.30, 25.0),
            (-1.0, 0.4, 32.0, 0.35, 0.28, 150.0),
            (0.8, 1.0, 9.0, 0.59, 0.32, 100.0),
        ];
        neon_field(area, buf, l, |x, y| {
            let mut glow = l.atmosphere(x, y);
            let u = (x - 0.5) * space.w / (2.0 * space.scale);
            let v = y * space.h / space.scale;
            for (i, voice) in INSTRUMENTS.iter().enumerate() {
                let (dx, dy, frequency, centre, width, offset) = waves[i];
                let t = l.phase(*voice) * 0.28;
                let bend = (u * 3.0 - t * 0.4).sin() * 0.5;
                let wave = ((u * dx + v * dy) * frequency + bend - t).sin();
                let interference = 0.25 + 0.75 * wave.powi(2);
                glow.add(
                    p.voice(*voice) * ridge(y - centre, width) * interference,
                    offset,
                );
            }
            glow
        });
    }
}

// ------------------------------------------------------------ Reef

/// Seven drifting metaballs merge into pools of light at their intersections.
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
        let space = FieldSpace::new(area);
        let centres = [
            (0.50, 0.22),
            (0.50, 0.80),
            (0.16, 0.63),
            (0.84, 0.63),
            (0.18, 0.25),
            (0.82, 0.25),
            (0.50, 0.49),
        ];
        // Compute drift once per blob; only squared distances in the cell loop.
        let blobs: [_; 7] = std::array::from_fn(|i| {
            let voice = INSTRUMENTS[i];
            let d = p.voice(voice);
            let t = l.phase(voice) * 0.20;
            let (cx, cy) = centres[i];
            (
                cx + t.sin() * 0.055,
                cy + (t * 0.7).sin() * 0.04,
                0.23 + d * 0.22,
                d,
            )
        });
        neon_field(area, buf, l, |x, y| {
            let mut glow = l.atmosphere(x, y);
            let lift = l.kick_field(space, x, y) * 0.06;
            for (i, &(cx, cy, radius, drive)) in blobs.iter().enumerate() {
                let dx = (x - cx) * space.w / (2.0 * space.scale);
                let dy = (y + lift - cy) * space.h / space.scale;
                let blob = (1.0 - (dx * dx + dy * dy) / (radius * radius)).max(0.0);
                glow.add(
                    drive * blob.powi(2) * 2.2,
                    if i.is_multiple_of(2) {
                        75.0 + i as f32 * 8.0
                    } else {
                        180.0
                    },
                );
            }
            // A soft isosurface makes overlapping lobes merge into luminous pools.
            let tint = glow.tint();
            glow.energy = (glow.energy - 0.035).max(0.0) * 1.4;
            glow.offset = tint * glow.energy;
            glow
        });
    }
}

// ------------------------------------------------------------ City

/// A dark skyline cuts into neon haze; windows and long beams cross its face.
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
        let space = FieldSpace::new(area);
        let bass = p.voice(Voice::Bass);
        neon_field(area, buf, l, |x, y| {
            let mut glow = l.atmosphere(x, y);
            let block = (x * 11.0).floor();
            let roof = 0.65 - (0.10 + bass * 0.25) * (0.4 + 0.6 * (block * 2.37).sin().abs())
                + bass * (l.phase(Voice::Bass) * 0.25 + block).sin() * 0.035;
            let facade =
                ((y - roof) * space.h).clamp(0.0, 1.0) * ((0.85 - y) * space.h).clamp(0.0, 1.0);
            let sky = ridge(y - 0.40, 0.58) * (1.0 - facade * 0.94);
            glow.add(
                p.voice(Voice::Pad)
                    * sky
                    * (0.65 + 0.25 * (x * 3.0 + l.phase(Voice::Pad) * 0.15).sin()),
                95.0,
            );
            let edge = ridge((x * 11.0).fract() - 0.08, 0.10);
            glow.add(
                bass * (facade * (0.10 + edge * 0.8) + ridge(y - roof, 0.045) * 0.6),
                180.0,
            );
            let windows = 0.5 + 0.5 * (y * 19.0 - block - l.phase(Voice::Arp) * 0.55).sin();
            let threshold = bayer((x * space.w) as u16, (y * space.h) as u16);
            glow.add(
                p.voice(Voice::Arp) * facade * if windows > threshold { 0.95 } else { 0.08 },
                30.0,
            );
            glow.add(
                p.voice(Voice::Perc)
                    * ridge(y - 0.88, 0.17)
                    * (0.35 + 0.65 * (x * 17.0 - l.phase(Voice::Perc) * 1.4).sin().powi(2)),
                180.0,
            );
            glow.add(
                p.voice(Voice::Clap)
                    * ridge(y - 0.66, 0.12)
                    * (0.4 + 0.6 * (x * 10.0 + l.phase(Voice::Clap)).sin().powi(2)),
                110.0,
            );
            glow.add(
                p.voice(Voice::Tonal)
                    * ridge(
                        y - 0.17 - x * 0.18 - (l.phase(Voice::Tonal) * 0.16).sin() * 0.08,
                        0.09,
                    ),
                180.0,
            );
            glow.add(
                p.voice(Voice::Lead)
                    * ridge(
                        y - 0.37 + x * 0.22 - (l.phase(Voice::Lead) * 0.19).sin() * 0.08,
                        0.08,
                    ),
                60.0,
            );
            glow
        });
    }
}

// ------------------------------------------------------------ Atlas

/// Seven constellations. A bridge lights only while both ends play.
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
        l.kick(area, buf);
    }
}

// ------------------------------------------------------------ Veil

/// Half-row samples turn broad drifting light curtains into smooth gradients.
#[derive(Default)]
pub struct Veil {
    layers: Layers,
}

/// Upper and lower samples have independent colours; unlit halves stay black.
fn halfblock(buf: &mut Buffer, area: Rect, x: u16, y: u16, colours: [Color; 2]) {
    buf[(area.x + x, area.y + y)]
        .set_char('▀')
        .set_style(Style::default().fg(colours[0]).bg(colours[1]));
}

impl Scene for Veil {
    fn name(&self) -> &'static str {
        "veil"
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
        let space = FieldSpace::new(area);
        let hue = p.hue() - 120.0;
        let bands = [
            (0.26, 0.0, 0.36, 90.0),
            (0.85, 0.0, 0.24, 180.0),
            (0.63, 0.32, 0.16, 0.0),
            (0.60, -0.35, 0.16, 180.0),
            (0.14, 0.12, 0.14, 25.0),
            (0.39, -0.16, 0.13, 160.0),
            (0.48, 0.20, 0.15, 110.0),
        ];
        for y in 0..area.height {
            for x in 0..area.width {
                let colours = std::array::from_fn(|half| {
                    let nx = (x as f32 + 0.5) / space.w;
                    let ny = (y as f32 + half as f32 * 0.5 + 0.25) / space.h;
                    let mut glow = l.atmosphere(nx, ny);
                    for (i, voice) in INSTRUMENTS.iter().enumerate() {
                        let (centre, slope, width, offset) = bands[i];
                        let t = l.phase(*voice) * 0.22;
                        let bend = (nx * (3.0 + i as f32) - t).sin() * 0.055;
                        let distance = ny - centre - (nx - 0.5) * slope - bend;
                        let coverage = ridge(distance, width);
                        glow.add(p.voice(*voice) * coverage * coverage * 1.4, offset);
                    }
                    glow.add(l.kick_field(space, nx, ny) * 1.5, 180.0);
                    hsv(hue + glow.tint(), 0.85, glow.value() * scanline(y))
                });
                halfblock(buf, area, x, y, colours);
            }
        }
    }
}

// ------------------------------------------------------------ Ion

/// Ordered dithering resolves slow nebula folds between shade-ramp steps.
#[derive(Default)]
pub struct Ion {
    layers: Layers,
}

impl Scene for Ion {
    fn name(&self) -> &'static str {
        "ion"
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
        let space = FieldSpace::new(area);
        let folds: [_; 7] = std::array::from_fn(|i| {
            let t = l.phase(INSTRUMENTS[i]) * 0.12;
            let angle = i as f32 * 0.8 + t;
            (angle.cos(), angle.sin(), t)
        });
        draw_field(
            area,
            buf,
            l,
            |x, y| {
                let u = (x - 0.5) * space.w / (2.0 * space.scale);
                let v = (y - 0.5) * space.h / space.scale;
                let mut glow = l.atmosphere(x, y);
                for (i, voice) in INSTRUMENTS.iter().enumerate() {
                    let (cos, sin, t) = folds[i];
                    let along = u * cos + v * sin;
                    let across = v * cos - u * sin;
                    let bend = (along * (4.0 + i as f32) - t).sin() * 0.13;
                    let veil = ridge(across - bend - (i as f32 - 3.0) * 0.075, 0.24);
                    let plasma = 0.3 + 0.7 * (along * 5.0 + t * 1.5).cos().powi(2);
                    glow.add(
                        p.voice(*voice) * veil * plasma,
                        if i.is_multiple_of(2) {
                            80.0 + i as f32 * 7.0
                        } else {
                            180.0
                        },
                    );
                }
                glow
            },
            dithered_shade,
        );
    }
}

// ------------------------------------------------------------ Echo

/// A retained light field. Ticks decay and redraw it even while hidden.
#[derive(Default)]
pub struct Echo {
    layers: Layers,
    // The app sizes hidden scenes before ticking. Direct render callers can
    // also initialise this cache; repeated renders never advance time.
    trails: RefCell<TrailFrame>,
}

#[derive(Default)]
struct TrailFrame {
    size: (u16, u16),
    cells: Vec<Glow>,
}

impl TrailFrame {
    fn redraw(&mut self, layers: &Layers, dt: f32) {
        let area = Rect::new(0, 0, self.size.0, self.size.1);
        let space = FieldSpace::new(area);
        let radii = [0.46, 0.66, 0.56, 0.34, 0.24, 0.16, 0.40];
        let brushes: [_; 7] = std::array::from_fn(|i| {
            let voice = INSTRUMENTS[i];
            (
                radii[i],
                layers.phase(voice) * 0.28,
                layers.pulse.voice(voice),
            )
        });
        let decay = (1.0 - dt / 1.4).max(0.0);
        for y in 0..area.height {
            for x in 0..area.width {
                let nx = x as f32 / space.w;
                let ny = y as f32 / space.h;
                let u = (nx - 0.5) * space.w / (2.0 * space.scale);
                let v = (ny - 0.45) * space.h / space.scale;
                let radius = (u * u + v * v).sqrt();
                let angle = v.atan2(u);
                let mut fresh = layers.atmosphere(nx, ny);
                for (i, &(orbit, phase, drive)) in brushes.iter().enumerate() {
                    let curl = (angle * 2.0 + phase).sin() * 0.075;
                    let ribbon = ridge(radius - orbit - curl, 0.075 + drive * 0.04);
                    let sweep = 0.35 + 0.65 * (angle + phase + i as f32).cos().powi(2);
                    fresh.add(
                        drive * ribbon * sweep,
                        if i.is_multiple_of(2) {
                            85.0 + i as f32 * 6.0
                        } else {
                            180.0
                        },
                    );
                }
                fresh.add(layers.kick_field(space, nx, ny) * 1.5, 180.0);
                let old =
                    &mut self.cells[usize::from(y) * usize::from(area.width) + usize::from(x)];
                old.energy *= decay;
                old.offset *= decay;
                if fresh.energy >= old.energy {
                    *old = fresh;
                }
            }
        }
    }
}

impl Echo {
    fn prepare(&self, area: Rect) {
        let mut trails = self.trails.borrow_mut();
        if trails.size != (area.width, area.height) {
            trails.size = (area.width, area.height);
            trails.cells.clear();
            trails.cells.resize(
                usize::from(area.width) * usize::from(area.height),
                Glow::default(),
            );
            trails.redraw(&self.layers, 0.0);
        }
    }
}

impl Scene for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn on_event(&mut self, event: &Event) {
        self.layers.pulse.on_event(event);
    }
    fn tune(&mut self, sensitivity: Sensitivity) {
        self.layers.pulse.sensitivity = sensitivity;
    }
    fn tick(&mut self, dt: f32) {
        self.layers.tick(dt);
        self.trails.get_mut().redraw(&self.layers, dt);
    }
    fn resize(&mut self, area: Rect) {
        self.prepare(area);
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.prepare(area);
        let trails = self.trails.borrow();
        let hue = self.layers.pulse.hue() - 120.0;
        for y in 0..area.height {
            for x in 0..area.width {
                let glow = trails.cells[usize::from(y) * usize::from(area.width) + usize::from(x)];
                shade(buf, area, x, y, glow.value(), hue + glow.tint());
            }
        }
    }
}

pub(crate) fn hsv(h: f32, s: f32, v: f32) -> Color {
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

    fn visibly_different(a: &ratatui::buffer::Cell, b: &ratatui::buffer::Cell) -> bool {
        if a.symbol() == " " && b.symbol() == " " {
            a.bg != b.bg
        } else {
            a != b
        }
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
                                .any(|x| visibly_different(&active[(x, y)], &silent[(x, y)]))
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
                        .filter(|(a, b)| visibly_different(a, b))
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
            for index in 5..all().len() {
                let mut scene = all().remove(index);
                let silent = frame(scene.as_ref(), area);
                scene.on_event(&Event::Kick(0.0));
                assert_eq!(silent, frame(scene.as_ref(), area));
                scene.on_event(&Event::Kick(0.7));
                scene.tick(age);
                let actual = frame(scene.as_ref(), area);
                let mut replay = all().remove(index);
                replay.on_event(&Event::Kick(0.0));
                replay.on_event(&Event::Kick(0.7));
                replay.tick(age);
                assert_eq!(
                    actual,
                    frame(replay.as_ref(), area),
                    "{} kick is not repeatable",
                    scene.name()
                );
                if age < Pulse::LIFETIME {
                    assert_ne!(actual, silent, "{} hides kick at {age}", scene.name());
                } else {
                    assert_eq!(actual, silent, "{} never settles", scene.name());
                }
            }
        }
    }

    #[test]
    fn new_scenes_keep_order_and_explorations_follow_atlas() {
        let names: Vec<_> = all().iter().map(|scene| scene.name()).collect();
        assert_eq!(
            names,
            [
                "fluid", "system", "binary", "tide", "rain", "estuary", "loom", "reef", "city",
                "atlas", "veil", "ion", "echo"
            ]
        );
    }

    #[test]
    fn new_scenes_kick_field_is_round_anchored_and_level_scaled() {
        for (w, h) in [(60, 20), (120, 40), (200, 60)] {
            let space = FieldSpace::new(Rect::new(0, 0, w, h));
            let mut layers = Layers::default();
            layers.pulse.on_event(&Event::Kick(0.8));
            let hit = &layers.pulse.live[0];
            let cx = 0.5 + hit.wobble.0 / space.w;
            let cy = 1.0 + hit.wobble.1 / space.h;
            assert!((layers.kick_field(space, cx, cy) - 0.8).abs() < 1e-5);
            layers.tick(0.5);
            let radius = 0.3 * space.scale;
            let right = layers.kick_field(space, cx + radius * 2.0 / space.w, cy);
            let above = layers.kick_field(space, cx, cy - radius / space.h);
            assert!((right - above).abs() < 1e-5, "2:1 cell aspect");
            assert!(above > 0.5);
            layers.pulse.live[0].level = 0.4;
            assert!(
                (layers.kick_field(space, cx, cy - radius / space.h) * 2.0 - above).abs() < 1e-5
            );
            layers.pulse.live[0].level = 0.0;
            assert_eq!(layers.kick_field(space, cx, cy - radius / space.h), 0.0);
        }
    }

    #[test]
    fn new_scenes_halfblocks_and_dither_resolve_subcell_gradients() {
        let area = Rect::new(3, 2, 60, 20);
        let mut veil = Veil::default();
        play(&mut veil, &Voice::ALL);
        let rendered = frame(&veil, area);
        assert!(
            rendered
                .content
                .iter()
                .filter(|cell| cell.symbol() == "▀" && cell.fg != cell.bg)
                .count()
                > 600
        );
        let tile = Rect::new(0, 0, 4, 4);
        let mut buf = Buffer::empty(tile);
        for y in 0..4 {
            for x in 0..4 {
                dithered_shade(&mut buf, tile, x, y, 0.375, 200.0);
                assert_eq!(bayer(x, y), bayer(x + 4, y + 4));
            }
        }
        assert_eq!(buf.content.iter().filter(|c| c.symbol() == "░").count(), 8);
        assert_eq!(buf.content.iter().filter(|c| c.symbol() == "▒").count(), 8);
    }

    #[test]
    fn new_scenes_echo_retains_decays_and_updates_hidden_frames() {
        let area = Rect::new(3, 2, 60, 20);
        let mut echo = Echo::default();
        echo.resize(area);
        echo.on_event(&Event::Kick(0.8));
        echo.tick(0.1);
        let lit = frame(&echo, area);
        assert_eq!(lit, frame(&echo, area), "render must not decay trails");
        // Stop excitation without discarding the frame: only the retained ring remains.
        echo.layers.pulse.live.clear();
        echo.tick(0.1);
        let retained: f32 = echo.trails.borrow().cells.iter().map(|c| c.energy).sum();
        assert!(retained > 1.0);
        let mut fresh = Echo::default();
        assert_ne!(frame(&echo, area), frame(&fresh, area));
        for _ in 0..30 {
            echo.tick(0.1);
        }
        let faded: f32 = echo.trails.borrow().cells.iter().map(|c| c.energy).sum();
        assert!(faded < retained * 0.15);
        // Hidden ticks and visible ticks produce the same state.
        play(&mut echo, &Voice::ALL);
        play(&mut fresh, &Voice::ALL);
        echo.tick(3.0);
        fresh.tick(3.0);
        for _ in 0..10 {
            echo.tick(0.1);
            fresh.tick(0.1);
            frame(&fresh, area);
        }
        assert_eq!(frame(&echo, area), frame(&fresh, area));
        for size in [(1, 1), (0, 20), (120, 40), (60, 20)] {
            let area = Rect::new(3, 2, size.0, size.1);
            frame(&echo, area);
            echo.tick(0.1);
            frame(&echo, area);
            assert_eq!(
                echo.trails.borrow().cells.len(),
                usize::from(size.0) * usize::from(size.1)
            );
        }
    }

    /// Run explicitly for visual review and debug frame timing; writes cell dumps
    /// under the OS temporary directory, without adding production scene I/O.
    #[test]
    #[ignore = "visual review and debug timing"]
    fn new_scenes_render_review() {
        use std::fmt::Write;
        use std::time::Instant;
        let directory = std::env::temp_dir().join("foorm-review");
        std::fs::create_dir_all(&directory).unwrap();
        for (width, height) in [(60, 20), (120, 40), (200, 60)] {
            let area = Rect::new(0, 0, width, height);
            let mut scenes = all();
            for scene in &mut scenes {
                play(scene.as_mut(), &Voice::ALL);
                scene.on_event(&chord(1, 0.1, 1.0));
                frame(scene.as_ref(), area);
            }
            for index in [5, 6, 7, 8, 10, 11, 12] {
                let mut durations = Vec::new();
                let mut buffer = Buffer::empty(area);
                for step in 0usize..70 {
                    let start = Instant::now();
                    for scene in &mut scenes {
                        if step.is_multiple_of(15) {
                            scene.on_event(&Event::Kick(0.65));
                        }
                        scene.tick(1.0 / 30.0);
                    }
                    buffer.reset();
                    scenes[index].render(area, &mut buffer);
                    if step >= 10 {
                        durations.push(start.elapsed());
                    }
                }
                durations.sort();
                eprintln!(
                    "{} {width}x{height}: tick-all + render p95 {:.2} ms",
                    scenes[index].name(),
                    durations[57].as_secs_f64() * 1000.0
                );
                let mut dump = format!("{width} {height}\n");
                for cell in &buffer.content {
                    let rgb = |colour| match colour {
                        Color::Rgb(r, g, b) => (r, g, b),
                        _ => (0, 0, 0),
                    };
                    let (r, g, b) = rgb(cell.fg);
                    let (br, bg, bb) = rgb(cell.bg);
                    writeln!(
                        dump,
                        "{} {r} {g} {b} {br} {bg} {bb}",
                        cell.symbol().chars().next().unwrap() as u32
                    )
                    .unwrap();
                }
                std::fs::write(
                    directory.join(format!("{}-{width}.cells", scenes[index].name())),
                    dump,
                )
                .unwrap();
            }
        }
        eprintln!("renders: {}", directory.display());
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
