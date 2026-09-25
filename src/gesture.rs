//! Live gestures mirrored from nooise (`/nooise/gesture/<name>`), applied as
//! a grade over the finished frame so every scene answers the same key the
//! same way: Bloom glows, Lift brightens and sweeps the floor away, Submerge
//! darkens and sinks the palette, Echo trails.
//!
//! A gesture has two sources and takes the larger: foorm's own keys (the same
//! `z x c v` as nooise, with nooise's rise and return times so the two feel
//! alike) and nooise's mirrored amounts, already enveloped by its audio clock
//! so a gesture played there rises and returns exactly with its sound. Two
//! gestures held at once crossfade by their relative amounts rather than
//! stacking: pressing a second key while the first is held slides the frame
//! from one look into the other.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::osc::{Event, Gesture};
use crate::scene::{GRADIENT, hsv};

/// Per-cell hue, saturation, value as painted.
type Hsv = (f32, f32, f32);

/// Seconds for a locally held gesture to reach full, as in nooise.
fn rise_seconds(gesture: Gesture) -> f32 {
    match gesture {
        Gesture::Bloom => 1.5,
        Gesture::Submerge => 1.2,
        Gesture::Echo => 0.7,
        Gesture::Lift => 0.55,
    }
}

/// Seconds for a released gesture to return, as in nooise.
const RETURN_SECONDS: f32 = 0.05;

#[derive(Clone, Copy, Default)]
struct Local {
    amount: f32,
    held: bool,
}

#[derive(Default)]
pub struct Gestures {
    local: [Local; Gesture::ALL.len()],
    osc: [f32; Gesture::ALL.len()],
    /// Last graded frame, for Echo trails; sized to the last area.
    echo: Vec<Hsv>,
    echo_size: (u16, u16),
}

impl Gestures {
    pub fn on_event(&mut self, event: &Event) {
        if let Event::Gesture(gesture, amount) = event {
            self.osc[*gesture as usize] = amount.clamp(0.0, 1.0);
        }
    }

    pub fn press(&mut self, gesture: Gesture) {
        self.local[gesture as usize].held = true;
    }

    pub fn release(&mut self, gesture: Gesture) {
        self.local[gesture as usize].held = false;
    }

    /// For terminals that report no key releases: one press holds, the next
    /// releases.
    pub fn toggle(&mut self, gesture: Gesture) {
        let local = &mut self.local[gesture as usize];
        local.held = !local.held;
    }

    pub fn held(&self, gesture: Gesture) -> bool {
        self.local[gesture as usize].held
    }

    /// Advance the local envelopes by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        for (gesture, local) in Gesture::ALL.into_iter().zip(&mut self.local) {
            local.amount = if local.held {
                (local.amount + dt / rise_seconds(gesture)).min(1.0)
            } else {
                (local.amount - dt / RETURN_SECONDS).max(0.0)
            };
        }
    }

    /// The larger of the local hold and nooise's mirrored amount.
    pub fn amount(&self, gesture: Gesture) -> f32 {
        self.local[gesture as usize]
            .amount
            .max(self.osc[gesture as usize])
    }

    fn amounts(&self) -> [f32; Gesture::ALL.len()] {
        Gesture::ALL.map(|g| self.amount(g))
    }

    /// Weight of each gesture in the grade: alone, its amount; together,
    /// their amounts share one whole, so a rising second gesture fades the
    /// first out as it fades in.
    pub fn weights(&self) -> [f32; Gesture::ALL.len()] {
        let amounts = self.amounts();
        let total: f32 = amounts.iter().sum();
        if total <= 1.0 {
            amounts
        } else {
            amounts.map(|a| a / total)
        }
    }

    pub fn apply(&mut self, area: Rect, buf: &mut Buffer) {
        let w = self.weights();
        let (cols, rows) = (area.width as usize, area.height as usize);
        if self.echo_size != (area.width, area.height) {
            self.echo = vec![(0.0, 0.0, 0.0); cols * rows];
            self.echo_size = (area.width, area.height);
        }
        let idle = w.iter().all(|&a| a <= 0.0);
        let src: Vec<Hsv> = (0..rows)
            .flat_map(|y| (0..cols).map(move |x| (x, y)))
            .map(|(x, y)| read(&buf[(area.x + x as u16, area.y + y as u16)]))
            .collect();
        if idle {
            self.echo.copy_from_slice(&src);
            return;
        }
        let bloom = w[Gesture::Bloom as usize];
        let lift = w[Gesture::Lift as usize];
        let submerge = w[Gesture::Submerge as usize];
        let echo = w[Gesture::Echo as usize];
        for y in 0..rows {
            for x in 0..cols {
                let (mut h, mut s, mut v) = src[y * cols + x];
                if bloom > 0.0 {
                    let mut glow: f32 = 0.0;
                    for dy in y.saturating_sub(1)..=(y + 1).min(rows - 1) {
                        for dx in x.saturating_sub(2)..=(x + 2).min(cols - 1) {
                            if (dx, dy) != (x, y) {
                                let n = src[dy * cols + dx];
                                if n.2 > glow {
                                    glow = n.2;
                                    if v <= 0.0 {
                                        h = n.0;
                                    }
                                }
                            }
                        }
                    }
                    v = v.max(glow * 0.8 * bloom);
                    s *= 1.0 - 0.3 * bloom;
                }
                if lift > 0.0 {
                    let floor = y as f32 / rows.max(1) as f32;
                    v += (1.0 - v) * 0.5 * lift;
                    v *= 1.0 - 0.8 * lift * floor * floor;
                    h += 45.0 * lift;
                }
                if submerge > 0.0 {
                    v *= 1.0 - 0.55 * submerge;
                    h = lerp_hue(h, 215.0, submerge);
                    s += (1.0 - s) * 0.5 * submerge;
                }
                if echo > 0.0 {
                    let prev = self.echo[y * cols + x];
                    let trail = prev.2 * 0.93 * echo;
                    if trail > v {
                        (h, s, v) = (prev.0, prev.1, trail);
                    }
                }
                let out = (h, s, v.clamp(0.0, 1.0));
                self.echo[y * cols + x] = out;
                write(&mut buf[(area.x + x as u16, area.y + y as u16)], out);
            }
        }
    }
}

