// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// coord-tui — rapid-setup terminal UI for BoJ local-coord-mcp.
//
// Connects to the coord adapter on 127.0.0.1:7745, registers this shell
// session as a peer, then shows a live view of all active peers and task
// claims with fast keyboard dispatch.
//
// Architecture: pure state-transition functions are isolated (marked below)
// to support future SPARK/Ada formal verification via Idris2-ABI + Zig-FFI.

use std::{
    io,
    time::{Duration, Instant},
};

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};
use serde_json::{json, Value};

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "coord-tui", about = "BoJ coord-mcp rapid-setup TUI")]
struct Cli {
    /// Context label — disambiguates multiple windows of the same client.
    /// Defaults to $BOJ_COORD_CONTEXT or the current git repo name.
    #[arg(long, short, env = "BOJ_COORD_CONTEXT")]
    context: Option<String>,

    /// Client kind reported to the coord registry.
    #[arg(long, short, env = "BOJ_COORD_KIND", default_value = "claude")]
    kind: String,

    /// Coord adapter base URL.
    #[arg(long, env = "COORD_BACKEND_URL", default_value = "http://127.0.0.1:7745")]
    url: String,

    /// Silent registration mode: register, write ~/.cache/coord-tui/peer.env,
    /// print peer_id to stdout, then exit immediately (no TUI).
    /// Used by shell hooks triggered on tool launch.
    #[arg(long)]
    id: bool,
}

// ─── HTTP ─────────────────────────────────────────────────────────────────────

