use std::{
    collections::VecDeque,
    io::{self, Stdout},
    time::Duration,
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::speedtest::{self, Sample, TestResult, Update};

const CYAN: Color = Color::Rgb(59, 220, 255);
const PURPLE: Color = Color::Rgb(183, 110, 255);
const GREEN: Color = Color::Rgb(99, 255, 183);
const DIM: Color = Color::Rgb(105, 115, 135);
const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

pub async fn run() -> Result<()> {
    let mut terminal = TerminalGuard::new()?;
    let (input_tx, mut input_rx) = unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if input_tx.send(event).is_err() {
                break;
            }
        }
    });

    let (update_tx, update_rx) = unbounded_channel();
    let mut app = App::new(update_tx.clone(), update_rx);
    app.start_test();
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        terminal.terminal.draw(|frame| draw(frame, &app))?;

        tokio::select! {
            _ = ticker.tick(), if app.status == Status::Running => app.tick(),
            Some(event) = input_rx.recv() => {
                if let Event::Key(key) = event
                    && key.kind == KeyEventKind::Press
                {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Char('r') => app.start_test(),
                        _ => {}
                    }
                }
            }
            Some(update) = app.updates.recv() => app.apply(update),
        }
    }

    Ok(())
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Running,
    Done,
    Failed,
}

struct App {
    status: Status,
    phase: &'static str,
    speed: f64,
    elapsed: Duration,
    result: Option<TestResult>,
    error: Option<String>,
    history: VecDeque<TestResult>,
    animation: usize,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
    sender: tokio::sync::mpsc::UnboundedSender<Update>,
    updates: UnboundedReceiver<Update>,
}

impl App {
    fn new(
        sender: tokio::sync::mpsc::UnboundedSender<Update>,
        updates: UnboundedReceiver<Update>,
    ) -> Self {
        Self {
            status: Status::Running,
            phase: "Warming up",
            speed: 0.0,
            elapsed: Duration::ZERO,
            result: None,
            error: None,
            history: VecDeque::new(),
            animation: 0,
            task: None,
            sender,
            updates,
        }
    }

    fn start_test(&mut self) {
        if self.status == Status::Running && self.task.is_some() {
            return;
        }
        self.status = Status::Running;
        self.phase = "Warming up";
        self.speed = 0.0;
        self.elapsed = Duration::ZERO;
        self.result = None;
        self.error = None;
        self.task = Some(tokio::spawn(speedtest::run(self.sender.clone())));
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::Phase(phase) => self.phase = phase,
            Update::Sample(Sample {
                speed_mbps,
                elapsed,
            }) => {
                self.phase = "Measuring download speed";
                self.speed = speed_mbps;
                self.elapsed = elapsed;
            }
            Update::Finished(result) => {
                self.speed = result.speed_mbps;
                self.status = Status::Done;
                self.phase = "Run complete";
                self.history.push_front(result.clone());
                self.history.truncate(6);
                self.result = Some(result);
                self.task = None;
            }
            Update::Failed(error) => {
                self.status = Status::Failed;
                self.phase = "Test interrupted";
                self.error = Some(error);
                self.task = None;
            }
        }
    }

    fn tick(&mut self) {
        self.animation = self.animation.wrapping_add(1);
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let header_height = 1;
    let footer_height = u16::from(area.height >= 3);
    let recent_height = u16::from(area.height >= 5);
    let scene_height = area
        .height
        .saturating_sub(header_height + footer_height + recent_height);

    draw_header(
        frame,
        Rect::new(area.x, area.y, area.width, header_height),
        app,
    );
    if scene_height > 0 {
        draw_scene(
            frame,
            Rect::new(area.x, area.y + header_height, area.width, scene_height),
            app,
        );
    }
    if recent_height > 0 {
        draw_recent(
            frame,
            Rect::new(
                area.x,
                area.y + header_height + scene_height,
                area.width,
                recent_height,
            ),
            app,
        );
    }
    if footer_height > 0 {
        draw_footer(
            frame,
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        );
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let status = match app.status {
        Status::Running if area.width >= 42 => format!(
            "{} {}  {:>4.1}s",
            FRAMES[(app.animation / 2) % FRAMES.len()],
            app.phase,
            app.elapsed.as_secs_f64()
        ),
        Status::Running if area.width >= 18 => {
            format!("{} testing", FRAMES[(app.animation / 2) % FRAMES.len()])
        }
        Status::Running => FRAMES[(app.animation / 2) % FRAMES.len()].to_owned(),
        Status::Done => "● landed".to_owned(),
        Status::Failed => "! signal lost".to_owned(),
    };
    let name = if area.width >= 11 { " speed " } else { "speed" };
    let used = name.chars().count() + status.chars().count();
    let gap = " ".repeat((area.width as usize).saturating_sub(used));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(name, Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
            Span::raw(gap),
            Span::styled(status, Style::default().fg(DIM)),
        ])),
        area,
    );
}

