use crate::cmd::prs::{self, Commit, CommitGraphEntry, MergeStateStatus, approve_pr, fetch_prs};
use crate::cmd::search::{SearchItem, search_code};
use crate::{slug::Slug, styling};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use open;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

// Type alias for GraphQL PR node for brevity (reuse prs module types)
type PrNode = prs::pull_request::PullRequest;
const SEARCH_HISTORY_LIMIT: usize = 100;

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

#[derive(Debug, Clone, Copy, Default)]
struct Preview {
    mode: Option<PreviewMode>,
    scroll: u16,
    height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PreviewMode {
    Body,
    Diff,
    Commits,
}

impl std::fmt::Display for PreviewMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_str = match self {
            Self::Body => "Body",
            Self::Diff => "Diff",
            Self::Commits => "Commits",
        };
        f.write_str(as_str)
    }
}

#[derive(Debug, Clone)]
enum PendingTask {
    MergeSelected,
    ApproveSelected,
    Reload,
    ReloadSelected,
    ReloadContrib,
    LoadBodyForSelected,
    LoadDiffForSelected,
    LoadCommitsForSelected,
    SearchCode { owner: String, query: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Prs,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchFocus {
    Input,
    Results,
}

struct SearchState {
    owner: String,
    query: String,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    results: Vec<SearchItem>,
    list_state: ListState,
    focus: SearchFocus,
    preview_open: bool,
}

impl SearchState {
    fn new(owner: String, history: Vec<String>) -> Self {
        Self {
            owner,
            query: String::default(),
            history,
            history_index: None,
            history_draft: String::default(),
            results: Vec::new(),
            list_state: ListState::default(),
            focus: SearchFocus::Input,
            preview_open: false,
        }
    }

    fn navigate(&mut self, d: isize) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len() as isize;
        let i = (self.list_state.selected().unwrap_or(0) as isize + d).rem_euclid(len);
        self.list_state.select(Some(i as usize));
    }

    fn select_first(&mut self) {
        if self.results.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn remember_query(&mut self, query: &str) -> bool {
        let changed = push_search_history(&mut self.history, query);
        self.history_index = None;
        self.history_draft.clear();
        changed
    }

    fn reset_history_nav(&mut self) {
        self.history_index = None;
        self.history_draft.clear();
    }

    fn navigate_history(&mut self, d: isize) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.history_index, d) {
            (None, -1) => {
                self.history_draft = self.query.clone();
                Some(self.history.len() - 1)
            }
            (None, 1) => None,
            (Some(0), -1) => Some(0),
            (Some(i), -1) => Some(i - 1),
            (Some(i), 1) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), 1) => {
                self.history_index = None;
                self.query = std::mem::take(&mut self.history_draft);
                return;
            }
            (current, _) => current,
        };
        let Some(next) = next else {
            return;
        };
        self.history_index = Some(next);
        self.query = self.history[next].clone();
    }
}

fn search_history_path() -> PathBuf {
    let mut path = crate::config::CONFIG_PATH.clone();
    path.set_file_name("search_history");
    path
}

fn load_search_history(path: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut history = Vec::new();
    for line in content.lines() {
        push_search_history(&mut history, line);
    }
    history
}

fn save_search_history(path: &Path, history: &[String]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut content = history.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    fs::write(path, content)
}

fn push_search_history(history: &mut Vec<String>, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    let before = history.clone();
    history.retain(|item| item != query);
    history.push(query.to_string());
    let excess = history.len().saturating_sub(SEARCH_HISTORY_LIMIT);
    if excess > 0 {
        history.drain(0..excess);
    }
    *history != before
}

fn is_search_back_key(focus: SearchFocus, code: KeyCode) -> bool {
    matches!(
        (focus, code),
        (SearchFocus::Input, KeyCode::Esc) | (SearchFocus::Results, KeyCode::Char('q'))
    )
}

#[cfg(target_os = "macos")]
const CLIPBOARD_COMMANDS: &[(&str, &[&str])] = &[("pbcopy", &[])];

#[cfg(target_os = "windows")]
const CLIPBOARD_COMMANDS: &[(&str, &[&str])] = &[("clip", &[])];

#[cfg(all(unix, not(target_os = "macos")))]
const CLIPBOARD_COMMANDS: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

fn copy_to_clipboard(text: &str) -> io::Result<()> {
    let mut last_error = None;
    for (program, args) in CLIPBOARD_COMMANDS {
        match run_clipboard_command(program, args, text) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "clipboard unavailable")))
}

fn run_clipboard_command(program: &str, args: &[&str], text: &str) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // Many clipboard tools read until EOF, so close stdin before wait().
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "clipboard command failed: {program}"
        )))
    }
}

struct App {
    prs: Vec<PrNode>,
    list_state: ListState,
    should_quit: bool,
    status_message: Option<String>,
    status_clear_at: Option<Instant>,
    specs: Vec<Slug>,
    cache: HashMap<(PreviewMode, String), Text<'static>>, // (mode, pr_id) -> content
    preview: Preview,
    contrib_lines: Option<Vec<Line<'static>>>,
    contrib_stats: Option<Vec<Line<'static>>>,
    contrib_height: u16,
    contrib_title: String,
    pending_task: Option<PendingTask>,
    mode: AppMode,
    search: SearchState,
}

impl App {
    async fn new(specs: Vec<Slug>) -> surf::Result<App> {
        let prs = fetch_prs(&specs).await?;
        let mut list_state = ListState::default();
        if !prs.is_empty() {
            list_state.select(Some(0));
        }
        let search_owner = App::default_search_owner(&specs);
        let search_history = load_search_history(&search_history_path());
        Ok(App {
            prs,
            list_state,
            should_quit: false,
            status_message: None,
            status_clear_at: None,
            specs,
            cache: HashMap::new(),
            preview: Preview::default(),
            contrib_lines: None,
            contrib_stats: None,
            contrib_height: 9,
            contrib_title: "Contributions".to_string(),
            pending_task: None,
            mode: AppMode::Prs,
            search: SearchState::new(search_owner, search_history),
        })
    }