fn post(base: &str, tool: &str, body: &Value) -> Result<Value, String> {
    let endpoint = format!("{}/tools/{}", base, tool);
    let body_str = serde_json::to_string(body).unwrap_or_default();

    let raw = match ureq::post(&endpoint)
        .set("Content-Type", "application/json")
        .send_string(&body_str)
    {
        Ok(resp) => resp.into_string().map_err(|e| format!("read: {}", e))?,
        Err(ureq::Error::Status(_, resp)) => {
            resp.into_string().unwrap_or_else(|_| r#"{"success":false}"#.to_owned())
        }
        Err(e) => return Err(format!("network: {}", e)),
    };

    serde_json::from_str(&raw).map_err(|e| format!("json: {}", e))
}

// ─── Domain types ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
struct Peer {
    id: String,
    kind: String,
    state: String,
    context: String,
    variant: String,
    status: String,
}

#[derive(Clone, Debug, Default)]
struct Claim {
    task: String,
    holder: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Mode { Normal, Claiming, Statusing, Help }

#[derive(Clone, Debug, PartialEq)]
enum Focus { Peers, Claims }

// ─── Pure state helpers (SPARK-admittable) ────────────────────────────────────

fn next_index(current: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 { None } else { Some(current.map_or(0, |i| (i + 1).min(len - 1))) }
}

fn prev_index(current: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 { None } else { Some(current.map_or(0, |i| i.saturating_sub(1))) }
}

fn clamp_selection(current: Option<usize>, len: usize) -> Option<usize> {
    match (current, len) {
        (_, 0) => None,
        (None, _) => Some(0),
        (Some(i), n) if i >= n => Some(n - 1),
        (Some(i), _) => Some(i),
    }
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

// ─── App ──────────────────────────────────────────────────────────────────────

struct App {
    url: String,
    kind: String,
    context: String,
    peer_id: Option<String>,
    token: Option<String>,
    peers: Vec<Peer>,
    claims: Vec<Claim>,
    peer_sel: TableState,
    claim_sel: TableState,
    mode: Mode,
    focus: Focus,
    input: String,
    msg: String,
    last_refresh: Instant,
    sidebar_open: bool,
}

impl App {
    fn new(url: String, kind: String, context: String) -> Self {
        let mut peer_sel = TableState::default();
        peer_sel.select(Some(0));
        let mut claim_sel = TableState::default();
        claim_sel.select(Some(0));
        App {
            url, kind, context,
            peer_id: None, token: None,
            peers: vec![], claims: vec![],
            peer_sel, claim_sel,
            mode: Mode::Normal, focus: Focus::Peers,
            input: String::new(),
            msg: String::from("Connecting…"),
            last_refresh: Instant::now() - Duration::from_secs(60),
            sidebar_open: true,
        }
    }

    fn register(&mut self) {
        let body = json!({
            "client_kind": self.kind,
            "context":     self.context,
            "role":        "journeyman"
        });
        match post(&self.url, "coord_register", &body) {
            Ok(v) if v["success"].as_bool().unwrap_or(false) => {
                self.peer_id = v["peer_id"].as_str().map(String::from);
                self.token   = v["token"].as_str().map(String::from);
                let id = self.peer_id.as_deref().unwrap_or("?");
                self.msg = format!("registered as {}", id);
                let _ = execute!(std::io::stdout(), SetTitle(format!("coord-tui [{}]", id)));
                self.refresh();
            }
            Ok(v) => self.msg = format!("register failed: {}", str_field(&v, "error")),
            Err(e) => self.msg = format!("7745 unreachable — is `systemctl --user start local-coord-mcp` running? ({})", e),
        }
    }

    fn refresh(&mut self) {
        let token = match self.token.clone() { Some(t) => t, None => return };

        // Peers
        if let Ok(v) = post(&self.url, "coord_list_peers", &json!({"token": token})) {
            if let Some(arr) = v["peers"].as_array() {
                self.peers = arr.iter().map(|p| Peer {
                    id:      str_field(p, "peer_id").to_owned(),
                    kind:    str_field(p, "kind").to_owned(),
                    state:   str_field(p, "state").to_owned(),
                    context: str_field(p, "context").to_owned(),
                    variant: str_field(p, "variant").to_owned(),
                    status:  str_field(p, "status").to_owned(),
                }).collect();
            }
        }

        // Claims via dedicated list endpoint
        if let Ok(v) = post(&self.url, "coord_list_claims", &json!({"token": token})) {
            self.claims = v["active_claims"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|c| Claim {
                    task:   str_field(c, "task").to_owned(),
                    holder: str_field(c, "holder").to_owned(),
                })
                .collect();
        }

        self.peer_sel.select(clamp_selection(self.peer_sel.selected(), self.peers.len()));
        self.claim_sel.select(clamp_selection(self.claim_sel.selected(), self.claims.len()));
        self.last_refresh = Instant::now();
    }

    // ── Commands ──────────────────────────────────────────────────────────────

    fn do_claim(&mut self) {
        let task = self.input.trim().to_owned();
        self.input.clear();
        if task.is_empty() { return; }
        let token = match self.token.clone() { Some(t) => t, None => return };

        match post(&self.url, "coord_claim_task", &json!({"token": token, "task": task})) {
            Ok(v) => {
                // Adapter returns {"success":true,"message":"granted"} or
                // {"success":false,"error":"task held by <peer>"}.
                let msg = str_field(&v, "message");
                if msg == "granted" {
                    self.msg = format!("✓ claimed: {}", task);
                } else {
                    let err = v["error"].as_str()
                        .unwrap_or_else(|| if msg.is_empty() { "denied" } else { msg });
                    self.msg = format!("✗ {}", err);
                }
                self.refresh();
            }
            Err(e) => self.msg = format!("claim error: {}", e),
        }
    }

    fn do_status(&mut self) {
        let status = self.input.trim().to_owned();
        self.input.clear();
        if status.is_empty() { return; }
        let token = match self.token.clone() { Some(t) => t, None => return };

        match post(&self.url, "coord_status", &json!({"token": token, "status": status})) {
            Ok(_) => { self.msg = format!("status: {}", status); self.refresh(); }
            Err(e) => self.msg = format!("status error: {}", e),
        }
    }

    fn do_progress(&mut self) {
        let token = match self.token.clone() { Some(t) => t, None => return };
        let my_id = self.peer_id.clone().unwrap_or_default();
        let idx = match self.claim_sel.selected() { Some(i) => i, None => return };
        let claim = match self.claims.get(idx) { Some(c) => c.clone(), None => return };

        if claim.holder != my_id {
            self.msg = format!("✗ not your claim (held by {})", claim.holder);
            return;
        }
        match post(&self.url, "coord_progress", &json!({"token": token, "task": claim.task})) {
            Ok(_) => self.msg = format!("♥ heartbeat: {}", claim.task),
            Err(e) => self.msg = format!("progress error: {}", e),
        }
    }

    // ── Navigation (pure-ish wrappers) ────────────────────────────────────────

    fn nav_down(&mut self) {
        match self.focus {
            Focus::Peers  => self.peer_sel.select(next_index(self.peer_sel.selected(), self.peers.len())),
            Focus::Claims => self.claim_sel.select(next_index(self.claim_sel.selected(), self.claims.len())),
        }
    }

    fn nav_up(&mut self) {
        match self.focus {
            Focus::Peers  => self.peer_sel.select(prev_index(self.peer_sel.selected(), self.peers.len())),
            Focus::Claims => self.claim_sel.select(prev_index(self.claim_sel.selected(), self.claims.len())),
        }
    }
}

// ─── Drawing ──────────────────────────────────────────────────────────────────

const HIGHLIGHT: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

const DIM: Style = Style::new().fg(Color::DarkGray);
const GREEN: Style = Style::new().fg(Color::Green);
const YELLOW: Style = Style::new().fg(Color::Yellow);
const CYAN_BOLD: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let show_sidebar = app.sidebar_open && area.width >= 80;

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Min(4),
        Constraint::Length(1),
    ]).split(area);

    draw_header(f, app, chunks[0]);
    draw_footer(f, app, chunks[3]);

    if show_sidebar {
        const SIDEBAR_W: u16 = 24;
        // Span sidebar across both the peers and claims rows.
        let mid = Rect::new(
            chunks[1].x, chunks[1].y,
            chunks[1].width, chunks[1].height + chunks[2].height,
        );
        let cols = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(SIDEBAR_W),
        ]).split(mid);
        let rows = Layout::vertical([
            Constraint::Min(6),
            Constraint::Min(4),
        ]).split(cols[0]);
        draw_peers(f, app, rows[0]);
        draw_claims(f, app, rows[1]);
        draw_sidebar(f, app, cols[1]);
    } else {
        draw_peers(f, app, chunks[1]);
        draw_claims(f, app, chunks[2]);
    }

    match app.mode {
        Mode::Claiming | Mode::Statusing => draw_input(f, app, area),
        Mode::Help => draw_help(f, app, area),
        Mode::Normal => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let dot = if app.token.is_some() {
        Span::styled("●", GREEN)
    } else {
        Span::styled("●", Style::new().fg(Color::Red))
    };
    let peer = app.peer_id.as_deref().unwrap_or("—");
    let secs = app.last_refresh.elapsed().as_secs();
    let age = if secs < 2 { "just now".to_owned() } else { format!("{}s ago", secs) };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" coord-tui ", CYAN_BOLD),
            Span::raw("│ peer: "),
            Span::styled(peer, YELLOW),
            Span::raw(format!("  ctx: {}  7745: ", app.context)),
            dot,
            Span::styled(format!("  refreshed {}", age), DIM),
        ])),
        area,
    );
}