fn draw_scene(frame: &mut Frame, area: Rect, app: &App) {
    match app.status {
        Status::Running => draw_flight(frame, area, app),
        Status::Done => draw_landed(frame, area, app),
        Status::Failed => draw_failure(frame, area, app),
    }
}

fn draw_flight(frame: &mut Frame, area: Rect, app: &App) {
    draw_stars(frame, area, app.animation);

    let speed_rows = big_speed_rows(app.speed);
    let speed_width = speed_rows
        .first()
        .map_or(0, |line| line.chars().count() as u16);
    if area.height >= 5 && speed_width <= area.width {
        let ship_height = area.height - 4;
        draw_ship(
            frame,
            Rect::new(area.x, area.y, area.width, ship_height),
            true,
            app.animation,
        );
        draw_big_speed(
            frame,
            Rect::new(area.x, area.bottom() - 4, area.width, 4),
            speed_rows,
        );
        return;
    }

    let compact = if app.speed > 0.0 {
        format!("current  {:.1} Mbps", app.speed)
    } else if area.width < 24 {
        "finding route…".to_owned()
    } else {
        "finding a clear route…".to_owned()
    };
    draw_centered(
        frame,
        Rect::new(area.x, area.y + area.height / 2, area.width, 1),
        compact,
        GREEN,
        true,
    );
}

fn draw_landed(frame: &mut Frame, area: Rect, app: &App) {
    draw_stars(frame, area, 0);
    let Some(result) = app.result.as_ref() else {
        return;
    };

    let speed_rows = big_speed_rows(result.speed_mbps);
    let speed_width = speed_rows
        .first()
        .map_or(0, |line| line.chars().count() as u16);
    if area.height >= 5 && speed_width <= area.width {
        let has_surface = area.height >= 6;
        let stats_height = 5 + u16::from(has_surface);
        let ship_height = area.height.saturating_sub(stats_height);
        if ship_height > 0 {
            draw_ship(
                frame,
                Rect::new(area.x, area.y, area.width, ship_height),
                false,
                app.animation,
            );
        }
        let speed_y = area.bottom() - stats_height;
        draw_big_speed(frame, Rect::new(area.x, speed_y, area.width, 4), speed_rows);
        draw_centered(
            frame,
            Rect::new(area.x, speed_y + 4, area.width, 1),
            format!(
                "session {:02} · {} ms · {}",
                app.history.len(),
                result.latency_ms,
                result.server
            ),
            DIM,
            false,
        );
        if has_surface {
            draw_centered(
                frame,
                Rect::new(area.x, speed_y + 5, area.width, 1),
                planet_surface(area.width),
                Color::Rgb(82, 144, 174),
                false,
            );
        }
        return;
    }

    if area.height <= 2 {
        draw_centered(
            frame,
            Rect::new(area.x, area.y, area.width, 1),
            format!("current  {:.1} Mbps", result.speed_mbps),
            GREEN,
            true,
        );
        if area.height == 2 {
            draw_centered(
                frame,
                Rect::new(area.x, area.y + 1, area.width, 1),
                format!("{} ms · {}", result.latency_ms, result.server),
                DIM,
                false,
            );
        }
        return;
    }

    draw_centered(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        format!("current  {:.1} Mbps", result.speed_mbps),
        GREEN,
        true,
    );
    draw_centered(
        frame,
        Rect::new(area.x, area.y + 1, area.width, 1),
        format!(
            "session {:02} · {} ms · {}",
            app.history.len(),
            result.latency_ms,
            result.server
        ),
        DIM,
        false,
    );
}