    fn default_search_owner(specs: &[Slug]) -> String {
        specs
            .first()
            .map(|slug| match slug {
                Slug::Owner(owner) => owner.clone(),
                Slug::Repo { owner, .. } => owner.clone(),
            })
            .unwrap_or_default()
    }

    fn navigate(&mut self, d: isize) {
        if self.prs.is_empty() {
            return;
        }
        let i = (self.list_state.selected().unwrap_or(0) as isize + d) % self.prs.len() as isize;
        self.list_state.select(Some(i as usize));
        self.preview.scroll = 0;
    }

    fn get_selected_pr(&self) -> Option<&PrNode> {
        self.list_state.selected().and_then(|i| self.prs.get(i))
    }

    async fn merge_selected(&mut self) {
        if let Some(selected_index) = self.list_state.selected()
            && let Some(pr) = self.prs.get(selected_index).cloned()
        {
            if pr.merge_state_status == MergeStateStatus::Clean {
                self.set_status_persistent(format!("Merging PR {}...", pr.numslug()));
                match crate::cmd::prs::merge_pr(&pr.id).await {
                    Ok(_) => {
                        self.set_status_persistent(format!(
                            "✅ Merged PR {}. Reloading...",
                            pr.numslug()
                        ));
                        self.pending_task = Some(PendingTask::ReloadSelected);
                        // Reload contributions to reflect the newly merged PR
                        if let Err(e) = self.load_contributions().await {
                            self.set_status(format!("❌ Contrib load error: {}", e));
                        }
                    }
                    Err(e) => {
                        self.set_status(format!("❌ Failed to merge PR {}: {}", pr.numslug(), e));
                    }
                }
            } else {
                self.set_status(format!(
                    "Cannot merge PR {}: not in clean state",
                    pr.numslug()
                ));
            }
        }
    }
    async fn approve_selected(&mut self) {
        if let Some(selected_index) = self.list_state.selected()
            && let Some(pr) = self.prs.get(selected_index).cloned()
        {
            self.set_status_persistent(format!("Approving PR {}...", pr.numslug()));
            match approve_pr(&pr.id).await {
                Ok(_) => {
                    self.set_status_persistent(format!(
                        "✅ Approved PR {}. Reloading...",
                        pr.numslug()
                    ));
                    self.pending_task = Some(PendingTask::ReloadSelected);
                }
                Err(e) => {
                    self.set_status(format!("❌ Failed to approve PR {}: {}", pr.numslug(), e));
                }
            }
        }
    }

    fn open_url(&self) {
        if let Some(pr) = self.get_selected_pr()
            && let Err(e) = open::that(&pr.url)
        {
            eprintln!("Failed to open URL: {}", e);
        }
    }

    async fn load_body(&mut self, pr: &PrNode) -> surf::Result<()> {
        self.set_status_persistent(format!("🔎 Loading body for #{}...", pr.number));
        let body: String =
            prs::fetch_pr_body(&pr.repository.owner.login, &pr.repository.name, pr.number).await?;
        let text = styling::prettify_pr_preview(&pr.title, &pr.url, &body);
        self.cache.insert((PreviewMode::Body, pr.id.clone()), text);
        self.set_status(format!("✅ Loaded body for #{}", pr.number));
        Ok(())
    }

    async fn load_diff(&mut self, pr: &PrNode) -> surf::Result<()> {
        self.set_status_persistent(format!("🔎 Loading diff for #{}...", pr.number));
        let files =
            prs::fetch_pr_diffs(&pr.repository.owner.login, &pr.repository.name, pr.number).await?;
        let mut out = String::default();
        for f in files {
            out += f.to_string().as_str();
        }
        if out.is_empty() {
            out = "No file changes found.".to_string();
        }
        let text = styling::make_diff_text(&out);
        self.cache.insert((PreviewMode::Diff, pr.id.clone()), text);
        self.set_status(format!("✅ Loaded diff for #{}", pr.number));
        Ok(())
    }

    async fn load_commits(&mut self, pr: &PrNode) -> surf::Result<()> {
        self.set_status_persistent(format!("🔎 Loading commits for #{}...", pr.number));
        let commits =
            prs::fetch_pr_commits(&pr.repository.owner.login, &pr.repository.name, pr.number)
                .await?;
        let entries = build_commit_graph_entries(&commits);
        let text = make_commit_graph_text(&entries); // pre-render to check for emptiness
        self.cache
            .insert((PreviewMode::Commits, pr.id.clone()), text);
        self.set_status(format!("✅ Loaded commits for #{}", pr.number));
        Ok(())
    }

    fn scroll_preview_down(&mut self, n: u16) {
        if self.preview.mode.is_some() {
            self.preview.scroll = self.preview.scroll.saturating_add(n);
        }
    }
    fn scroll_preview_up(&mut self, n: u16) {
        if self.preview.mode.is_some() {
            self.preview.scroll = self.preview.scroll.saturating_sub(n);
        }
    }

    async fn reload(&mut self) {
        let new_list = match fetch_prs(&self.specs).await {
            Ok(prs) => prs,
            Err(e) => {
                self.set_status(format!("❌ Reload error: {}", e));
                return;
            }
        };

        self.apply_pr_list_and_restore_selection(new_list);
        self.refresh_preview().await;
        self.set_status(format!("✅ Reloaded. {} PRs.", self.prs.len()));

        if let Err(e) = self.load_contributions().await {
            self.set_status(format!("❌ Contrib load error: {}", e));
        }
    }

    fn apply_pr_list_and_restore_selection(&mut self, new_list: Vec<PrNode>) {
        let sel = self.list_state.selected().unwrap_or(0);
        self.prs = new_list;
        if self.prs.is_empty() {
            self.list_state.select(None);
        } else {
            let new_sel = sel.min(self.prs.len().saturating_sub(1));
            self.list_state.select(Some(new_sel));
        }
        self.prune_cache_to_existing();
    }

