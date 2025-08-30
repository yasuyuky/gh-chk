use crate::cmd::prs::MergeStateStatus;
use crate::{graphql, rest};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use open;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::io;

// Type alias for GraphQL PR node for brevity (reuse prs module types)
type PrNode = crate::cmd::prs::repository::pull_requests::nodes::Nodes;

fn extract_reviewer_names(
    review_requests: &crate::cmd::prs::repository::pull_requests::nodes::review_requests::ReviewRequests,
) -> Vec<String> {
    review_requests
        .nodes
        .iter()
        .filter_map(|node| node.requested_reviewer.as_ref().map(ToString::to_string))
        .collect()
}

impl MergeStateStatus {
    fn to_color(&self) -> Color {
        match self {
            Self::Behind => Color::Yellow,
            Self::Blocked => Color::Red,
            Self::Clean => Color::Green,
            Self::Dirty => Color::Yellow,
            Self::Draft => Color::White,
            Self::HasHooks => Color::Yellow,
            Self::Unknown => Color::Magenta,
            Self::Unstable => Color::Yellow,
        }
    }
}

// Contributions GraphQL fetching and types are defined in contributions.rs
// Reuse PR GraphQL types from prs.rs to avoid duplication.

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    PrBodyRes {
        data: {
            repository: {
                pull_request: {
                    body_text: String,
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
enum SlugSpec {
    Owner(String),
    Repo { owner: String, name: String },
}

#[derive(Debug, Clone)]
struct PrData {
    pub id: String,
    pub number: usize,
    pub title: String,
    pub url: String,
    pub created_at: String,
    pub slug: String,
    pub merge_state_status: MergeStateStatus,
    pub reviewers: Vec<String>,
}

impl PrData {
    pub fn display_line(&self) -> String {
        let reviewers_str = if self.reviewers.is_empty() {
            String::default()
        } else {
            format!(" 👥 {}", self.reviewers.join(", "))
        };
        let created_date = self
            .created_at
            .split('T')
            .next()
            .unwrap_or(&self.created_at)
            .to_string();
        format!(
            "{} {} {} {}{} ({})",
            format!("#{}", self.number),
            self.merge_state_status.to_emoji(),
            self.slug,
            self.title,
            reviewers_str,
            created_date
        )
    }

    pub fn get_color(&self) -> Color {
        self.merge_state_status.to_color()
    }
}

fn split_slug(slug: &str) -> Option<(String, String)> {
    slug.split_once('/')
        .map(|(o, n)| (o.to_string(), n.to_string()))
}

fn make_pr_data(owner: &str, repo: &str, pr: &PrNode) -> PrData {
    PrData {
        id: pr.id.clone(),
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        created_at: pr.created_at.clone(),
        slug: format!("{}/{}", owner, repo),
        merge_state_status: pr.merge_state_status.clone(),
        reviewers: extract_reviewer_names(&pr.review_requests),
    }
}

async fn fetch_owner_prs(owner: &str) -> surf::Result<Vec<PrData>> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "variables": v });
    let res = graphql::query::<crate::cmd::prs::res::Res>(&q).await?;

    let prs = res
        .data
        .repository_owner
        .repositories
        .nodes
        .iter()
        .flat_map(|repo| {
            repo.pull_requests
                .nodes
                .iter()
                .map(move |pr| make_pr_data(owner, &repo.name, pr))
        })
        .collect();
    Ok(prs)
}

async fn fetch_repo_prs(owner: &str, name: &str) -> surf::Result<Vec<PrData>> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.repo.graphql"), "variables": v });
    let res = graphql::query::<crate::cmd::prs::repo_res::RepoRes>(&q).await?;
    Ok(res
        .data
        .repository_owner
        .repository
        .pull_requests
        .nodes
        .iter()
        .map(|pr| make_pr_data(owner, name, pr))
        .collect())
}

