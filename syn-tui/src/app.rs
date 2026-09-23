//! Drawing and key decoding. The loop itself lives in `syn-app`, so this crate
//! stays a view over a [`View`] snapshot and can be rendered into a test
//! backend without a terminal.

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;

use syn_audio::Frame;
use syn_core::schema::schema;
use syn_core::sim::Image;
use syn_core::state::AppState;

use crate::field::{ColorMode, FieldView};

/// What a key press means. The app decides what to do with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Quit,
    Up,
    Down,
    Enter,
    Escape,
    Delete,
    ToggleSound,
    Like,
    Dislike,
    Surprise,
    Undo,
    Save,
    Points,
    Details,
    Export,
    Help,
    /// Cycles what fills the screen: spectrum → picture → full-screen picture.
    Picture,
}

/// What fills the screen: the spectrum, or the picture in its place, or the
/// picture over the whole window. The two share the space because together
/// they fight — the spectrum's bars read as part of the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VizMode {
    Spectrum,
    Panel,
    Full,
}

impl VizMode {
    pub fn next(self) -> Self {
        match self {
            Self::Spectrum => Self::Panel,
            Self::Panel => Self::Full,
            Self::Full => Self::Spectrum,
        }
    }

    /// The config's spelling; anything unknown is the panel.
    pub fn parse(s: &str) -> Self {
        match s {
            "spectrum" | "off" => Self::Spectrum,
            "full" => Self::Full,
            _ => Self::Panel,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spectrum => "spectrum",
            Self::Panel => "panel",
            Self::Full => "full",
        }
    }

    /// How the status line names it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Spectrum => "spectrum",
            Self::Panel => "picture",
            Self::Full => "full-screen picture",
        }
    }
}

/// Everything the screen shows, gathered by the app for one draw.
pub struct View<'a> {
    pub name: &'a str,
    pub state: &'a AppState,
    pub frame: Frame,
    pub muted: bool,
    pub status: &'a str,
    pub show_help: bool,
    /// The kept points, when the list is open, and which row is selected.
    pub points: &'a [String],
    pub points_open: bool,
    pub selected: usize,
    /// The picture, where it goes, and how its colours are sent.
    pub picture: Option<&'a Image>,
    pub viz: VizMode,
    pub color: ColorMode,
}

pub fn decode(key: KeyEvent) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return Some(Action::Quit);
    }
    Some(match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Esc => Action::Escape,
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Enter => Action::Enter,
        KeyCode::Char('x') | KeyCode::Delete => Action::Delete,
        KeyCode::Char(' ') => Action::ToggleSound,
        KeyCode::Char('l') => Action::Like,
        KeyCode::Char('d') => Action::Dislike,
        KeyCode::Char('r') => Action::Surprise,
        KeyCode::Char('u') => Action::Undo,
        KeyCode::Char('s') => Action::Save,
        KeyCode::Char('p') => Action::Points,
        KeyCode::Char('i') => Action::Details,
        KeyCode::Char('e') => Action::Export,
        KeyCode::Char('?') | KeyCode::Char('h') => Action::Help,
        KeyCode::Char('v') => Action::Picture,
        _ => return None,
    })
}

/// Waits up to `timeout` for a key and decodes it.
pub fn poll_action(timeout: Duration) -> io::Result<Option<Action>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }
    match event::read()? {
        Event::Key(key) => Ok(decode(key)),
        _ => Ok(None),
    }
}