    async fn refresh_preview(&mut self) {
        if let Some(mode) = self.preview.mode
            && let Some(pr) = self.get_selected_pr().cloned()
        {
            match mode {
                PreviewMode::Body => {
                    let _ = self.load_body(&pr).await;
                }
                PreviewMode::Diff => {
                    let _ = self.load_diff(&pr).await;
                }
                PreviewMode::Commits => {
                    let _ = self.load_commits(&pr).await;
                }
            }
        }
    }

    fn prune_cache_to_existing(&mut self) {
        let ids: HashSet<String> = self.prs.iter().map(|pr| pr.id.clone()).collect();
        self.cache.retain(|(_, pr_id), _| ids.contains(pr_id));
    }

    fn drop_preview_cache_for(&mut self, pr_id: &str) {
        let id = pr_id.to_string();
        for mode in [PreviewMode::Body, PreviewMode::Diff, PreviewMode::Commits] {
            self.cache.remove(&(mode, id.clone()));
        }
    }

    fn replace_repo_prs(
        &mut self,
        owner: &str,
        name: &str,
        repo_prs: &mut Vec<PrNode>,
        keep_selection: Option<String>,
    ) {
        let insert_at = self
            .prs
            .iter()
            .position(|pr| pr.repository.owner.login == owner && pr.repository.name == name)
            .unwrap_or(self.prs.len()); // skipcq: RS-W1031

        self.prs
            .retain(|pr| !(pr.repository.owner.login == owner && pr.repository.name == name));

        let mut incoming: Vec<PrNode> = Vec::new();
        incoming.append(repo_prs);
        self.prs.splice(insert_at..insert_at, incoming);

        self.prune_cache_to_existing();

        let selection = keep_selection
            .and_then(|id| self.prs.iter().position(|pr| pr.id == id))
            .or_else(|| {
                if self.prs.is_empty() {
                    None
                } else if insert_at >= self.prs.len() {
                    Some(self.prs.len() - 1)
                } else {
                    Some(insert_at)
                }
            });
        self.list_state.select(selection);
        self.preview.scroll = 0;
    }

    async fn reload_selected_pr(&mut self) {
        let Some(selected_index) = self.list_state.selected() else {
            return;
        };
        let Some(pr) = self.prs.get(selected_index).cloned() else {
            return;
        };

        let owner = pr.repository.owner.login.clone();
        let name = pr.repository.name.clone();
        self.set_status_persistent(format!("🔄 Reloading {}...", pr.numslug()));
        match prs::fetch_repo_prs(&owner, &name).await {
            Ok(mut repo_prs) => {
                self.replace_repo_prs(&owner, &name, &mut repo_prs, Some(pr.id.clone()));
                self.drop_preview_cache_for(&pr.id);
                self.refresh_preview().await;
                let still_present = self.prs.iter().any(|p| p.id == pr.id);
                if still_present {
                    self.set_status(format!("✅ Reloaded {}.", pr.numslug()));
                } else {
                    self.set_status(format!("✅ Removed {} (no longer open).", pr.numslug()));
                }
            }
            Err(e) => self.set_status(format!("❌ Reload error for {}: {}", pr.numslug(), e)),
        }
    }

    async fn load_contributions(&mut self) -> surf::Result<()> {
        let login = crate::cmd::viewer::get().await?;
        let res = crate::cmd::contributions::fetch_calendar(&login).await?;
        let cal = &res.data.user.contributions_collection.contribution_calendar;
        let weeks = &cal.weeks;
        let mut lines: Vec<Line> = Vec::new();
        self.contrib_title = format!("Contributions: total {}", cal.total_contributions);
        let dates = crate::cmd::contributions::calendar_dates(&res);
        let window = ContribWindow::new(dates.today, dates.week_start);
        let mut year_to_date = ContribTotals::default();
        let mut month_to_date = ContribTotals::default();
        let mut week_to_date = ContribTotals::default();
        for day in 0..7 {
            let mut spans: Vec<Span> = Vec::new();
            for w in weeks {
                if let Some(d) = w.contribution_days.get(day) {
                    window.update_totals(
                        &d.date,
                        d.contribution_count,
                        &mut year_to_date,
                        &mut month_to_date,
                        &mut week_to_date,
                    );
                    spans.push(contrib_span(d.contribution_count, &d.color));
                } else {
                    spans.push(Span::raw("  "));
                }
            }
            lines.push(Line::from(spans));
        }
        let stats = vec![
            Line::from(format!(
                "# year to date:  {:4} {:>5.2}",
                year_to_date.total,
                year_to_date.avg()
            )),
            Line::from(format!(
                "# month to date: {:4} {:>5.2}",
                month_to_date.total,
                month_to_date.avg()
            )),
            Line::from(format!(
                "# week to date:  {:4} {:>5.2}",
                week_to_date.total,
                week_to_date.avg()
            )),
        ];
        self.contrib_lines = Some(lines);
        self.contrib_stats = Some(stats);
        Ok(())
    }
}

#[derive(Default)]
struct ContribTotals {
    total: usize,
    days: usize,
}

impl ContribTotals {
    fn add(&mut self, count: usize) {
        self.total += count;
        self.days += 1;
    }

    fn avg(&self) -> f64 {
        if self.days == 0 {
            0.0
        } else {
            self.total as f64 / self.days as f64
        }
    }
}

struct ContribWindow {
    today: String,
    today_year: String,
    today_month: String,
    week_start: String,
}

impl ContribWindow {
    fn new(today: &str, week_start: &str) -> Self {
        let today_year = today[..4].to_string();
        let today_month = today[..7].to_string();
        Self {
            today: today.to_string(),
            today_year,
            today_month,
            week_start: week_start.to_string(),
        }
    }