async fn approve_pr(pr_id: &str) -> surf::Result<()> {
    let v = json!({ "pullRequestId": pr_id });
    let q = json!({ "query": include_str!("../query/approve.pr.graphql"), "variables": v });
    graphql::query::<serde_json::Value>(&q).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewMode {
    Body,
    Diff,
}

struct App {
    prs: Vec<PrData>,
    list_state: ListState,
    should_quit: bool,
    status_message: Option<String>,
    specs: Vec<SlugSpec>,
    preview_open: bool,
    preview_cache: HashMap<String, String>,
    diff_cache: HashMap<String, String>,
    preview_mode: PreviewMode,
    preview_scroll: u16,
    preview_area_height: u16,
    contrib_lines: Option<Vec<Line<'static>>>,
    contrib_height: u16,
    contrib_title: String,
}

impl App {
    fn new(prs: Vec<PrData>, specs: Vec<SlugSpec>) -> App {
        let mut list_state = ListState::default();
        if !prs.is_empty() {
            list_state.select(Some(0));
        }
        App {
            prs,
            list_state,
            should_quit: false,
            status_message: None,
            specs,
            preview_open: false,
            preview_cache: HashMap::new(),
            diff_cache: HashMap::new(),
            preview_mode: PreviewMode::Body,
            preview_scroll: 0,
            preview_area_height: 0,
            contrib_lines: None,
            contrib_height: 9,
            contrib_title: "Contributions".to_string(),
        }
    }

    fn next(&mut self) {
        if self.prs.is_empty() {
            return;
        }
        let len = self.prs.len();
        let i = self
            .list_state
            .selected()
            .map(|i| (i + 1) % len)
            .unwrap_or(0);
        self.list_state.select(Some(i));
        self.preview_scroll = 0;
    }

    fn previous(&mut self) {
        if self.prs.is_empty() {
            return;
        }
        let len = self.prs.len();
        let i = self
            .list_state
            .selected()
            .map(|i| (i + len - 1) % len)
            .unwrap_or(0);
        self.list_state.select(Some(i));
        self.preview_scroll = 0;
    }

    fn get_selected_pr(&self) -> Option<&PrData> {
        self.list_state.selected().and_then(|i| self.prs.get(i))
    }

    async fn merge_selected(&mut self) {
        if let Some(selected_index) = self.list_state.selected() {
            if let Some(pr) = self.prs.get(selected_index).cloned() {
                if pr.merge_state_status == MergeStateStatus::Clean {
                    self.status_message =
                        Some(format!("Merging PR #{} in {}...", pr.number, pr.slug));
                    match crate::cmd::prs::merge_pr(&pr.id).await {
                        Ok(_) => {
                            self.status_message =
                                Some(format!("✅ Merged PR #{} in {}", pr.number, pr.slug));
                            self.prs.remove(selected_index);
                            if self.prs.is_empty() {
                                self.list_state.select(None);
                            } else if selected_index >= self.prs.len() {
                                self.list_state.select(Some(self.prs.len() - 1));
                            }
                        }
                        Err(e) => {
                            self.status_message = Some(format!(
                                "❌ Failed to merge PR #{} in {}: {}",
                                pr.number, pr.slug, e
                            ));
                        }
                    }
                } else {
                    self.status_message = Some(format!(
                        "Cannot merge PR #{} in {}: not in clean state",
                        pr.number, pr.slug
                    ));
                }
            }
        }
    }
    async fn approve_selected(&mut self) {
        if let Some(selected_index) = self.list_state.selected() {
            if let Some(pr) = self.prs.get(selected_index).cloned() {
                self.status_message =
                    Some(format!("Approving PR #{} in {}...", pr.number, pr.slug));
                match approve_pr(&pr.id).await {
                    Ok(_) => {
                        self.status_message =
                            Some(format!("✅ Approved PR #{} in {}", pr.number, pr.slug));
                    }
                    Err(e) => {
                        self.status_message = Some(format!(
                            "❌ Failed to approve PR #{} in {}: {}",
                            pr.number, pr.slug, e
                        ));
                    }
                }
            }
        }
    }

    fn open_url(&self) {
        if let Some(pr) = self.get_selected_pr() {
            if let Err(e) = open::that(&pr.url) {
                eprintln!("Failed to open URL: {}", e);
            }
        }
    }

    async fn toggle_preview(&mut self) {
        self.preview_open = !self.preview_open;
        if self.preview_open {
            self.preview_scroll = 0;
            self.maybe_prefetch_on_move().await;
        }
    }

    async fn maybe_prefetch_on_move(&mut self) {
        if !self.preview_open {
            return;
        }
        if let Some(pr) = self.get_selected_pr().cloned() {
            if !self.preview_cache.contains_key(&pr.id) {
                let _ = self.load_preview_for(&pr).await;
            }
            if self.preview_mode == PreviewMode::Diff && !self.diff_cache.contains_key(&pr.id) {
                let _ = self.load_diff_for(&pr).await;
            }
        }
    }

    async fn load_preview_for(&mut self, pr: &PrData) -> surf::Result<()> {
        self.status_message = Some(format!("🔎 Loading preview for #{}...", pr.number));
        let (owner, name) = match split_slug(&pr.slug) {
            Some(x) => x,
            None => return Ok(()),
        };
        let body = fetch_pr_body(&owner, &name, pr.number).await?;
        self.preview_cache.insert(pr.id.clone(), body);
        self.status_message = Some(format!("✅ Loaded preview for #{}", pr.number));
        Ok(())
    }

    async fn load_diff_for(&mut self, pr: &PrData) -> surf::Result<()> {
        self.status_message = Some(format!("🔎 Loading diff for #{}...", pr.number));
        let (owner, name) = match split_slug(&pr.slug) {
            Some(x) => x,
            None => return Ok(()),
        };
        let files = fetch_pr_files(&owner, &name, pr.number).await?;
        let mut out = String::default();
        for f in files {
            out.push_str(&format!(
                "=== {} (+{}, -{}) ===\n",
                f.filename, f.additions, f.deletions
            ));
            match f.patch {
                Some(p) => {
                    out.push_str(&p);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                None => out.push_str("(no textual diff available)\n"),
            }
            out.push('\n');
        }
        if out.is_empty() {
            out = "No file changes found.".to_string();
        }
        self.diff_cache.insert(pr.id.clone(), out);
        self.status_message = Some(format!("✅ Loaded diff for #{}", pr.number));
        Ok(())
    }

    async fn switch_preview_mode(&mut self, mode: PreviewMode) {
        self.preview_mode = mode;
        if !self.preview_open {
            self.preview_open = true;
        }
        self.preview_scroll = 0;
        if let Some(pr) = self.get_selected_pr().cloned() {
            match mode {
                PreviewMode::Body => {
                    if !self.preview_cache.contains_key(&pr.id) {
                        let _ = self.load_preview_for(&pr).await;
                    }
                }
                PreviewMode::Diff => {
                    if !self.diff_cache.contains_key(&pr.id) {
                        let _ = self.load_diff_for(&pr).await;
                    }
                }
            }
        }
    }

    fn scroll_preview_down(&mut self, n: u16) {
        if self.preview_open {
            self.preview_scroll = self.preview_scroll.saturating_add(n);
        }
    }
    fn scroll_preview_up(&mut self, n: u16) {
        if self.preview_open {
            self.preview_scroll = self.preview_scroll.saturating_sub(n);
        }
    }

    async fn reload(&mut self) {
        self.status_message = Some("🔄 Reloading...".to_string());
        let mut new_list: Vec<PrData> = Vec::new();
        let mut any_err: Option<String> = None;
        for spec in self.specs.clone() {
            match spec {
                SlugSpec::Owner(owner) => match fetch_owner_prs(&owner).await {
                    Ok(mut prs) => new_list.append(&mut prs),
                    Err(e) => any_err = Some(format!("Failed to fetch {}: {}", owner, e)),
                },
                SlugSpec::Repo { owner, name } => match fetch_repo_prs(&owner, &name).await {
                    Ok(mut prs) => new_list.append(&mut prs),
                    Err(e) => any_err = Some(format!("Failed to fetch {}/{}: {}", owner, name, e)),
                },
            }
        }
        if let Some(err) = any_err {
            self.status_message = Some(format!("❌ Reload error: {}", err));
        } else {
            let sel = self.list_state.selected().unwrap_or(0);
            self.prs = new_list;
            if self.prs.is_empty() {
                self.list_state.select(None);
            } else {
                let new_sel = sel.min(self.prs.len().saturating_sub(1));
                self.list_state.select(Some(new_sel));
            }
            self.status_message = Some(format!("✅ Reloaded. {} PRs.", self.prs.len()));
        }
        if let Err(e) = self.load_contributions().await {
            self.status_message = Some(format!("❌ Contrib load error: {}", e));
        }
    }

    async fn load_contributions(&mut self) -> surf::Result<()> {
        let login = crate::cmd::viewer::get().await?;
        let res = crate::cmd::contributions::fetch_calendar(&login).await?;
        let cal = &res.data.user.contributions_collection.contribution_calendar;
        let weeks = &cal.weeks;
        let mut lines: Vec<Line> = Vec::new();
        self.contrib_title = format!("Contributions: total {}", cal.total_contributions);
        for day in 0..7 {
            let mut spans: Vec<Span> = Vec::new();
            for w in weeks {
                if let Some(d) = w.contribution_days.get(day) {
                    let (r, g, b) = hex_to_rgb(&d.color);
                    let fg = contrast_fg(r, g, b);
                    let cnt = d.contribution_count;
                    let txt = if cnt >= 100 {
                        String::from("++")
                    } else {
                        format!("{:>2}", cnt)
                    };
                    spans.push(Span::styled(
                        txt,
                        Style::default().bg(Color::Rgb(r, g, b)).fg(fg),
                    ));
                } else {
                    spans.push(Span::raw("  "));
                }
            }
            lines.push(Line::from(spans));
        }
        self.contrib_lines = Some(lines);
        Ok(())
    }
}

fn hex_to_rgb(s: &str) -> (u8, u8, u8) {
    let hex = s.trim_start_matches('#');
    if hex.len() < 6 {
        return (0, 0, 0);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    (r, g, b)
}

fn contrast_fg(r: u8, g: u8, b: u8) -> Color {
    let r_f = r as f32 / 255.0;
    let g_f = g as f32 / 255.0;
    let b_f = b as f32 / 255.0;
    let lum = 0.2126 * r_f + 0.7152 * g_f + 0.0722 * b_f;
    if lum > 0.6 {
        Color::Black
    } else {
        Color::White
    }
}

fn make_diff_text(diff: &str) -> Text<'_> {
    let mut text = Text::default();
    for line in diff.lines() {
        let styled = if line.starts_with("===") {
            Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if line.starts_with("@@") {
            Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if line.starts_with('+') {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Green),
            ))
        } else if line.starts_with('-') {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(line.to_string())
        };
        text.lines.push(styled);
    }
    text
}

fn ui(f: &mut Frame, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(0),
                Constraint::Length(app.contrib_height),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.area());

    let main_chunks = if app.preview_open {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(outer[0])
    } else {
        vec![outer[0]].into()
    };

    let items: Vec<ListItem> = app
        .prs
        .iter()
        .map(|pr| {
            let line = pr.display_line();
            ListItem::new(Line::from(Span::styled(
                line,
                Style::default().fg(pr.get_color()),
            )))
        })
        .collect();

    let items = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Pull Requests"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(items, main_chunks[0], &mut app.list_state);

    if app.preview_open {
        let preview_text: Text = if let Some(pr) = app.get_selected_pr() {
            match app.preview_mode {
                PreviewMode::Body => match app.preview_cache.get(&pr.id) {
                    Some(body) => Text::from(format!("{}\n{}\n\n{}", pr.title, pr.url, body)),
                    None => Text::from("Loading preview..."),
                },
                PreviewMode::Diff => match app.diff_cache.get(&pr.id) {
                    Some(diff) => {
                        let header = format!("Diff for #{} {}", pr.number, pr.slug);
                        let mut text = Text::from(header);
                        text.lines.push(Line::from(""));
                        let mut colored = make_diff_text(diff);
                        text.lines.append(&mut colored.lines);
                        text
                    }
                    None => Text::from("Loading diff..."),
                },
            }
        } else {
            Text::from("No selection")
        };

        let preview = Paragraph::new(preview_text)
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .wrap(Wrap { trim: false })
            .scroll((app.preview_scroll, 0));
        let area = if main_chunks.len() > 1 {
            main_chunks[1]
        } else {
            outer[0]
        };
        app.preview_area_height = area.height;
        f.render_widget(preview, area);
    }

    // Contributions pane across full width in the middle row
    let contrib_block = Block::default()
        .borders(Borders::ALL)
        .title(app.contrib_title.clone());
    if let Some(lines) = &app.contrib_lines {
        // Fit contribution graph to the available inner width.
        // Each week cell occupies 2 columns (either "++" or right-padded 2-digit number or spaces).
        let area = outer[1];
        let inner_width = area.width.saturating_sub(2); // account for block borders
        let visible_weeks = (inner_width / 2) as usize;
        let mut trimmed: Vec<Line> = Vec::with_capacity(lines.len());
        for line in lines.iter() {
            let spans = &line.spans;
            let len = spans.len();
            let start = len.saturating_sub(visible_weeks);
            let slice: Vec<Span> = spans[start..len].to_vec();
            trimmed.push(Line::from(slice));
        }
        let contrib = Paragraph::new(trimmed)
            .block(contrib_block)
            .wrap(Wrap { trim: false });
        f.render_widget(contrib, outer[1]);
    } else {
        let contrib = Paragraph::new("Loading contributions...")
            .block(contrib_block)
            .wrap(Wrap { trim: true });
        f.render_widget(contrib, outer[1]);
    }

    let help_text = if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        let base =
            "q:quit • ?:help • Enter/o:open • m:merge • a:approve • r:reload • ←/→:list/body/diff";
        let nav = if app.preview_open {
            "↑/↓/wheel:scroll"
        } else {
            "↑/↓:navigate"
        };
        if app.preview_open {
            let mode = match app.preview_mode {
                PreviewMode::Body => "Body",
                PreviewMode::Diff => "Diff",
            };
            format!("{} • {} • mode:{}", base, nav, mode)
        } else {
            format!("{} • {}", base, nav)
        }
    };

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });

    f.render_widget(help, outer[2]);
}