fn draw_peers(f: &mut Frame, app: &App, area: Rect) {
    let my_id = app.peer_id.as_deref().unwrap_or("");
    let active_border = if app.focus == Focus::Peers {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };

    let header = Row::new(["PEER ID", "KIND", "STATE", "CTX", "STATUS"]).style(DIM);
    let rows: Vec<Row> = app.peers.iter().map(|p| {
        let mine = p.id == my_id;
        let id_cell = if mine {
            Cell::from(format!("{} ◀", p.id)).style(GREEN)
        } else {
            Cell::from(p.id.clone())
        };
        let st = if p.status.is_empty() { "—".to_owned() } else { p.status.clone() };
        let ctx = if p.context.is_empty() { "—".to_owned() } else { p.context.clone() };
        Row::new([id_cell, Cell::from(p.kind.clone()), Cell::from(p.state.clone()),
                  Cell::from(ctx), Cell::from(st)])
    }).collect();

    // Narrower columns when the sidebar is visible to stay within 80 cols.
    let widths = if app.sidebar_open {
        [Constraint::Length(18), Constraint::Length(8), Constraint::Length(8),
         Constraint::Length(10), Constraint::Min(8)]
    } else {
        [Constraint::Length(22), Constraint::Length(9), Constraint::Length(11),
         Constraint::Length(14), Constraint::Min(18)]
    };
    let mut state = app.peer_sel.clone();
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(header)
            .block(Block::bordered()
                .title(format!(" Peers ({}) ", app.peers.len()))
                .border_style(active_border))
            .row_highlight_style(HIGHLIGHT),
        area,
        &mut state,
    );
}