    fn update_totals(
        &self,
        date: &str,
        count: usize,
        year: &mut ContribTotals,
        month: &mut ContribTotals,
        week: &mut ContribTotals,
    ) {
        if date > self.today.as_str() {
            return;
        }
        if date.starts_with(&self.today_year) {
            year.add(count);
        }
        if date.starts_with(&self.today_month) {
            month.add(count);
        }
        if date >= self.week_start.as_str() {
            week.add(count);
        }
    }
}

fn contrib_span(count: usize, color: &str) -> Span<'static> {
    let (r, g, b) = styling::hex_to_rgb(color);
    let fg = styling::contrast_fg(r, g, b);
    let txt = if count >= 100 {
        String::from("++")
    } else {
        format!("{:>2}", count)
    };
    Span::styled(txt, Style::default().bg(Color::Rgb(r, g, b)).fg(fg))
}

fn make_commit_graph_text(entries: &[CommitGraphEntry]) -> Text<'static> {
    if entries.is_empty() {
        return Text::from("No commits found.");
    }

    let mut text = Text::default();
    for entry in entries {
        let mut spans: Vec<Span> = Vec::new();
        if !entry.graph.is_empty() {
            spans.push(Span::styled(
                entry.graph.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(
            entry.short_sha.clone(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(entry.summary.clone()));
        if let Some(author) = &entry.author {
            spans.push(Span::raw("  • "));
            spans.push(Span::styled(
                author.clone(),
                Style::default().fg(Color::Cyan),
            ));
        }
        if let Some(date) = &entry.date {
            spans.push(Span::raw("  ("));
            spans.push(Span::styled(date.clone(), Style::default().fg(Color::Gray)));
            spans.push(Span::raw(")"));
        }
        text.lines.push(Line::from(spans));
    }
    text
}

fn make_preview_block_title(app: &App, area_width: u16, total_lines: u16) -> String {
    if let (Some(pr), Some(mode)) = (app.get_selected_pr(), app.preview.mode) {
        // Reserve a bit for borders/padding
        let w = area_width.saturating_sub(4) as usize;
        // Base info
        let base = format!("#{} {} • {}", pr.number, pr.slug(), mode);
        // Try to include a shortened PR title if space allows
        let mut title = base.clone();
        if w > base.len() + 3 {
            let remain = w - base.len() - 3;
            let short = styling::ellipsize(&pr.title, remain);
            title = format!("{} • {}", base, short);
        }

        // Append simple scroll indicator if content overflows
        let visible = area_width.saturating_sub(2); // rough, columns vs lines differ, keep minimal
        let _ = visible; // keep calculation simple, omit lines/cols mismatch
        let _ = total_lines; // placeholder for future detailed indicators
        title
    } else {
        "Preview".to_string()
    }
}

fn layout_outer(area: Rect, contrib_height: u16) -> Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(0),
                Constraint::Length(contrib_height),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(area)
}

fn layout_main_chunks(area: Rect, preview_mode: Option<PreviewMode>) -> Rc<[Rect]> {
    if preview_mode.is_some() {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(area)
    } else {
        vec![area].into()
    }
}

fn build_pr_list(app: &App) -> List<'static> {
    let mut items: Vec<ListItem> = Vec::new();
    for pr in &app.prs {
        let line = format!("{} {}", pr, pr.ci_status());
        let styled = Span::styled(line, Style::default().fg(pr.merge_state_status.to_color()));
        items.push(ListItem::new(Line::from(styled)));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Pull Requests: total {}", app.prs.len()));
    let highlight_style = Style::default().add_modifier(Modifier::BOLD);
    List::new(items)
        .block(block)
        .highlight_style(highlight_style)
        .highlight_symbol(">> ")
}

fn build_preview_text(app: &App) -> Text<'static> {
    if let Some(pr) = app.get_selected_pr() {
        match app.preview.mode {
            Some(mode) => match app.cache.get(&(mode, pr.id.clone())) {
                Some(cached) => cached.clone(),
                None => Text::from(format!("Loading...{}", mode)),
            },
            None => Text::from("Preview closed"),
        }
    } else {
        Text::from("No selection")
    }
}

fn build_search_list(app: &App) -> List<'static> {
    let mut items: Vec<ListItem> = Vec::new();
    for item in &app.search.results {
        let repo = Span::styled(item.repo.clone(), Style::default().fg(Color::Cyan));
        let path = Span::styled(item.path.clone(), Style::default().fg(Color::Yellow));
        let line = Line::from(vec![repo, Span::raw(" "), path]);
        items.push(ListItem::new(line));
    }
    let title = if app.search.owner.is_empty() {
        "Search Results".to_string()
    } else {
        format!("Search Results: user {}", app.search.owner)
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let highlight_style = Style::default().add_modifier(Modifier::BOLD);
    List::new(items)
        .block(block)
        .highlight_style(highlight_style)
        .highlight_symbol(">> ")
}

fn build_search_preview(app: &App) -> Text<'static> {
    let Some(idx) = app.search.list_state.selected() else {
        return Text::from("No selection");
    };
    let Some(item) = app.search.results.get(idx) else {
        return Text::from("No selection");
    };
    let mut text = Text::default();
    text.lines.push(Line::from(vec![
        Span::styled(item.repo.clone(), Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(item.path.clone(), Style::default().fg(Color::Yellow)),
    ]));
    text.lines
        .push(Line::from(Span::raw(item.html_url.clone())));
    if item.matches.is_empty() {
        text.lines.push(Line::from("No preview available."));
        return text;
    }
    text.lines.push(Line::from(Span::raw("")));
    for fragment in &item.matches {
        for line in fragment.lines() {
            text.lines.push(Line::from(line.to_string()));
        }
        text.lines.push(Line::from(Span::raw("")));
    }
    text
}

fn build_search_input_help(owner: &str) -> Text<'static> {
    let mut text = Text::default();
    text.lines.push(Line::from("Search query tips"));
    text.lines.push(Line::from(""));
    text.lines.push(Line::from(vec![
        Span::styled("extension:yml", Style::default().fg(Color::Yellow)),
        Span::raw(" filter by extension"),
    ]));
    text.lines.push(Line::from(vec![
        Span::styled("path:.github/workflows", Style::default().fg(Color::Yellow)),
        Span::raw(" narrow by path"),
    ]));
    text.lines.push(Line::from(vec![
        Span::styled("filename:Dockerfile", Style::default().fg(Color::Yellow)),
        Span::raw(" match an exact file name"),
    ]));
    text.lines.push(Line::from(vec![
        Span::styled("language:rust", Style::default().fg(Color::Yellow)),
        Span::raw(" filter by language"),
    ]));
    text.lines.push(Line::from(vec![
        Span::styled("repo:owner/repo", Style::default().fg(Color::Yellow)),
        Span::raw(" limit to one repository"),
    ]));
    text.lines.push(Line::from(""));
    text.lines.push(Line::from("Example"));
    text.lines.push(Line::from(vec![Span::styled(
        "ci extension:yml path:.github/workflows",
        Style::default().fg(Color::Cyan),
    )]));
    text.lines.push(Line::from(""));
    if owner.is_empty() {
        text.lines.push(Line::from(
            "The query is sent as-is because no default user is set.",
        ));
    } else {
        text.lines.push(Line::from(vec![
            Span::raw("This mode also adds "),
            Span::styled(format!("user:{}", owner), Style::default().fg(Color::Green)),
            Span::raw(" automatically."),
        ]));
    }
    text
}

