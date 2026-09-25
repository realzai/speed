use std::{
    collections::VecDeque,
    io::{self, Stdout},
    time::{Duration, Instant},
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
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Wrap},
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

    loop {
        terminal.terminal.draw(|frame| draw(frame, &app))?;

        tokio::select! {
            _ = ticker.tick() => app.tick(),
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
    samples: Vec<(f64, f64)>,
    result: Option<TestResult>,
    error: Option<String>,
    history: VecDeque<TestResult>,
    animation: usize,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
    sender: tokio::sync::mpsc::UnboundedSender<Update>,
    updates: UnboundedReceiver<Update>,
    started: Instant,
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
            samples: Vec::new(),
            result: None,
            error: None,
            history: VecDeque::new(),
            animation: 0,
            task: None,
            sender,
            updates,
            started: Instant::now(),
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
        self.samples.clear();
        self.result = None;
        self.error = None;
        self.started = Instant::now();
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
                self.samples.push((elapsed.as_secs_f64(), speed_mbps));
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
    if area.width < 58 || area.height < 18 {
        frame.render_widget(
            Paragraph::new(
                "speed needs a terminal at least 58 × 18\n\nResize the window, or press q to quit.",
            )
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(CYAN)),
            area,
        );
        return;
    }

    let shell = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(45, 55, 72)))
        .title(Line::from(vec![
            Span::styled(
                " SPEED ",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled("// FAST.COM ", Style::default().fg(DIM)),
        ]));
    let inner = shell.inner(area);
    frame.render_widget(shell, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(9),
            Constraint::Length(3),
        ])
        .split(inner);

    draw_header(frame, rows[0], app);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(rows[1]);
    draw_speed(frame, body[0], app);
    draw_history(frame, body[1], app);
    draw_footer(frame, rows[2], app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let indicator = match app.status {
        Status::Running => format!(
            "{}  {}",
            FRAMES[(app.animation / 2) % FRAMES.len()],
            app.phase
        ),
        Status::Done => "●  Ready for another lap".to_owned(),
        Status::Failed => "!  Something got in the way".to_owned(),
    };
    let right = if app.status == Status::Running {
        format!("{:>4.1}s / 10s", app.elapsed.as_secs_f64())
    } else {
        "SPACE TO RUN".to_owned()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {indicator}"), Style::default().fg(CYAN)),
            Span::raw(
                " ".repeat(
                    area.width
                        .saturating_sub(indicator.len() as u16 + right.len() as u16 + 6)
                        as usize,
                ),
            ),
            Span::styled(right, Style::default().fg(DIM)),
        ]))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::Rgb(35, 45, 61))),
        ),
        area,
    );
}

fn draw_speed(frame: &mut Frame, area: Rect, app: &App) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(4)])
        .split(area);

    let speed_color = if app.status == Status::Failed {
        Color::Red
    } else {
        GREEN
    };
    let headline = match app.status {
        Status::Failed => "NO SIGNAL".to_owned(),
        _ => format!("{:.1}", app.speed),
    };
    let subtitle = match app.status {
        Status::Failed => app.error.as_deref().unwrap_or("Unknown network error"),
        Status::Done => connection_mood(app.speed),
        Status::Running if app.speed == 0.0 => "getting the runway clear…",
        Status::Running => "Mbps right now",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(speed_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(subtitle, Style::default().fg(DIM))),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(Color::Rgb(35, 45, 61))),
        ),
        sections[0],
    );

    let max_speed = app
        .samples
        .iter()
        .map(|(_, speed)| *speed)
        .fold(10.0, f64::max)
        * 1.15;
    let datasets = vec![
        Dataset::default()
            .name("live")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(PURPLE))
            .data(&app.samples),
    ];
    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::RIGHT)
                .border_style(Style::default().fg(Color::Rgb(35, 45, 61)))
                .title(Span::styled(" THROUGHPUT ", Style::default().fg(DIM))),
        )
        .x_axis(
            Axis::default()
                .bounds([0.0, 10.0])
                .style(Style::default().fg(Color::Rgb(45, 55, 72))),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, max_speed])
                .style(Style::default().fg(Color::Rgb(45, 55, 72))),
        );
    frame.render_widget(chart, sections[1]);
}

fn draw_history(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![Line::from(Span::styled(
        "  SESSION",
        Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
    ))];

    if let Some(result) = &app.result {
        lines.extend([
            Line::from(""),
            metric("PING", format!("{} ms", result.latency_ms)),
            metric("SERVER", result.server.clone()),
            metric("YOU", result.client.clone()),
            metric(
                "DATA",
                format!("{:.1} MB", result.bytes as f64 / 1_000_000.0),
            ),
        ]);
    } else if app.status == Status::Running {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "  Racing packets across",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  Netflix's network…",
                Style::default().fg(DIM),
            )),
        ]);
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  RECENT RUNS",
        Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    if app.history.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Laps appear here.",
            Style::default().fg(DIM),
        )));
    } else {
        for (index, result) in app.history.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:02}  ", index + 1), Style::default().fg(DIM)),
                Span::styled(
                    format!("{:>7.1} Mbps", result.speed_mbps),
                    Style::default().fg(GREEN),
                ),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn metric(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<7}"), Style::default().fg(DIM)),
        Span::styled(value, Style::default().fg(Color::White)),
    ])
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let runner = if app.status == Status::Running {
        let width = area.width.saturating_sub(34).max(1) as usize;
        let position = (app.animation / 2) % width;
        format!(
            "{}◆{}",
            "·".repeat(position),
            "·".repeat(width.saturating_sub(position + 1))
        )
    } else {
        "────────────────────".to_owned()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("  {runner}  "), Style::default().fg(PURPLE)),
            Span::styled(
                "SPACE/ENTER",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" rerun   ", Style::default().fg(DIM)),
            Span::styled("Q", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(" quit", Style::default().fg(DIM)),
        ]))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Rgb(35, 45, 61))),
        ),
        area,
    );
}

fn connection_mood(speed: f64) -> &'static str {
    match speed {
        speed if speed >= 500.0 => "warp speed — ridiculously fast",
        speed if speed >= 100.0 => "flying — 4K has room to spare",
        speed if speed >= 25.0 => "cruising — streaming looks great",
        speed if speed >= 10.0 => "steady — everyday browsing is covered",
        _ => "taking the scenic route",
    }
}