fn draw_ship(frame: &mut Frame, area: Rect, flying: bool, animation: usize) {
    if area.height == 0 {
        return;
    }
    if area.height < 4 || area.width < 28 {
        let craft = if flying {
            ["·  ≈≈[◇]▶", " ≈·≈≈[◇]▶", "·≈   ≈[◇]▶", "  ·≈≈≈[◇]▶"][(animation / 3) % 4]
        } else {
            "    [◇]▶"
        };
        draw_centered(
            frame,
            Rect::new(area.x, area.y + area.height / 2, area.width, 1),
            craft.to_owned(),
            CYAN,
            true,
        );
        return;
    }

    let art = [
        "       __       ",
        "  ____/  \\___   ",
        "<|    ◇      )▶ ",
        "  ‾‾‾\\____/‾‾   ",
    ];
    let bob = if flying {
        [0_i16, -1, -1, 0, 1, 1, 0, 0][(animation / 4) % 8]
    } else {
        0
    };
    let start_y = (area.y + area.height.saturating_sub(art.len() as u16) / 2)
        .saturating_add_signed(bob)
        .clamp(area.y, area.bottom().saturating_sub(art.len() as u16));
    let body_width = 16;
    let plume_width = 8;
    let group_width = body_width + u16::from(flying) * plume_width;
    let group_x = area.x + area.width.saturating_sub(group_width) / 2;
    let body_x = group_x + u16::from(flying) * plume_width;
    for (offset, line) in art.iter().enumerate() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                *line,
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            )),
            Rect::new(body_x, start_y + offset as u16, body_width, 1),
        );
    }
    if flying {
        let plume = [
            ["  ·≈    ", "≈≈≈≈≈≈≈≈", " ·   ≈  "],
            [" ·  ≈·  ", "≈≈≈≈≈≈≈ ", "   ·≈   "],
            ["   ≈ ·  ", "≈≈≈≈≈≈≈≈", " ·≈     "],
            [" ·≈     ", " ≈≈≈≈≈≈≈", "    ≈·  "],
        ][(animation / 3) % 4];
        for (offset, line) in plume.iter().enumerate() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    *line,
                    Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
                )),
                Rect::new(group_x, start_y + 1 + offset as u16, plume_width, 1),
            );
        }
    }
}