fn render_pr_list(f: &mut Frame, app: &mut App, area: Rect) {
    let list = build_pr_list(app);
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let preview_text = build_preview_text(app);
    let title = make_preview_block_title(app, area.width, preview_text.lines.len() as u16);
    let preview = Paragraph::new(preview_text)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false })
        .scroll((app.preview.scroll, 0));
    app.preview.height = area.height;
    f.render_widget(preview, area);
}

fn contrib_stats_width(stats: Option<&[Line<'static>]>) -> u16 {
    stats.map_or(0, |stats| {
        stats
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.len())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    }) as u16
}

fn split_contrib_areas(inner: Rect, stats: Option<&[Line<'static>]>) -> (Rect, Option<Rect>) {
    let stats_width = contrib_stats_width(stats);
    let min_chart_width = 10;
    let spacer_width = 1;
    let use_side_stats = stats_width > 0
        && inner.width >= stats_width.saturating_add(spacer_width + min_chart_width);
    if !use_side_stats {
        return (inner, None);
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Min(0),
                Constraint::Length(spacer_width),
                Constraint::Length(stats_width),
            ]
            .as_ref(),
        )
        .split(inner);
    (chunks[0], Some(chunks[2]))
}

fn trim_contrib_lines(lines: &[Line<'static>], visible_weeks: usize) -> Vec<Line<'static>> {
    let mut trimmed: Vec<Line> = Vec::with_capacity(lines.len());
    for line in lines.iter() {
        let spans = &line.spans;
        let len = spans.len();
        let start = len.saturating_sub(visible_weeks);
        let slice: Vec<Span> = spans[start..len].to_vec();
        trimmed.push(Line::from(slice));
    }
    trimmed
}

fn render_contributions(f: &mut Frame, app: &mut App, area: Rect) {
    let contrib_block = Block::default()
        .borders(Borders::ALL)
        .title(app.contrib_title.clone());
    let inner = contrib_block.inner(area);
    f.render_widget(contrib_block, area);
    if let Some(lines) = &app.contrib_lines {
        let stats = app.contrib_stats.as_deref();
        let (chart_area, stats_area) = split_contrib_areas(inner, stats);
        let inner_width = chart_area.width;
        let visible_weeks = (inner_width / 2) as usize;
        let mut trimmed = trim_contrib_lines(lines, visible_weeks);
        if stats_area.is_none()
            && let Some(stats) = &app.contrib_stats
        {
            trimmed.extend(stats.iter().cloned());
        }
        let contrib = Paragraph::new(trimmed).wrap(Wrap { trim: false });
        f.render_widget(contrib, chart_area);
        if let (Some(stats), Some(stats_area)) = (stats, stats_area) {
            let stats = Paragraph::new(stats.to_vec()).wrap(Wrap { trim: true });
            f.render_widget(stats, stats_area);
        }
    } else {
        let contrib = Paragraph::new("Loading contributions...").wrap(Wrap { trim: true });
        f.render_widget(contrib, inner);
    }
}

fn build_help_text(app: &App) -> String {
    if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        match app.mode {
            AppMode::Prs => {
                let base = "q:quit • s:search • ?:help • Enter/o:open • m:merge • a:approve • r:reload PR • R:reload all • c:reload contrib • ←/→:list/body/diff/graph";
                let nav = if app.preview.mode.is_some() {
                    "↑/↓/wheel:scroll"
                } else {
                    "↑/↓:navigate"
                };
                app.preview.mode.map_or_else(
                    || format!("{} • {}", base, nav),
                    |mode| format!("{} • {} • mode:{}", base, nav, mode),
                )
            }
            AppMode::Search => {
                let base = search_help_base(app.search.focus);
                let nav = if app.search.focus == SearchFocus::Results {
                    "↑/↓:navigate • Tab:focus input"
                } else {
                    "type to search"
                };
                format!("{} • {}", base, nav)
            }
        }
    }
}

fn search_help_base(focus: SearchFocus) -> &'static str {
    match focus {
        SearchFocus::Input => "Enter:search • Esc:back",
        SearchFocus::Results => "q:back • Enter:open • c:copy slug • →:preview • ←:close preview",
    }
}