/// Raw mode and the alternate screen, released on drop even on a panic.
pub struct Screen {
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Screen {
    pub fn open() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        out.execute(EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok(Self { terminal })
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const SPECTRUM_ROWS: u16 = 5;
/// Below this the panel is not worth drawing.
const MIN_PICTURE_ROWS: u16 = 4;

fn bar(value: f64, width: usize) -> String {
    let filled = (value.clamp(0.0, 1.0) * width as f64).round() as usize;
    let mut s = "█".repeat(filled);
    s.push_str(&"·".repeat(width - filled));
    s
}

/// Draws the whole screen, and says where the picture went (its area in
/// cells), so the caller can size the picture to it.
pub fn draw(f: &mut ratatui::Frame, v: &View) -> Option<Rect> {
    let area = f.area();
    let field = if v.viz == VizMode::Full { draw_full(f, v, area) } else { draw_panel(f, v, area) };
    if let (Some(rect), Some(image)) = (field, v.picture) {
        f.render_widget(FieldView { image, mode: v.color }, rect);
    }

    if v.points_open {
        points_popup(f, area, v);
    }
    if v.show_help {
        help_popup(f, area);
    }
    field
}

/// The picture fills the window; one line underneath says what is playing.
fn draw_full(f: &mut ratatui::Frame, v: &View, area: Rect) -> Option<Rect> {
    let [picture, status] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    let line = format!(
        " {} {} · {} · [v] spectrum  [?] help",
        v.name,
        if v.muted { "· silent" } else { "· ♪" },
        v.status
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(line, Style::default().add_modifier(Modifier::DIM)))),
        status,
    );
    Some(picture)
}

/// The meters, with the picture (when it is on) between the spectrum and the
/// text; it gets whatever height the text leaves.
fn draw_panel(f: &mut ratatui::Frame, v: &View, area: Rect) -> Option<Rect> {
    let title = format!(" synesthesia — {} {} ", v.name, if v.muted { "· silent" } else { "· ♪" });
    let outer = Block::default().borders(Borders::ALL).title(title);
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let text = body_lines(v);
    let with_picture = v.viz == VizMode::Panel;
    let rows = Layout::vertical([
        Constraint::Length(2),                                            // meters
        Constraint::Length(if with_picture { 0 } else { SPECTRUM_ROWS }), // spectrum
        Constraint::Min(if with_picture { MIN_PICTURE_ROWS } else { 0 }), // picture
        if with_picture { Constraint::Length(text.len() as u16) } else { Constraint::Min(3) }, // formulas / lfos / fx
        Constraint::Length(1),                                                                 // status
        Constraint::Length(1),                                                                 // keys
    ])
    .split(inner);

    f.render_widget(meters(v), rows[0]);
    if !with_picture {
        f.render_widget(spectrum(v, rows[1]), rows[1]);
    }
    f.render_widget(Paragraph::new(text), rows[3]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(v.status, Style::default().fg(Color::Yellow)))),
        rows[4],
    );
    f.render_widget(keys(rows[5].width), rows[5]);
    (with_picture && rows[2].height >= MIN_PICTURE_ROWS).then_some(rows[2])
}

fn points_popup(f: &mut ratatui::Frame, area: Rect, v: &View) {
    let w = area.width.min(60);
    let h = area.height.min(4 + v.points.len() as u16).max(5);
    let popup =
        Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    let mut lines: Vec<Line> = Vec::new();
    if v.points.is_empty() {
        lines.push(Line::from("no points kept yet — press s to keep this one"));
    } else {
        for (i, name) in v.points.iter().enumerate() {
            let selected = i == v.selected;
            let text = format!("{} {}", if selected { "›" } else { " " }, name);
            let style =
                if selected { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
            lines.push(Line::from(Span::styled(text, style)));
        }
    }
    lines.push(Line::from(Span::styled(
        "↑↓ choose · enter load · x forget · esc close",
        Style::default().add_modifier(Modifier::DIM),
    )));
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" points ")),
        popup,
    );
}

fn meters(v: &View) -> Paragraph<'static> {
    let ft = v.frame.features;
    let swell = if ft.swell > 0.05 {
        format!("▲{:.2}", ft.swell)
    } else if ft.swell < -0.05 {
        format!("▼{:.2}", -ft.swell)
    } else {
        "  ·  ".into()
    };
    let limiter = if v.frame.limiter_db < -0.1 {
        format!("  limiter {:>5.1} dB", v.frame.limiter_db)
    } else {
        String::new()
    };
    Paragraph::new(vec![
        Line::from(format!(
            "loud {}  swell {}  bright {}  hits {}",
            bar(ft.loudness, 16),
            swell,
            bar(ft.brightness, 8),
            v.frame.hits
        )),
        Line::from(format!(
            "low {}  mid {}  high {}  peak {:.3}{}",
            bar(ft.low, 8),
            bar(ft.mid, 8),
            bar(ft.high, 8),
            v.frame.peak,
            limiter
        )),
    ])
}

