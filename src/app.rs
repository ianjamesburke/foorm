//! The terminal loop: drain events into the scene, animate at a fixed frame
//! rate, draw the scene full-screen, and overlay settings or the tune panel
//! on request. The tune panel edits the shared `Sensitivity` table live and
//! the final table is printed on quit so it can be pasted into `scene.rs`.

use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Frame;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::gesture::Gestures;
use crate::osc::{Event, Gesture, Voice};
use crate::scene::{self, Scene, Sensitivity};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

struct App {
    listen: SocketAddr,
    scenes: Vec<Box<dyn Scene>>,
    current: usize,
    settings_open: bool,
    tune_open: bool,
    tune_row: usize,
    sensitivity: Sensitivity,
    gestures: Gestures,
    received: u64,
    kicks: u64,
    unknown: u64,
    last_event: Option<Event>,
    beat: f32,
    level: f32,
    voice_levels: [f32; Voice::ALL.len()],
}

impl App {
    fn absorb(&mut self, event: Event) {
        self.received += 1;
        match &event {
            Event::Kick(_) => self.kicks += 1,
            Event::Beat(b) => self.beat = *b,
            Event::Level(l) => self.level = *l,
            Event::VoiceLevel(voice, l) => self.voice_levels[*voice as usize] = *l,
            Event::Unknown(_) => self.unknown += 1,
            Event::Chord { .. } | Event::Gesture(..) => {}
        }
        for scene in &mut self.scenes {
            scene.on_event(&event);
        }
        self.gestures.on_event(&event);
        self.last_event = Some(event);
    }

    /// Returns false when the user asked to quit. Raw mode turns Ctrl+C into
    /// a key event rather than a signal, so it is honoured here.
    fn on_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        if self.tune_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('t') => self.tune_open = false,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.tune_row = (self.tune_row + Sensitivity::ROWS - 1) % Sensitivity::ROWS;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.tune_row = (self.tune_row + 1) % Sensitivity::ROWS;
                }
                KeyCode::Left | KeyCode::Char('-') => self.scale_row(1.0 / 1.25),
                KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => self.scale_row(1.25),
                KeyCode::Char('r') => {
                    let default = Sensitivity::default().get(self.tune_row);
                    self.sensitivity.set(self.tune_row, default);
                    self.push_sensitivity();
                }
                KeyCode::Char('q') => return false,
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc if !self.settings_open => return false,
            KeyCode::Esc => self.settings_open = false,
            KeyCode::Tab | KeyCode::Char('s') => self.settings_open = !self.settings_open,
            KeyCode::Char('t') => self.tune_open = true,
            KeyCode::Char('n') | KeyCode::Right => {
                self.current = (self.current + 1) % self.scenes.len();
            }
            KeyCode::Char('p') | KeyCode::Left => {
                self.current = (self.current + self.scenes.len() - 1) % self.scenes.len();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let digit = c.to_digit(10).unwrap_or(0) as usize;
                let n = if digit == 0 { 10 } else { digit };
                if (1..=self.scenes.len()).contains(&n) {
                    self.current = n - 1;
                }
            }
            _ => {}
        }
        true
    }

    /// Multiply the selected row's full level: log steps so every voice,
    /// whether its level lives near 0.002 or 0.2, moves by the same feel.
    fn scale_row(&mut self, by: f32) {
        let now = self.sensitivity.get(self.tune_row);
        self.sensitivity.set(self.tune_row, now * by);
        self.push_sensitivity();
    }

    fn push_sensitivity(&mut self) {
        for scene in &mut self.scenes {
            scene.tune(self.sensitivity);
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        self.scenes[self.current].render(area, f.buffer_mut());
        self.gestures.apply(area, f.buffer_mut());
        if self.settings_open {
            self.draw_settings(f, area);
        }
        if self.tune_open {
            self.draw_tune(f, area);
        }
    }

    /// One row per sensitivity entry: the full level being edited, the raw
    /// level nooise is reporting now, and the drive that pair yields.
    fn draw_tune(&self, f: &mut Frame, area: Rect) {
        let width = 64.min(area.width);
        let height = (Sensitivity::ROWS as u16 + 4).min(area.height);
        let panel = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let mut lines = Vec::with_capacity(Sensitivity::ROWS + 2);
        lines.push(Line::from("   voice   full     level    drive"));
        for row in 0..Sensitivity::ROWS {
            let full = self.sensitivity.get(row);
            let level = Voice::ALL
                .get(row)
                .map(|v| self.voice_levels[*v as usize])
                .unwrap_or(self.level);
            let drive = Sensitivity::drive(full, level);
            let bar: String = (0..20)
                .map(|i| {
                    if (i as f32) < drive * 20.0 {
                        '█'
                    } else {
                        '░'
                    }
                })
                .collect();
            let cursor = if row == self.tune_row { '>' } else { ' ' };
            let text = format!(
                "{cursor} {:<7} {full:<8.4} {level:<8.4} {bar} {drive:.2}",
                Sensitivity::label(row)
            );
            let style = if row == self.tune_row {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Gray)
            };
            lines.push(Line::from(text).style(style));
        }
        lines.push(
            Line::from("↑↓ row  ←→ ×1.25  r reset  t/Esc close  q quit+print")
                .alignment(Alignment::Right),
        );
        f.render_widget(Clear, panel);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" tune ")
                    .style(Style::default().fg(Color::Gray).bg(Color::Black)),
            ),
            panel,
        );
    }

    fn draw_settings(&self, f: &mut Frame, area: Rect) {
        let width = 72.min(area.width);
        let height = 12.min(area.height);
        let panel = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let last = self
            .last_event
            .as_ref()
            .map(|e| format!("{e:?}"))
            .unwrap_or_else(|| "none yet".into());
        let lines = vec![
            Line::from(format!("listen   {}", self.listen)),
            Line::from(format!(
                "scene    {} ({}/{}, n/p or 1-9/0 to switch)",
                self.scenes[self.current].name(),
                self.current + 1,
                self.scenes.len()
            )),
            Line::from(format!(
                "beat     {:.2}   level {:.3}",
                self.beat, self.level
            )),
            Line::from(
                Voice::ALL
                    .iter()
                    .map(|v| format!("{} {:.3}", v.name(), self.voice_levels[*v as usize]))
                    .collect::<Vec<_>>()
                    .join("  "),
            ),
            Line::from(
                Gesture::ALL
                    .iter()
                    .map(|g| format!("{} {} {:.2}", g.key(), g.name(), self.gestures.amount(*g)))
                    .collect::<Vec<_>>()
                    .join("  "),
            ),
            Line::from(format!("received {}", self.received)),
            Line::from(format!("kicks    {}", self.kicks)),
            Line::from(format!("unknown  {}", self.unknown)),
            Line::from(format!("last     {last}")),
            Line::from("Tab/s close   t tune   q quit").alignment(Alignment::Right),
        ];
        f.render_widget(Clear, panel);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" foorm ")
                    .style(Style::default().fg(Color::Gray).bg(Color::Black)),
            ),
            panel,
        );
    }
}

