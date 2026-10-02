mod scan;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

const MINT: Color = Color::Rgb(126, 231, 193);
const GOLD: Color = Color::Rgb(255, 203, 124);
const DIM: Color = Color::Rgb(136, 154, 166);
const BG: Color = Color::Rgb(18, 29, 38);

fn human(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < 5 {
        value /= 1024.0;
        unit += 1;
    }
    let suffix = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"][unit];
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {suffix}")
    } else {
        format!("{value:.0} {suffix}")
    }
}

struct Screen {
    tree: scan::Scan,
    root: PathBuf,
    current: usize,
    selected: usize,
    history: Vec<(usize, usize)>,
}

impl Screen {
    fn children(&self) -> &[usize] {
        &self.tree.nodes[self.current].children
    }

    fn down(&mut self) {
        if !self.children().is_empty() {
            self.selected = (self.selected + 1).min(self.children().len() - 1);
        }
    }

    fn enter(&mut self) {
        if let Some(&index) = self.children().get(self.selected) {
            if self.tree.nodes[index].is_dir {
                self.history.push((self.current, self.selected));
                self.current = index;
                self.selected = 0;
            }
        }
    }

    fn back(&mut self) {
        if let Some((current, selected)) = self.history.pop() {
            self.current = current;
            self.selected = selected;
        }
    }

    fn path(&self) -> PathBuf {
        let mut names = Vec::new();
        let mut node = self.current;
        while let Some(parent) = self.tree.nodes[node].parent {
            names.push(&self.tree.nodes[node].name);
            node = parent;
        }
        let mut path = self.root.clone();
        for name in names.into_iter().rev() {
            path.push(name);
        }
        path
    }
}

fn render(frame: &mut ratatui::Frame, screen: Option<&Screen>, root: &Path, count: u64) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG)), area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(area);
    let heading = Line::from(vec![
        Span::styled(
            "  ◉  DISK EXPLORER  ",
            Style::default().fg(MINT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" /  find the heavy stuff", Style::default().fg(DIM)),
    ]);
    frame.render_widget(
        Paragraph::new(heading).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(DIM)),
        ),
        sections[0],
    );

    if let Some(screen) = screen {
        let node = &screen.tree.nodes[screen.current];
        let summary = format!(
            "  {}   •   {} files   •   {} folders",
            human(node.bytes),
            screen.tree.files,
            screen.tree.directories
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("  {}", screen.path().display()),
                    Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
                ),
                Line::styled(summary, Style::default().fg(DIM)),
            ]),
            sections[1],
        );

        let largest = screen
            .children()
            .first()
            .map(|&id| screen.tree.nodes[id].bytes)
            .unwrap_or(0);
        let visible = sections[2].height.saturating_sub(2) as usize;
        let start = screen.selected.saturating_sub(visible / 2);
        let end = screen.children().len().min(start.saturating_add(visible));
        let items: Vec<ListItem> = screen.children()[start..end]
            .iter()
            .map(|&id| {
                let child = &screen.tree.nodes[id];
                let filled = if largest == 0 {
                    0
                } else {
                    ((child.bytes as f64 / largest as f64) * 10.0).round() as usize
                }
                .min(10);
                let bar = format!("{}{}", "━".repeat(filled), "·".repeat(10 - filled));
                let name = child.name.to_string_lossy();
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:>9}  ", human(child.bytes)),
                        Style::default().fg(GOLD),
                    ),
                    Span::styled(bar, Style::default().fg(MINT)),
                    Span::styled(
                        if child.is_dir { "  ▸ " } else { "    " },
                        Style::default().fg(MINT),
                    ),
                    Span::raw(name),
                    Span::styled(
                        if child.unreadable { "  !" } else { "" },
                        Style::default().fg(GOLD),
                    ),
                ]))
            })
            .collect();
        let label = format!(" CONTENTS  ·  {} items ", screen.children().len());
        let list = List::new(items)
            .block(
                Block::default()
                    .title(label)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(DIM)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(38, 69, 75))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("❯");
        let mut state = ListState::default();
        if !screen.children().is_empty() {
            state.select(Some(screen.selected - start));
        }
        frame.render_stateful_widget(list, sections[2], &mut state);
        let notice = if screen.tree.errors > 0 {
            format!(
                "  ↑↓ move  ↵ open  ← back  q quit    •    {} unreadable entries; sizes partial",
                screen.tree.errors
            )
        } else {
            "  ↑↓ move  ↵ open  ← back  q quit    •    read-only · allocated space".to_string()
        };
        frame.render_widget(
            Paragraph::new(notice).style(Style::default().fg(DIM)),
            sections[3],
        );
    } else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("  Scanning {}", root.display()),
                    Style::default().fg(GOLD),
                ),
                Line::styled(
                    format!("  {} entries counted  ·  q to quit", count),
                    Style::default().fg(DIM),
                ),
            ]),
            sections[1],
        );
        frame.render_widget(
            Paragraph::new("  Measuring the landscape…").block(
                Block::default()
                    .title(" SCANNING ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(MINT)),
            ),
            sections[2],
        );
        frame.render_widget(
            Paragraph::new("  Read-only · symbolic links are not followed")
                .style(Style::default().fg(DIM)),
            sections[3],
        );
    }
}

struct TerminalGuard(Terminal<CrosstermBackend<Stdout>>);

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(err) = execute!(io::stdout(), EnterAlternateScreen) {
            disable_raw_mode()?;
            return Err(err);
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self(terminal)),
            Err(err) => {
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                let _ = disable_raw_mode();
                Err(err)
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn run(root: PathBuf) -> io::Result<()> {
    let progress = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&progress);
    let path = root.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(scan::scan(&path, &counter));
    });
    let mut terminal = TerminalGuard::new()?;
    let mut screen: Option<Screen> = None;
    loop {
        if screen.is_none() {
            match rx.try_recv() {
                Ok(result) => {
                    screen = Some(Screen {
                        tree: result?,
                        root: root.clone(),
                        current: 0,
                        selected: 0,
                        history: Vec::new(),
                    })
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(io::Error::other("scanner stopped unexpectedly"));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        terminal.0.draw(|frame| {
            render(
                frame,
                screen.as_ref(),
                &root,
                progress.load(Ordering::Relaxed),
            )
        })?;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    break;
                }
                if let Some(screen) = &mut screen {
                    match key.code {
                        KeyCode::Down | KeyCode::Char('j') => screen.down(),
                        KeyCode::Up | KeyCode::Char('k') => {
                            screen.selected = screen.selected.saturating_sub(1)
                        }
                        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => screen.enter(),
                        KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => screen.back(),
                        KeyCode::Home => screen.selected = 0,
                        KeyCode::End => screen.selected = screen.children().len().saturating_sub(1),
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = match args.next() {
        Some(value) if value == "--help" || value == "-h" => {
            println!(
                "disk-explorer [PATH]\n\nView allocated disk usage under PATH (defaults to your home directory).\nRead-only. Arrows or j/k navigate; Enter opens; Left goes back; q quits."
            );
            return Ok(());
        }
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())),
    };
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected at most one path",
        ));
    }
    let root = input.canonicalize()?;
    run(root)
}