async fn fetch_pr_body(owner: &str, name: &str, number: usize) -> surf::Result<String> {
    let vars = json!({ "owner": owner, "name": name, "number": number as i64 });
    let q = json!({ "query": include_str!("../query/pr.body.graphql"), "variables": vars });
    let res = graphql::query::<pr_body_res::PrBodyRes>(&q).await?;
    Ok(res.data.repository.pull_request.body_text)
}

#[derive(Deserialize)]
struct PrFileRes {
    filename: String,
    additions: i64,
    deletions: i64,
    patch: Option<String>,
}

struct PrFile {
    filename: String,
    additions: i64,
    deletions: i64,
    patch: Option<String>,
}

async fn fetch_pr_files(owner: &str, name: &str, number: usize) -> surf::Result<Vec<PrFile>> {
    let path = format!("repos/{}/{}/pulls/{}/files", owner, name, number);
    let q: rest::QueryMap = rest::QueryMap::default();
    let res: Vec<PrFileRes> = rest::get(&path, 1, &q).await?;
    Ok(res
        .into_iter()
        .map(|f| PrFile {
            filename: f.filename,
            additions: f.additions,
            deletions: f.deletions,
            patch: f.patch,
        })
        .collect())
}

fn run_tui(prs: Vec<PrData>, specs: Vec<SlugSpec>) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(prs, specs);
    let _ = async_std::task::block_on(app.load_contributions());
    let res = async_std::task::block_on(run_app(&mut terminal, &mut app));

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }
    Ok(())
}