fn spectrum(v: &View, area: Rect) -> Paragraph<'static> {
    let bands = &v.frame.spectrum;
    let width = (area.width as usize).min(bands.len());
    let rows = SPECTRUM_ROWS as usize;
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        // row 0 is the top: it shows the part of the bar above (rows-1-row)/rows
        let floor = (rows - 1 - row) as f64 / rows as f64;
        let line: String = (0..width)
            .map(|i| {
                let value = f64::from(bands[i * bands.len() / width]) / 255.0;
                let local = ((value - floor) * rows as f64).clamp(0.0, 1.0);
                BLOCKS[(local * 8.0).round() as usize]
            })
            .collect();
        lines.push(Line::from(Span::styled(line, Style::default().fg(Color::Cyan))));
    }
    Paragraph::new(lines)
}

fn body_lines(v: &View) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let mut formulas: Vec<String> = Vec::new();
    for id in &schema().formula_ids {
        if let Some(f) = v.state.audio.formulas.get(id) {
            if f.enabled {
                formulas.push(format!("{id} ●"));
            }
        }
    }
    lines.push(Line::from(format!("formulas  {}", formulas.join("  "))));

    let mod_state = &v.state.modulation;
    for (i, lfo) in mod_state.lfos.iter().enumerate() {
        let targets: Vec<String> = mod_state
            .routes
            .iter()
            .filter(|r| r.src == i)
            .map(|r| format!("{}.{}", r.target, r.param))
            .collect();
        if targets.is_empty() {
            continue;
        }
        let period = if lfo.rate > 0.0 { 1.0 / lfo.rate } else { f64::INFINITY };
        let phase = syn_core::lfo_value(lfo, v.frame.time);
        lines.push(Line::from(format!(
            "lfo {}  {:>6.1}s {:<8} {} → {}",
            i + 1,
            period,
            format!("{:?}", lfo.shape).to_lowercase(),
            bar((phase + 1.0) / 2.0, 10),
            targets.join(", ")
        )));
    }

    let fx = &v.state.audio.fx;
    let mut modules: Vec<&str> = Vec::new();
    if fx.filter_on {
        modules.push("filter");
    }
    if fx.chorus_on {
        modules.push("chorus");
    }
    if fx.phaser_on {
        modules.push("phaser");
    }
    if fx.delay_on {
        modules.push("delay");
    }
    if fx.reverb_on {
        modules.push("reverb");
    }
    if fx.limiter_on {
        modules.push("limiter");
    }
    lines.push(Line::from(format!(
        "fx  {}   master {:.2}   {:.0}s",
        if modules.is_empty() { "—".to_string() } else { modules.join("·") },
        v.state.audio.master_gain,
        v.frame.time
    )));
    lines
}

/// The key line, shortened when the terminal is narrow — a footer that runs
/// off the edge hides the one key people look for, `q`.
fn keys(width: u16) -> Paragraph<'static> {
    const FULL: &str = "[space] sound  [l] like  [d] dislike  [r] surprise  [u] undo  [s] save  [p] points  [v] spectrum/picture  [i] info  [?] help  [q] quit";
    const SHORT: &str = "[space] [l]ike [d]islike [r]andom [u]ndo [s]ave [p]oints [v]iew [i]nfo [?] [q]uit";
    let text = if usize::from(width) >= FULL.chars().count() { FULL } else { SHORT };
    Paragraph::new(Line::from(Span::styled(text, Style::default().add_modifier(Modifier::DIM))))
}