pub fn run(listen: SocketAddr, events: Receiver<Event>) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = ratatui::Terminal::new(CrosstermBackend::new(stdout))?;

    let result = event_loop(&mut terminal, listen, events);

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    let sensitivity = result?;
    if sensitivity != Sensitivity::default() {
        println!(
            "tuned sensitivity (paste into scene.rs Default):\n{}",
            sensitivity.literal()
        );
    }
    Ok(())
}

fn event_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    listen: SocketAddr,
    events: Receiver<Event>,
) -> Result<Sensitivity, Box<dyn Error>> {
    let mut app = App {
        listen,
        scenes: scene::all(),
        current: 0,
        settings_open: false,
        tune_open: false,
        tune_row: 0,
        sensitivity: Sensitivity::default(),
        gestures: Gestures::default(),
        received: 0,
        kicks: 0,
        unknown: 0,
        last_event: None,
        beat: 0.0,
        level: 0.0,
        voice_levels: [0.0; Voice::ALL.len()],
    };
    let mut last_frame = Instant::now();
    loop {
        for event in events.try_iter() {
            app.absorb(event);
        }
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32();
        let size = terminal.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        for scene in &mut app.scenes {
            scene.resize(area);
            scene.tick(dt);
        }
        last_frame = now;
        terminal.draw(|f| app.draw(f))?;

        let deadline = now + FRAME_INTERVAL;
        while let Some(timeout) = deadline.checked_duration_since(Instant::now()) {
            if !event::poll(timeout)? {
                break;
            }
            if let TermEvent::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && !app.on_key(key)
            {
                return Ok(app.sensitivity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_keep_their_scenes_and_navigation_reaches_every_scene() {
        let mut app = App {
            listen: "127.0.0.1:9000".parse().unwrap(),
            scenes: scene::all(),
            current: 0,
            settings_open: false,
            tune_open: false,
            tune_row: 0,
            sensitivity: Sensitivity::default(),
            gestures: Gestures::default(),
            received: 0,
            kicks: 0,
            unknown: 0,
            last_event: None,
            beat: 0.0,
            level: 0.0,
            voice_levels: [0.0; Voice::ALL.len()],
        };
        for (i, c) in "1234567890".chars().enumerate() {
            assert!(app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
            assert_eq!(app.current, i);
        }
        assert_eq!(app.scenes[app.current].name(), "atlas");
        app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.current, 10);
        for expected in (11..app.scenes.len()).chain(0..=9) {
            app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
            assert_eq!(app.current, expected);
        }
        for expected in (0..9).rev().chain((9..app.scenes.len()).rev()) {
            app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
            assert_eq!(app.current, expected);
        }
        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.current, 8);
    }
}