fn draw_claims(f: &mut Frame, app: &App, area: Rect) {
    let my_id = app.peer_id.as_deref().unwrap_or("");
    let active_border = if app.focus == Focus::Claims {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };

    let header = Row::new(["TASK", "HOLDER"]).style(DIM);
    let rows: Vec<Row> = app.claims.iter().map(|c| {
        let mine = c.holder == my_id;
        let holder_cell = if mine {
            Cell::from(format!("{} ◀ yours", c.holder)).style(GREEN)
        } else {
            Cell::from(c.holder.clone())
        };
        Row::new([Cell::from(c.task.clone()), holder_cell])
    }).collect();

    let mut state = app.claim_sel.clone();
    f.render_stateful_widget(
        Table::new(rows, [Constraint::Min(28), Constraint::Min(22)])
            .header(header)
            .block(Block::bordered()
                .title(format!(" Claims ({}) ", app.claims.len()))
                .border_style(active_border))
            .row_highlight_style(HIGHLIGHT),
        area,
        &mut state,
    );
}

fn draw_sidebar(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::bordered()
        .title(" Commands  [\\] hide ")
        .border_style(Style::new().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sec = |s: &'static str| Line::from(Span::styled(s, CYAN_BOLD));
    let key = |k: &'static str, d: &'static str| {
        Line::from(vec![Span::styled(k, HIGHLIGHT), Span::raw(d)])
    };
    let cmd = |s: &'static str| Line::from(Span::styled(s, Style::new().fg(Color::Gray)));
    let dim = |s: &'static str| Line::from(Span::styled(s, DIM));

    let lines: Vec<Line> = vec![
        sec(" TUI keys"),
        Line::from(""),
        key("  c  ", " claim task"),
        key("  s  ", " set status"),
        key("  p  ", " heartbeat"),
        key("  R  ", " refresh"),
        key(" Tab ", " switch panel"),
        key("  ?  ", " full help"),
        key("  q  ", " quit"),
        Line::from(""),
        sec(" Shell helpers"),
        Line::from(""),
        cmd("  coord-peers"),
        cmd("  coord-claims"),
        cmd("  coord-claim <task>"),
        cmd("  coord-status <s>"),
        cmd("  coord-whoami"),
        Line::from(""),
        sec(" just coord-*"),
        Line::from(""),
        cmd("  just coord"),
        cmd("  just coord-peers"),
        cmd("  just coord-claims"),
        cmd("  just coord-claim"),
        cmd("  just coord-status"),
        cmd("  just coord-health"),
        Line::from(""),
        sec(" Register"),
        Line::from(""),
        cmd("  claude / gemini"),
        cmd("  cursor / vibe"),
        dim("    (auto via hooks)"),
        cmd("  coord-tui --id"),
        dim("    --kind <kind>"),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let normal_keys;
    let keys: &str = match app.mode {
        Mode::Normal => {
            let cmd_hint = if app.sidebar_open { "[\\]hide" } else { "[\\]cmd" };
            normal_keys = format!(
                " [c]laim  [s]tatus  [p]rogress  [Tab]panel  [R]efresh  {}  [?]help  [q]uit ",
                cmd_hint
            );
            &normal_keys
        }
        Mode::Claiming | Mode::Statusing => " [Enter]confirm  [Esc]cancel ",
        Mode::Help => " [Esc] or [?] to close help ",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(keys, DIM),
            Span::raw("  "),
            Span::raw(&app.msg),
        ])),
        area,
    );
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let peer_id  = app.peer_id.as_deref().unwrap_or("(not registered)");
    let token    = app.token.as_deref().unwrap_or("");
    let tok_disp = if token.len() >= 8 { format!("{}…", &token[..8]) } else { "(none)".into() };

    let lines: Vec<Line> = vec![
        Line::from(vec![Span::styled(" Keys ", CYAN_BOLD)]),
        Line::from(""),
        Line::from(vec![Span::styled("  c  ", HIGHLIGHT), Span::raw(" claim a task (mutex — first wins)")]),
        Line::from(vec![Span::styled("  s  ", HIGHLIGHT), Span::raw(" set your status text")]),
        Line::from(vec![Span::styled("  p  ", HIGHLIGHT), Span::raw(" heartbeat on selected claim (keep-alive)")]),
        Line::from(vec![Span::styled(" Tab ", HIGHLIGHT), Span::raw(" switch focus: Peers / Claims")]),
        Line::from(vec![Span::styled("  R  ", HIGHLIGHT), Span::raw(" force refresh now")]),
        Line::from(vec![Span::styled("j/k ↑↓", HIGHLIGHT), Span::raw(" navigate rows")]),
        Line::from(vec![Span::styled("  q  ", HIGHLIGHT), Span::raw(" quit")]),
        Line::from(""),
        Line::from(vec![Span::styled(" Shell helpers (source coord-hooks.sh) ", CYAN_BOLD)]),
        Line::from(""),
        Line::from("  coord-peers           list all active peers"),
        Line::from("  coord-claims          list all active task claims"),
        Line::from("  coord-claim <task>    claim a task from the terminal"),
        Line::from("  coord-status <text>   set your status from the terminal"),
        Line::from("  coord-whoami          show your peer ID and token"),
        Line::from(""),
        Line::from(vec![Span::styled(" Register a new window ", CYAN_BOLD)]),
        Line::from(""),
        Line::from("  claude / gemini / vibe / cursor / codex  (auto via hooks)"),
        Line::from("  coord-tui --id --kind claude              (manual)"),
        Line::from("  coord-tui --id --kind vibe --context vibe (for Vibe/IDE)"),
        Line::from(""),
        Line::from(vec![Span::styled(" This session ", CYAN_BOLD)]),
        Line::from(""),
        Line::from(format!("  Peer ID : {}", peer_id)),
        Line::from(format!("  Token   : {}", tok_disp)),
        Line::from(format!("  Adapter : {}", app.url)),
    ];

    let popup_h = (lines.len() + 4) as u16;
    let popup = centered(70, popup_h, area);
    let block = Block::bordered()
        .title(" Help — press ? or Esc to close ")
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(popup);
    f.render_widget(Clear, popup);
    f.render_widget(block, popup);
    f.render_widget(Paragraph::new(lines).style(Style::new().fg(Color::White)), inner);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let label = match app.mode {
        Mode::Claiming  => " Claim task (press Enter): ",
        Mode::Statusing => " Set status (press Enter): ",
        Mode::Normal | Mode::Help => unreachable!(),
    };
    let popup = centered(62, 3, area);
    let block = Block::bordered()
        .title(label)
        .border_style(Style::new().fg(Color::Yellow));
    let inner = block.inner(popup);
    f.render_widget(Clear, popup);
    f.render_widget(block, popup);
    f.render_widget(
        Paragraph::new(app.input.as_str()).style(Style::new().fg(Color::White)),
        inner,
    );
}

