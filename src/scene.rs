//! Scenes: each gives incoming events a shape. A scene owns only animation
//! state; it never touches the network or the terminal directly.

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

const GRADIENT: &[char] = &[' ', '·', '∙', '•', '●', '◉', '⬤'];
const RIPPLE_LIFETIME: f32 = 2.5;
const RIPPLE_SPEED: f32 = 0.5; // normalized units / s

/// Kicks spawn rings that radiate out from the centre and fade.
pub struct Pulse {
    t: f32,
    ripples: Vec<(f32, f32, f32)>, // (cx, cy, age) in 0..1 field coords
    hue: f32,
    kicks: u64,
}

impl Pulse {
    pub fn new() -> Self {
        Self {
            t: 0.0,
            ripples: Vec::new(),
            hue: 205.0,
            kicks: 0,
        }
    }
}

impl Scene for Pulse {
    fn name(&self) -> &'static str {
        "pulse"
    }

    fn on_event(&mut self, event: &Event) {
        match event {
            Event::Kick => {
                self.kicks += 1;
                let n = self.kicks as f32;
                // golden-angle scatter so consecutive rings don't stack
                let cx = 0.5 + ((n * 0.618_034).fract() - 0.5) * 0.6;
                let cy = 0.5 + ((n * 0.381_966).fract() - 0.5) * 0.6;
                self.ripples.push((cx, cy, 0.0));
            }
            Event::Chord(c) => {
                const HUES: [f32; 5] = [205.0, 270.0, 325.0, 158.0, 38.0];
                self.hue = HUES[(*c).rem_euclid(HUES.len() as i32) as usize];
            }
            Event::Beat(_) | Event::Unknown(_) => {}
        }
    }

    fn tick(&mut self, dt: f32) {
        self.t += dt;
        for r in &mut self.ripples {
            r.2 += dt;
        }
        self.ripples.retain(|r| r.2 < RIPPLE_LIFETIME);
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = area.width.max(1) as f32;
        let h = area.height.max(1) as f32;
        // terminal cells are ~2:1, so stretch x to keep rings round
        let aspect = (w / h) / 2.0;
        for y in 0..area.height {
            for x in 0..area.width {
                let nx = x as f32 / w;
                let ny = y as f32 / h;
                let mut v = 0.0f32;
                for &(cx, cy, age) in &self.ripples {
                    let dx = (nx - cx) * aspect.max(1.0);
                    let dy = (ny - cy) / aspect.min(1.0);
                    let dist = (dx * dx + dy * dy).sqrt();
                    let front = age * RIPPLE_SPEED;
                    let fade = (1.0 - age / RIPPLE_LIFETIME).max(0.0);
                    let ring = (-((dist - front) * 10.0).powi(2)).exp();
                    v += ring * fade;
                }
                let v = v.clamp(0.0, 1.0);
                let idx =
                    ((v * (GRADIENT.len() - 1) as f32).round() as usize).min(GRADIENT.len() - 1);
                let val = 0.15 + v * 0.85;
                buf[(area.x + x, area.y + y)]
                    .set_char(GRADIENT[idx])
                    .set_style(Style::default().fg(hsv(self.hue + (v - 0.5) * 30.0, 0.7, val)));
            }
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