fn help_popup(f: &mut ratatui::Frame, area: Rect) {
    let w = area.width.min(64);
    let h = area.height.min(15);
    let popup =
        Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    let text = vec![
        Line::from("Sound and image from one point in a large parameter space."),
        Line::from(""),
        Line::from("space   start or silence the sound"),
        Line::from("l / d   more of this / not this — the search follows"),
        Line::from("r       surprise: jump near a random preset"),
        Line::from("u       undo the last step"),
        Line::from("s / p   save this point / open the points list"),
        Line::from("v       spectrum → picture → full-screen picture"),
        Line::from("i       what is this point"),
        Line::from("e       copy the point as a #s= token"),
        Line::from("q       quit"),
    ];
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" help ")),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use syn_core::state::presets;

    fn render(view: &View) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|f| {
                draw(f, view);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_screen_shows_the_point_and_its_formulas() {
        let preset = &presets()[0];
        let view = View {
            name: &preset.name,
            state: &preset.state,
            frame: Frame { hits: 7, ..Frame::default() },
            muted: false,
            status: "ready",
            show_help: false,
            points: &[],
            points_open: false,
            selected: 0,
            picture: None,
            viz: VizMode::Spectrum,
            color: ColorMode::TrueColor,
        };
        let text = render(&view);
        assert!(text.contains("Fractal garden"), "{text}");
        assert!(text.contains("formulas"), "{text}");
        assert!(text.contains("additive"), "{text}");
        assert!(text.contains("hits 7"), "{text}");
        assert!(text.contains("[q]"), "{text}");
    }

    #[test]
    fn help_covers_the_screen_when_asked() {
        let preset = &presets()[1];
        let view = View {
            name: &preset.name,
            state: &preset.state,
            frame: Frame::default(),
            muted: true,
            status: "",
            show_help: true,
            points: &[],
            points_open: false,
            selected: 0,
            picture: None,
            viz: VizMode::Spectrum,
            color: ColorMode::TrueColor,
        };
        let text = render(&view);
        assert!(text.contains("help"), "{text}");
        assert!(text.contains("undo the last step"), "{text}");
        assert!(text.contains("silent"), "{text}");
    }

    #[test]
    fn the_points_list_shows_what_is_kept() {
        let preset = &presets()[2];
        let points = vec!["first try".to_string(), "keeper".to_string()];
        let view = View {
            name: &preset.name,
            state: &preset.state,
            frame: Frame::default(),
            muted: false,
            status: "",
            show_help: false,
            points: &points,
            points_open: true,
            selected: 1,
            picture: None,
            viz: VizMode::Spectrum,
            color: ColorMode::TrueColor,
        };
        let text = render(&view);
        assert!(text.contains("points"), "{text}");
        assert!(text.contains("first try") && text.contains("keeper"), "{text}");
        assert!(text.contains("› keeper"), "the selected row is marked:\n{text}");
    }

    #[test]
    fn the_picture_takes_the_panel_or_the_whole_screen() {
        let preset = &presets()[0];
        let mut image = Image::new(4, 4);
        image.rgb.fill([10, 200, 30]);
        let view = |viz| View {
            name: &preset.name,
            state: &preset.state,
            frame: Frame::default(),
            muted: false,
            status: "",
            show_help: false,
            points: &[],
            points_open: false,
            selected: 0,
            picture: Some(&image),
            viz,
            color: ColorMode::TrueColor,
        };
        let placed = |viz| {
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            let mut area = None;
            terminal.draw(|f| area = draw(f, &view(viz))).unwrap();
            (area, terminal.backend().buffer().clone())
        };
        assert_eq!(placed(VizMode::Spectrum).0, None);
        let (panel, buf) = placed(VizMode::Panel);
        let panel = panel.expect("a 30-row screen has room for the panel");
        assert!(panel.height >= MIN_PICTURE_ROWS && panel.width == 98);
        assert_eq!(buf[(panel.x, panel.y)].fg, Color::Rgb(10, 200, 30));
        // The picture takes the spectrum's place: it starts right under the
        // two meter rows (border, then meters).
        assert_eq!(panel.y, 3);
        let (full, _) = placed(VizMode::Full);
        assert_eq!(full, Some(Rect::new(0, 0, 100, 29)));
        assert_eq!(VizMode::Spectrum.next().next().next(), VizMode::Spectrum);
        assert_eq!(VizMode::parse(VizMode::Full.as_str()), VizMode::Full);
    }

    #[test]
    fn keys_map_to_actions() {
        let press = |c: char| decode(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        assert_eq!(press('q'), Some(Action::Quit));
        assert_eq!(press(' '), Some(Action::ToggleSound));
        assert_eq!(press('l'), Some(Action::Like));
        assert_eq!(press('d'), Some(Action::Dislike));
        assert_eq!(press('x'), Some(Action::Delete));
        assert_eq!(press('v'), Some(Action::Picture));
        assert_eq!(press('z'), None);
        assert_eq!(decode(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), Some(Action::Escape));
        assert_eq!(decode(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)), Some(Action::Up));
        assert_eq!(decode(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Some(Action::Quit));
    }
}