async fn handle_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.preview_open {
                app.scroll_preview_down(1);
            } else {
                app.next();
                app.maybe_prefetch_on_move().await;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.preview_open {
                app.scroll_preview_up(1);
            } else {
                app.previous();
                app.maybe_prefetch_on_move().await;
            }
        }
        KeyCode::Enter | KeyCode::Char('o') => {
            app.open_url();
        }
        KeyCode::Char('m') => {
            app.merge_selected().await;
        }
        KeyCode::Char('a') => {
            app.approve_selected().await;
        }
        KeyCode::Char('r') => {
            app.reload().await;
        }
        KeyCode::Char('?') => {
            app.status_message = None;
        }
        KeyCode::Right => {
            // Right: closed -> Body, Body -> Diff
            if !app.preview_open {
                app.switch_preview_mode(PreviewMode::Body).await;
            } else if app.preview_mode == PreviewMode::Body {
                app.switch_preview_mode(PreviewMode::Diff).await;
            }
        }
        KeyCode::Left => {
            // Left: Diff -> Body -> Close
            if app.preview_open {
                match app.preview_mode {
                    PreviewMode::Diff => app.switch_preview_mode(PreviewMode::Body).await,
                    PreviewMode::Body => app.toggle_preview().await,
                }
            }
        }
        _ => {}
    }
}

