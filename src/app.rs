//! The terminal loop: drain events into the scene, animate at a fixed frame
//! rate, draw the scene full-screen, and overlay settings on request.

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

use crate::osc::Event;
use crate::scene::{Pulse, Scene};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

struct App {
    listen: SocketAddr,
    scene: Box<dyn Scene>,
    settings_open: bool,
    received: u64,
    kicks: u64,
    unknown: u64,
    last_event: Option<Event>,
    beat: f32,
}

impl App {
    fn absorb(&mut self, event: Event) {
        self.received += 1;
        match &event {
            Event::Kick => self.kicks += 1,
            Event::Beat(b) => self.beat = *b,
            Event::Unknown(_) => self.unknown += 1,
            Event::Chord(_) => {}
        }
        self.scene.on_event(&event);
        self.last_event = Some(event);
    }

    /// Returns false when the user asked to quit. Raw mode turns Ctrl+C into
    /// a key event rather than a signal, so it is honoured here.
    fn on_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc if !self.settings_open => return false,
            KeyCode::Esc => self.settings_open = false,
            KeyCode::Tab | KeyCode::Char('s') => self.settings_open = !self.settings_open,
            _ => {}
        }
        true
    }

    fn draw(&self, f: &mut Frame) {
        let area = f.area();
        self.scene.render(area, f.buffer_mut());
        if self.settings_open {
            self.draw_settings(f, area);
        }
    }

    fn draw_settings(&self, f: &mut Frame, area: Rect) {
        let width = 44.min(area.width);
        let height = 10.min(area.height);
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
            Line::from(format!("scene    {}", self.scene.name())),
            Line::from(format!("beat     {:.2}", self.beat)),
            Line::from(format!("received {}", self.received)),
            Line::from(format!("kicks    {}", self.kicks)),
            Line::from(format!("unknown  {}", self.unknown)),
            Line::from(format!("last     {last}")),
            Line::from("Tab/s close   q quit").alignment(Alignment::Right),
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
    result
}

fn event_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    listen: SocketAddr,
    events: Receiver<Event>,
) -> Result<(), Box<dyn Error>> {
    let mut app = App {
        listen,
        scene: Box::new(Pulse::new()),
        settings_open: false,
        received: 0,
        kicks: 0,
        unknown: 0,
        last_event: None,
        beat: 0.0,
    };
    let mut last_frame = Instant::now();
    loop {
        for event in events.try_iter() {
            app.absorb(event);
        }
        let now = Instant::now();
        app.scene.tick(now.duration_since(last_frame).as_secs_f32());
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
                return Ok(());
            }
        }
    }
}