fn render_help(f: &mut Frame, app: &App, area: Rect) {
    let help_text = build_help_text(app);
    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });
    f.render_widget(help, area);
}

fn render_search_input(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.search.owner.is_empty() {
        "Search".to_string()
    } else {
        format!("Search (user:{})", app.search.owner)
    };
    let cursor = if app.search.focus == SearchFocus::Input {
        "|"
    } else {
        ""
    };
    let input = format!("{}{}", app.search.query, cursor);
    let search = Paragraph::new(input)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: true });
    f.render_widget(search, area);
}

fn render_search_list(f: &mut Frame, app: &mut App, area: Rect) {
    let list = build_search_list(app);
    f.render_stateful_widget(list, area, &mut app.search.list_state);
}

fn render_search_input_help(f: &mut Frame, app: &App, area: Rect) {
    let help = Paragraph::new(build_search_input_help(&app.search.owner))
        .block(Block::default().borders(Borders::ALL).title("Search Tips"))
        .wrap(Wrap { trim: false });
    f.render_widget(help, area);
}

fn render_search_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let preview_text = build_search_preview(app);
    let preview = Paragraph::new(preview_text)
        .block(Block::default().borders(Borders::ALL).title("Preview"))
        .wrap(Wrap { trim: false });
    f.render_widget(preview, area);
}

fn ui(f: &mut Frame, app: &mut App) {
    let outer = layout_outer(f.area(), app.contrib_height);
    match app.mode {
        AppMode::Prs => {
            let main_chunks = layout_main_chunks(outer[0], app.preview.mode);
            render_pr_list(f, app, main_chunks[0]);
            if app.preview.mode.is_some() {
                let area = if main_chunks.len() > 1 {
                    main_chunks[1]
                } else {
                    outer[0]
                };
                render_preview(f, app, area);
            }
        }
        AppMode::Search => {
            let search_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
                .split(outer[0]);
            render_search_input(f, app, search_chunks[0]);
            if app.search.focus == SearchFocus::Input {
                render_search_input_help(f, app, search_chunks[1]);
            } else {
                let result_chunks = if app.search.preview_open {
                    Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(
                            [Constraint::Percentage(50), Constraint::Percentage(50)].as_ref(),
                        )
                        .split(search_chunks[1])
                } else {
                    vec![search_chunks[1]].into()
                };
                render_search_list(f, app, result_chunks[0]);
                if app.search.preview_open && result_chunks.len() > 1 {
                    render_search_preview(f, app, result_chunks[1]);
                }
            }
        }
    }
    render_contributions(f, app, outer[1]);
    render_help(f, app, outer[2]);
}

impl App {
    // Set a status message that clears automatically after a short delay.
    fn set_status<T: Into<String>>(&mut self, msg: T) {
        self.status_message = Some(msg.into());
        self.status_clear_at = Some(Instant::now() + Duration::from_millis(3000));
    }

    // Set a status message that stays until explicitly replaced or cleared.
    fn set_status_persistent<T: Into<String>>(&mut self, msg: T) {
        self.status_message = Some(msg.into());
        self.status_clear_at = None;
    }
}

fn build_commit_graph_entries(commits: &[Commit]) -> Vec<CommitGraphEntry> {
    let mut active: Vec<String> = Vec::new();
    let mut lines: Vec<CommitGraphEntry> = Vec::new();

    for commit in commits.iter().rev() {
        // Ensure the current commit is the first active branch.
        if let Some(pos) = active.iter().position(|sha| sha == &commit.sha) {
            let sha = active.remove(pos);
            active.insert(0, sha);
        } else {
            active.insert(0, commit.sha.clone());
        }

        lines.push(CommitGraphEntry {
            graph: build_graph_prefix(&active),
            short_sha: commit.sha.chars().take(7).collect::<String>(),
            summary: commit.summary(),
            author: commit.display_author(),
            date: commit.display_date(),
        });

        // Remove the commit itself and add parents to track branch lines.
        active.remove(0);
        for (idx, parent) in commit.parent_shas().enumerate() {
            if let Some(existing) = active.iter().position(|sha| sha == parent) {
                let sha = active.remove(existing);
                active.insert(idx, sha);
            } else {
                active.insert(idx, parent.to_string());
            }
        }
        dedup_branches(&mut active);
    }

    lines
}

fn build_graph_prefix(active: &[String]) -> String {
    let mut prefix = String::default();
    for (idx, _) in active.iter().enumerate() {
        if idx == 0 {
            prefix.push('*');
        } else {
            prefix.push('|');
        }
        prefix.push(' ');
    }
    prefix
}

fn dedup_branches(branches: &mut Vec<String>) {
    let mut seen: HashSet<String> = HashSet::new();
    branches.retain(|sha| seen.insert(sha.clone()));
}

async fn run_tui(specs: Vec<Slug>) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new(specs).await?;
    app.load_contributions().await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut app).await;

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

impl App {
    async fn handle_key(&mut self, code: KeyCode) {
        match self.mode {
            AppMode::Prs => self.handle_key_prs(code).await,
            AppMode::Search => self.handle_key_search(code).await,
        }
    }