fn handle_mouse(app: &mut App, kind: MouseEventKind) {
    match kind {
        MouseEventKind::ScrollDown => app.scroll_preview_down(3),
        MouseEventKind::ScrollUp => app.scroll_preview_up(3),
        _ => {}
    }
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => handle_key(app, key.code).await,
                Event::Mouse(m) => handle_mouse(app, m.kind),
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

pub async fn run(slugs: Vec<String>) -> surf::Result<()> {
    let slugs = if slugs.is_empty() {
        vec![crate::cmd::viewer::get().await?]
    } else {
        slugs
    };

    let mut specs: Vec<SlugSpec> = Vec::new();
    let mut all_prs = Vec::new();
    for slug in slugs.clone() {
        let vs: Vec<String> = slug.split('/').map(String::from).collect();
        match vs.len() {
            1 => {
                specs.push(SlugSpec::Owner(vs[0].clone()));
                let prs = fetch_owner_prs(&vs[0]).await?;
                all_prs.extend(prs);
            }
            2 => {
                specs.push(SlugSpec::Repo {
                    owner: vs[0].clone(),
                    name: vs[1].clone(),
                });
                let prs = fetch_repo_prs(&vs[0], &vs[1]).await?;
                all_prs.extend(prs);
            }
            _ => panic!("unknown slug format"),
        }
    }

    run_tui(all_prs, specs).map_err(|e| {
        surf::Error::from_str(
            surf::StatusCode::InternalServerError,
            format!("TUI error: {}", e),
        )
    })?;
    Ok(())
}
