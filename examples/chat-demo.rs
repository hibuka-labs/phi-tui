//! Mini chat TUI: the smallest end-to-end `phi-tui` product.
//!
//! A fake echo "model" streams its reply through [`StreamState`] one chunk per
//! frame; committed lines land in a [`Transcript`], the reply renders as
//! markdown, and the viewport scrolls. This is the seed the phi-tui acceptance
//! talks about: a chat product needs phi-tui and nothing else.
//!
//! Run: `cargo run --example chat-demo`

use std::collections::VecDeque;
use std::io::{self, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap as RWrap},
};

use phi_tui::input::Composer;
use phi_tui::lines::LineKind;
use phi_tui::markdown::render_markdown;
use phi_tui::stream::StreamState;
use phi_tui::transcript::{DEFAULT_WRAP_WIDTH, Transcript};
use phi_tui::viewport::Viewport;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// The echo "model": streams back a fixed markdown reply, chunk by chunk —
/// stand-in for an LLM `TextDelta` stream.
const REPLY: &str = "## Echo\n\nYou said: **{msg}**\n\n- streaming works\n- markdown works\n";

fn main() -> io::Result<()> {
    let mut terminal = setup()?;
    let res = run(&mut terminal);
    teardown(&mut terminal)?;
    res
}

fn run(terminal: &mut Tui) -> io::Result<()> {
    let mut transcript: Transcript = Transcript::new();
    let mut viewport = Viewport::new();
    let mut composer = Composer::new();
    let mut stream: StreamState = StreamState::new(DEFAULT_WRAP_WIDTH);
    // Fake-model feed: one chunk per frame makes the streaming visible.
    let mut feed: VecDeque<String> = VecDeque::new();

    transcript.push_system("phi-tui chat-demo — type a line, Enter to send, Ctrl+C to quit");

    loop {
        // Feed the fake stream one delta per frame, then commit when done.
        if let Some(chunk) = feed.pop_front() {
            stream.push_text(&chunk, None);
        } else if stream.has_pending_text() {
            transcript.extend(stream.flush());
        }

        terminal.draw(|f| ui(f, &transcript, &mut viewport, &composer, &stream))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Enter => {
                    let msg = composer.text();
                    composer.clear();
                    if !msg.trim().is_empty() {
                        transcript.push_user(&msg);
                        feed.extend(chunks(&REPLY.replace("{msg}", &msg), 3));
                    }
                }
                KeyCode::Backspace => composer.backspace(),
                KeyCode::Char(c) => composer.insert_char(c),
                KeyCode::PageUp => {
                    viewport.scroll_up(5);
                }
                KeyCode::PageDown => {
                    viewport.scroll_down(5);
                }
                _ => {}
            }
        }
    }
}

/// Split into `n`-char chunks (fake token deltas).
fn chunks(s: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(n).map(|c| c.iter().collect()).collect()
}

fn ui(
    f: &mut Frame,
    transcript: &Transcript,
    viewport: &mut Viewport,
    composer: &Composer,
    stream: &StreamState,
) {
    let area = f.area();
    let composer_h = (composer.visual_height(area.width.saturating_sub(2)) as u16).min(4) + 2;
    let out_h = area.height.saturating_sub(composer_h + 1) as usize;

    let out = Rect {
        height: area.height.saturating_sub(composer_h + 1),
        ..area
    };
    let comp = Rect {
        y: out.y + out.height,
        height: composer_h,
        ..area
    };
    let status = Rect {
        y: comp.y + comp.height,
        height: 1,
        ..area
    };

    // Visible window = committed lines in view + the live streaming tail.
    let window = viewport.window_range(transcript.len(), out_h.max(1));
    viewport.set_visible(transcript.len(), out_h.max(1));

    let mut lines: Vec<Line> = Vec::new();
    for line in &transcript.output[window] {
        match line.kind {
            LineKind::Normal => lines.extend(render_markdown(&line.text)),
            _ => lines.push(Line::from(line.text.clone())),
        }
    }
    if let Some((raw, kind)) = stream.tail_raw() {
        if kind == LineKind::Normal {
            lines.extend(render_markdown(raw));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(RWrap { trim: false }), out);

    let input = Line::from(format!("❯ {}", composer.text()));
    f.render_widget(
        Paragraph::new(input).block(Block::default().borders(Borders::ALL)),
        comp,
    );
    f.render_widget(
        Paragraph::new("type · Enter send · PgUp/PgDn scroll · Ctrl+C quit"),
        status,
    );
}

fn setup() -> io::Result<Tui> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn teardown(terminal: &mut Tui) -> io::Result<()> {
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    crossterm::terminal::disable_raw_mode()?;
    terminal.show_cursor()
}