    async fn handle_key_prs(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.on_quit(),
            KeyCode::Char('s') => self.enter_search_mode(),
            KeyCode::Down | KeyCode::Char('j') => self.on_down().await,
            KeyCode::Up | KeyCode::Char('k') => self.on_up().await,
            KeyCode::Enter | KeyCode::Char('o') => self.on_open(),
            KeyCode::Char('m') => self.on_merge_key(),
            KeyCode::Char('a') => self.on_approve_key(),
            KeyCode::Char('r') => self.on_reload_key(),
            KeyCode::Char('R') => self.on_reload_all_key(),
            KeyCode::Char('c') => self.on_reload_contrib_key(),
            KeyCode::Char('?') => self.on_clear_help(),
            KeyCode::Right => self.on_right(),
            KeyCode::Left => self.on_left(),
            _ => {}
        }
    }

    async fn handle_key_search(&mut self, code: KeyCode) {
        if is_search_back_key(self.search.focus, code) {
            self.exit_search_mode();
            return;
        }
        match self.search.focus {
            SearchFocus::Input => match code {
                KeyCode::Enter => self.on_search_submit(),
                KeyCode::Backspace => {
                    self.search.query.pop();
                }
                KeyCode::Char(ch) => {
                    self.search.query.push(ch);
                }
                _ => {}
            },
            SearchFocus::Results => match code {
                KeyCode::Enter => self.open_search_result(),
                KeyCode::Char('c') => self.copy_search_result_repo(),
                KeyCode::Tab | KeyCode::Backspace => {
                    self.search.focus = SearchFocus::Input;
                    self.search.preview_open = false;
                }
                KeyCode::Right => self.search.preview_open = true,
                KeyCode::Left => self.search.preview_open = false,
                KeyCode::Up | KeyCode::Char('k') => self.search.navigate(-1),
                KeyCode::Down | KeyCode::Char('j') => self.search.navigate(1),
                _ => {}
            },
        }
    }

    fn handle_mouse(&mut self, kind: MouseEventKind) {
        if self.mode != AppMode::Prs {
            return;
        }
        match kind {
            MouseEventKind::ScrollDown => self.scroll_preview_down(3),
            MouseEventKind::ScrollUp => self.scroll_preview_up(3),
            _ => {}
        }
    }
    fn on_quit(&mut self) {
        self.should_quit = true;
    }

    async fn on_down(&mut self) {
        if self.preview.mode.is_some() {
            self.scroll_preview_down(1);
        } else {
            self.navigate(1);
        }
    }

    async fn on_up(&mut self) {
        if self.preview.mode.is_some() {
            self.scroll_preview_up(1);
        } else {
            self.navigate(-1);
        }
    }

    fn on_open(&mut self) {
        self.open_url();
    }

    fn open_search_result(&mut self) {
        let Some(item) = self.selected_search_result() else {
            return;
        };
        if let Err(e) = open::that(&item.html_url) {
            eprintln!("Failed to open URL: {}", e);
        }
    }

    fn selected_search_result(&self) -> Option<&SearchItem> {
        let idx = self.search.list_state.selected()?;
        self.search.results.get(idx)
    }

    fn copy_search_result_repo(&mut self) {
        let Some(repo) = self.selected_search_result().map(|item| item.repo.clone()) else {
            self.set_status("No search result selected.");
            return;
        };
        match copy_to_clipboard(&repo) {
            Ok(()) => self.set_status(format!("Copied repo slug: {}", repo)),
            Err(e) => self.set_status(format!("❌ Failed to copy repo slug: {}", e)),
        }
    }

    fn on_merge_key(&mut self) {
        if let Some(pr) = self.get_selected_pr() {
            self.set_status_persistent(format!("Merging PR {}...", pr.numslug()));
            self.pending_task = Some(PendingTask::MergeSelected);
        }
    }

    fn on_approve_key(&mut self) {
        if let Some(pr) = self.get_selected_pr() {
            self.set_status_persistent(format!("Approving PR {}...", pr.numslug()));
            self.pending_task = Some(PendingTask::ApproveSelected);
        }
    }

    fn on_reload_key(&mut self) {
        self.set_status_persistent("🔄 Reloading PR...".to_string());
        self.pending_task = Some(PendingTask::ReloadSelected);
    }

    fn on_reload_all_key(&mut self) {
        self.set_status_persistent("🔄 Reloading all...".to_string());
        self.pending_task = Some(PendingTask::Reload);
    }

    fn on_reload_contrib_key(&mut self) {
        self.set_status_persistent("🔄 Reloading contrib...".to_string());
        self.pending_task = Some(PendingTask::ReloadContrib);
    }

    fn on_clear_help(&mut self) {
        self.status_message = None;
        self.status_clear_at = None;
    }

    fn queue_mode_if_needed(&mut self, mode: PreviewMode) {
        if let Some(pr) = self.get_selected_pr().cloned() {
            let has_cache = self.cache.contains_key(&(mode, pr.id.clone()));
            let pending = match mode {
                PreviewMode::Body => PendingTask::LoadBodyForSelected,
                PreviewMode::Diff => PendingTask::LoadDiffForSelected,
                PreviewMode::Commits => PendingTask::LoadCommitsForSelected,
            };
            if !has_cache {
                self.set_status_persistent(format!("🔎 Loading {} for #{}...", mode, pr.number));
                self.pending_task = Some(pending);
            }
        }
    }

    fn on_right(&mut self) {
        // Right: closed -> Body -> Diff -> Commits
        self.preview.scroll = 0;
        match self.preview.mode {
            None => {
                self.preview.mode = Some(PreviewMode::Body);
                self.queue_mode_if_needed(PreviewMode::Body);
            }
            Some(PreviewMode::Body) => {
                self.preview.mode = Some(PreviewMode::Diff);
                self.queue_mode_if_needed(PreviewMode::Diff);
            }
            Some(PreviewMode::Diff) => {
                self.preview.mode = Some(PreviewMode::Commits);
                self.queue_mode_if_needed(PreviewMode::Commits);
            }
            Some(PreviewMode::Commits) => {}
        }
    }

    fn on_left(&mut self) {
        // Left: Commits -> Diff -> Body -> Close
        self.preview.scroll = 0;
        match self.preview.mode {
            Some(PreviewMode::Commits) => {
                self.preview.mode = Some(PreviewMode::Diff);
                self.queue_mode_if_needed(PreviewMode::Diff);
            }
            Some(PreviewMode::Diff) => {
                self.preview.mode = Some(PreviewMode::Body);
                self.queue_mode_if_needed(PreviewMode::Body);
            }
            Some(PreviewMode::Body) => self.preview.mode = None,
            None => {}
        }
    }

    fn enter_search_mode(&mut self) {
        if self.search.owner.is_empty() {
            self.search.owner = Self::default_search_owner(&self.specs);
        }
        self.search.focus = SearchFocus::Input;
        self.search.preview_open = false;
        self.mode = AppMode::Search;
        self.set_status(format!(
            "Search user:{} (Enter to search)",
            self.search.owner
        ));
    }

    fn exit_search_mode(&mut self) {
        self.mode = AppMode::Prs;
        self.search.preview_open = false;
        self.search.focus = SearchFocus::Input;
        self.status_message = None;
        self.status_clear_at = None;
    }

    fn on_search_submit(&mut self) {
        let query = self.search.query.trim();
        if query.is_empty() {
            self.set_status("Enter search terms.".to_string());
            return;
        }
        let owner = self.search.owner.clone();
        let query = query.to_string();
        self.set_status_persistent(format!("🔎 Searching code for user:{}...", owner));
        self.pending_task = Some(PendingTask::SearchCode { owner, query });
    }

    async fn run_search(&mut self, owner: String, query: String) {
        match search_code(&owner, &query).await {
            Ok(items) => {
                self.search.results = items;
                self.search.select_first();
                self.search.focus = SearchFocus::Results;
                self.search.preview_open = false;
                self.set_status(format!(
                    "✅ Search done. {} results.",
                    self.search.results.len()
                ));
            }
            Err(e) => {
                self.set_status(format!("❌ Search error: {}", e));
            }
        }
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
                Event::Key(key) => app.handle_key(key.code).await,
                Event::Mouse(m) => app.handle_mouse(m.kind),
                _ => {}
            }
        }

        // If a long-running task is queued, redraw once to show the status
        // immediately, then execute the task.
        if let Some(task) = app.pending_task.take() {
            terminal.draw(|f| ui(f, app))?;
            match task {
                PendingTask::MergeSelected => app.merge_selected().await,
                PendingTask::ApproveSelected => app.approve_selected().await,
                PendingTask::Reload => app.reload().await,
                PendingTask::ReloadSelected => app.reload_selected_pr().await,
                PendingTask::ReloadContrib => {
                    if let Err(e) = app.load_contributions().await {
                        app.set_status(format!("❌ Contrib load error: {}", e));
                    } else {
                        app.set_status("✅ Contrib reloaded.".to_string());
                    }
                }
                PendingTask::LoadBodyForSelected => {
                    if let Some(pr) = app.get_selected_pr().cloned() {
                        let _ = app.load_body(&pr).await;
                    }
                }
                PendingTask::LoadDiffForSelected => {
                    if let Some(pr) = app.get_selected_pr().cloned() {
                        let _ = app.load_diff(&pr).await;
                    }
                }
                PendingTask::LoadCommitsForSelected => {
                    if let Some(pr) = app.get_selected_pr().cloned() {
                        let _ = app.load_commits(&pr).await;
                    }
                }
                PendingTask::SearchCode { owner, query } => {
                    app.run_search(owner, query).await;
                }
            }
        }

        // Auto-clear status messages when their timer expires.
        if let Some(clear_at) = app.status_clear_at
            && Instant::now() >= clear_at
        {
            app.status_message = None;
            app.status_clear_at = None;
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

    let mut specs: Vec<Slug> = Vec::new();
    for slug in slugs {
        specs.push(Slug::try_from(slug.as_str())?);
    }

    run_tui(specs).await.map_err(|e| {
        surf::Error::from_str(
            surf::StatusCode::InternalServerError,
            format!("TUI error: {}", e),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_leaves_search_input() {
        assert!(is_search_back_key(SearchFocus::Input, KeyCode::Esc));
    }

    #[test]
    fn q_leaves_search_results() {
        assert!(is_search_back_key(SearchFocus::Results, KeyCode::Char('q')));
    }

    #[test]
    fn q_stays_available_in_search_input() {
        assert!(!is_search_back_key(SearchFocus::Input, KeyCode::Char('q')));
    }

    #[test]
    fn search_input_help_mentions_common_filters() {
        let text = build_search_input_help("octocat");
        let joined = text
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("extension:yml"));
        assert!(joined.contains("path:.github/workflows"));
        assert!(joined.contains("filename:Dockerfile"));
        assert!(joined.contains("language:rust"));
        assert!(joined.contains("repo:owner/repo"));
        assert!(joined.contains("user:octocat"));
    }

    #[test]
    fn search_results_help_mentions_copy_slug() {
        assert!(search_help_base(SearchFocus::Results).contains("c:copy slug"));
    }

    #[test]
    fn contrib_window_includes_calendar_today() {
        let window = ContribWindow::new("2026-05-10", "2026-05-10");
        let mut year = ContribTotals::default();
        let mut month = ContribTotals::default();
        let mut week = ContribTotals::default();

        window.update_totals("2026-05-10", 3, &mut year, &mut month, &mut week);

        assert_eq!(year.total, 3);
        assert_eq!(month.total, 3);
        assert_eq!(week.total, 3);
    }

    #[async_std::test]
    async fn c_without_selection_sets_status_in_search_results() {
        let mut app = App {
            prs: Vec::new(),
            list_state: ListState::default(),
            should_quit: false,
            status_message: None,
            status_clear_at: None,
            specs: Vec::new(),
            cache: HashMap::new(),
            preview: Preview::default(),
            contrib_lines: None,
            contrib_stats: None,
            contrib_height: 9,
            contrib_title: "Contributions".to_string(),
            pending_task: None,
            mode: AppMode::Search,
            search: SearchState {
                owner: String::default(),
                query: String::default(),
                results: Vec::new(),
                list_state: ListState::default(),
                focus: SearchFocus::Results,
                preview_open: false,
            },
        };

        app.handle_key_search(KeyCode::Char('c')).await;

        assert_eq!(
            app.status_message.as_deref(),
            Some("No search result selected.")
        );
    }
}