fn lerp_hue(from: f32, to: f32, k: f32) -> f32 {
    let mut d = (to - from).rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    from + d * k
}

/// The hue, saturation, and value a cell was painted with; an unlit or
/// non-RGB cell reads as black.
fn read(cell: &ratatui::buffer::Cell) -> Hsv {
    let Color::Rgb(r, g, b) = cell.fg else {
        return (0.0, 0.0, 0.0);
    };
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// Re-paint a cell from a graded value. Glyphs from the shared gradient
/// follow the value; any other glyph (a caption) keeps its shape and only
/// takes the colour.
fn write(cell: &mut ratatui::buffer::Cell, (h, s, v): Hsv) {
    let symbol = cell.symbol();
    let graded = GRADIENT.iter().any(|g| g.to_string() == symbol);
    if v <= 0.0 && (graded || symbol == " ") {
        cell.reset();
        return;
    }
    if graded {
        let idx = ((v * (GRADIENT.len() - 1) as f32).round() as usize).min(GRADIENT.len() - 1);
        cell.set_char(GRADIENT[idx]);
    }
    cell.set_style(Style::default().fg(hsv(h, s, v)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit_frame(area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        for x in 0..area.width {
            let cell = &mut buf[(x, area.height / 2)];
            cell.set_char('●');
            cell.set_style(Style::default().fg(hsv(200.0, 0.7, 0.8)));
        }
        buf
    }

    #[test]
    fn local_hold_rises_at_nooise_speed_returns_fast_and_joins_osc_by_max() {
        let mut g = Gestures::default();
        g.press(Gesture::Lift);
        g.tick(0.275);
        assert!((g.amount(Gesture::Lift) - 0.5).abs() < 1e-3);
        g.tick(1.0);
        assert_eq!(g.amount(Gesture::Lift), 1.0);
        g.release(Gesture::Lift);
        g.tick(0.05);
        assert_eq!(g.amount(Gesture::Lift), 0.0);

        g.on_event(&Event::Gesture(Gesture::Lift, 0.3));
        g.press(Gesture::Lift);
        g.tick(0.055);
        assert!(
            (g.amount(Gesture::Lift) - 0.3).abs() < 1e-3,
            "osc wins while local is lower"
        );
        g.tick(0.5);
        assert!(g.amount(Gesture::Lift) > 0.9, "local wins once higher");

        g.toggle(Gesture::Echo);
        assert!(g.held(Gesture::Echo));
        g.toggle(Gesture::Echo);
        assert!(!g.held(Gesture::Echo));
    }

    #[test]
    fn two_held_gestures_crossfade_instead_of_stacking() {
        let mut g = Gestures::default();
        g.on_event(&Event::Gesture(Gesture::Bloom, 1.0));
        assert_eq!(g.weights()[Gesture::Bloom as usize], 1.0);
        g.on_event(&Event::Gesture(Gesture::Submerge, 1.0));
        let w = g.weights();
        assert_eq!(w[Gesture::Bloom as usize], 0.5);
        assert_eq!(w[Gesture::Submerge as usize], 0.5);
        g.on_event(&Event::Gesture(Gesture::Submerge, 0.25));
        assert_eq!(g.weights()[Gesture::Submerge as usize], 0.2);
    }

    #[test]
    fn submerge_darkens_bloom_spreads_and_echo_trails() {
        let area = Rect::new(0, 0, 20, 8);
        let mid = (10, 4);
        let above = (10, 3);
        let mut g = Gestures::default();

        let mut buf = lit_frame(area);
        g.on_event(&Event::Gesture(Gesture::Submerge, 1.0));
        g.apply(area, &mut buf);
        assert!(read(&buf[mid]).2 < 0.5, "submerge darkens");

        g.on_event(&Event::Gesture(Gesture::Submerge, 0.0));
        g.on_event(&Event::Gesture(Gesture::Bloom, 1.0));
        let mut buf = lit_frame(area);
        assert_eq!(read(&buf[above]).2, 0.0);
        g.apply(area, &mut buf);
        assert!(read(&buf[above]).2 > 0.5, "bloom lights the neighbour");

        g.on_event(&Event::Gesture(Gesture::Bloom, 0.0));
        g.on_event(&Event::Gesture(Gesture::Echo, 1.0));
        let mut buf = lit_frame(area);
        g.apply(area, &mut buf);
        let mut dark = Buffer::empty(area);
        g.apply(area, &mut dark);
        assert!(read(&dark[mid]).2 > 0.5, "echo keeps last frame's cells");

        g.on_event(&Event::Gesture(Gesture::Echo, 0.0));
        let mut dark = Buffer::empty(area);
        g.apply(area, &mut dark);
        assert_eq!(read(&dark[mid]).2, 0.0, "idle grade leaves the frame alone");
    }

    #[test]
    fn every_gesture_grades_any_size() {
        for gesture in Gesture::ALL {
            let mut g = Gestures::default();
            g.on_event(&Event::Gesture(gesture, 0.7));
            for (w, h) in [(1, 1), (3, 2), (80, 24)] {
                let area = Rect::new(0, 0, w, h);
                let mut buf = lit_frame(area);
                g.apply(area, &mut buf);
            }
        }
    }
}