fn draw_big_speed(frame: &mut Frame, area: Rect, rows: Vec<String>) {
    let lines = rows
        .into_iter()
        .map(|row| {
            Line::from(Span::styled(
                row,
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ))
        })
        .chain(std::iter::once(Line::from(Span::styled(
            "CURRENT · Mbps",
            Style::default().fg(DIM),
        ))))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

fn big_speed_rows(speed: f64) -> Vec<String> {
    let value = format!("{speed:.1}");
    let mut rows = vec![String::new(), String::new(), String::new()];
    for (index, character) in value.chars().enumerate() {
        let glyph = speed_glyph(character);
        for (row, pattern) in rows.iter_mut().zip(glyph) {
            if index > 0 {
                row.push(' ');
            }
            row.push_str(pattern);
        }
    }
    rows
}

fn speed_glyph(character: char) -> [&'static str; 3] {
    match character {
        '0' => ["╭─╮", "│ │", "╰─╯"],
        '1' => [" ╷ ", " │ ", " ╵ "],
        '2' => ["╭─╮", "╭─╯", "╰─╴"],
        '3' => ["╭─╮", " ╶┤", "╰─╯"],
        '4' => ["╷ ╷", "╰─┤", "  ╵"],
        '5' => ["╭─╴", "╰─╮", "╰─╯"],
        '6' => ["╭─╴", "├─╮", "╰─╯"],
        '7' => ["╭─╮", "  │", "  ╵"],
        '8' => ["╭─╮", "├─┤", "╰─╯"],
        '9' => ["╭─╮", "╰─┤", "  ╵"],
        '.' => [" ", " ", "•"],
        _ => ["   ", "   ", "   "],
    }
}

fn draw_failure(frame: &mut Frame, area: Rect, app: &App) {
    draw_stars(frame, area, 0);
    let message = if area.width < 36 {
        "press space to retry"
    } else {
        app.error.as_deref().unwrap_or("Network test failed")
    };
    let middle = area.y + area.height / 2;
    draw_centered(
        frame,
        Rect::new(area.x, middle.saturating_sub(1), area.width, 1),
        "signal lost".to_owned(),
        Color::Red,
        true,
    );
    if area.height >= 2 {
        draw_centered(
            frame,
            Rect::new(area.x, middle, area.width, 1),
            message.to_owned(),
            DIM,
            false,
        );
    }
}

fn star_hash(x: usize, y: usize, salt: usize) -> usize {
    x.wrapping_mul(31 + salt) ^ y.wrapping_mul(73 + salt * 3) ^ salt.wrapping_mul(1_009)
}

fn star_at(column: u16, row: u16, animation: usize) -> char {
    let x = column as usize;
    let y = row as usize;
    let fast = star_hash(x + animation, y, 7);
    let medium = star_hash(x + animation / 3, y, 3);
    let distant = star_hash(x + animation / 8, y, 1);

    if fast.is_multiple_of(211) {
        '─'
    } else if medium.is_multiple_of(137) {
        '✦'
    } else if distant.is_multiple_of(59) {
        '·'
    } else if distant.is_multiple_of(83) {
        '.'
    } else {
        ' '
    }
}

fn draw_stars(frame: &mut Frame, area: Rect, animation: usize) {
    let lines = (0..area.height)
        .map(|row| {
            let stars = (0..area.width)
                .map(|column| star_at(column, row, animation))
                .collect::<String>();
            Line::from(stars)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(Color::Rgb(65, 72, 112))),
        area,
    );
}

fn draw_recent(frame: &mut Frame, area: Rect, app: &App) {
    let line = if app.history.is_empty() {
        Line::from(vec![
            Span::styled(" recent  ", Style::default().fg(PURPLE)),
            Span::styled(
                if area.width < 24 {
                    "—"
                } else {
                    "waiting for the first run"
                },
                Style::default().fg(DIM),
            ),
        ])
    } else {
        let mut spans = vec![Span::styled(
            " recent  ",
            Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
        )];
        for (index, result) in app.history.iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled("  ·  ", Style::default().fg(DIM)));
            }
            spans.push(Span::styled(
                format!("{:.1}", result.speed_mbps),
                Style::default().fg(GREEN),
            ));
        }
        spans.push(Span::styled(" Mbps", Style::default().fg(DIM)));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let controls = if area.width >= 38 {
        "space / enter  rerun     q  quit"
    } else if area.width >= 24 {
        "space rerun  ·  q quit"
    } else {
        "space · q"
    };
    frame.render_widget(
        Paragraph::new(controls)
            .alignment(Alignment::Center)
            .style(Style::default().fg(DIM)),
        area,
    );
}

fn draw_centered(frame: &mut Frame, area: Rect, text: String, color: Color, bold: bool) {
    let mut style = Style::default().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(style),
        area,
    );
}

fn planet_surface(width: u16) -> String {
    let dots = width.saturating_sub(8).clamp(3, 28) as usize;
    format!("· {} ·", "· ".repeat(dots / 2))
}