fn centered(pct_w: u16, h: u16, r: Rect) -> Rect {
    let w = (r.width as u32 * pct_w as u32 / 100).min(r.width as u32) as u16;
    let x = r.x + (r.width.saturating_sub(w)) / 2;
    let y = r.y + (r.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

// ─── Context detection ────────────────────────────────────────────────────────

fn detect_context() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .and_then(|o| o.status.success().then_some(o.stdout))
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|p| p.trim().split('/').last().map(String::from))
        .unwrap_or_else(|| "shell".to_owned())
}

// ─── Silent registration ──────────────────────────────────────────────────────

fn silent_register(url: &str, kind: &str, context: &str) {
    let body = json!({
        "client_kind": kind,
        "context":     context,
        "role":        "journeyman"
    });
    let Ok(v) = post(url, "coord_register", &body) else { return };
    if !v["success"].as_bool().unwrap_or(false) { return }

    let peer_id = v["peer_id"].as_str().unwrap_or("");
    let token   = v["token"].as_str().unwrap_or("");
    if peer_id.is_empty() { return }

    let home     = std::env::var("HOME").unwrap_or_default();
    let cache    = std::path::Path::new(&home).join(".cache").join("coord-tui");
    let _        = std::fs::create_dir_all(&cache);
    let env_path = cache.join("peer.env");
    let _        = std::fs::write(&env_path, format!(
        "BOJ_COORD_PEER_ID={peer_id}\nBOJ_COORD_TOKEN={token}\n"
    ));
    // Set the terminal window title to the peer ID so multi-window sessions
    // are identifiable at a glance in the taskbar / tab bar.
    let _ = execute!(std::io::stdout(), SetTitle(peer_id));
    println!("{peer_id}");
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let context = cli.context.unwrap_or_else(detect_context);

    if cli.id {
        silent_register(&cli.url, &cli.kind, &context);
        return Ok(());
    }

    let mut app = App::new(cli.url, cli.kind, context);
    app.register();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let poll_tick = Duration::from_millis(200);
    let auto_refresh = Duration::from_secs(5);

    loop {
        terminal.draw(|f| draw(f, &app))?;

        let needs_refresh = app.token.is_some()
            && app.last_refresh.elapsed() >= auto_refresh;

        if event::poll(poll_tick)? {
            if let Event::Key(key) = event::read()? {
                match &app.mode {
                    Mode::Normal => {
                        // Ctrl+C always exits
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c') {
                            break;
                        }
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Char('Q') => break,
                            KeyCode::Char('c') => {
                                app.input.clear();
                                app.mode = Mode::Claiming;
                            }
                            KeyCode::Char('s') => {
                                app.input.clear();
                                app.mode = Mode::Statusing;
                            }
                            KeyCode::Char('p') => app.do_progress(),
                            KeyCode::Char('R') => app.refresh(),
                            KeyCode::Tab => {
                                app.focus = if app.focus == Focus::Peers {
                                    Focus::Claims
                                } else {
                                    Focus::Peers
                                };
                            }
                            KeyCode::Down | KeyCode::Char('j') => app.nav_down(),
                            KeyCode::Up   | KeyCode::Char('k') => app.nav_up(),
                            KeyCode::Char('?') => { app.mode = Mode::Help; }
                            KeyCode::Char('\\') => { app.sidebar_open = !app.sidebar_open; }
                            _ => {}
                        }
                    }
                    Mode::Help => {
                        if key.code == KeyCode::Esc || key.code == KeyCode::Char('q')
                            || key.code == KeyCode::Char('?') {
                            app.mode = Mode::Normal;
                        }
                    }
                    Mode::Claiming | Mode::Statusing => match key.code {
                        KeyCode::Enter => {
                            let m = app.mode.clone();
                            app.mode = Mode::Normal;
                            if m == Mode::Claiming { app.do_claim(); }
                            else { app.do_status(); }
                        }
                        KeyCode::Esc => {
                            app.input.clear();
                            app.mode = Mode::Normal;
                        }
                        KeyCode::Backspace => { app.input.pop(); }
                        KeyCode::Char(c) => app.input.push(c),
                        _ => {}
                    },
                }
            }
        }

        if needs_refresh { app.refresh(); }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
