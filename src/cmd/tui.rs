use crate::cmd::prs::{self, Commit, CommitGraphEntry, MergeStateStatus, approve_pr, fetch_prs};
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
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

// Type alias for GraphQL PR node for brevity (reuse prs module types)
type PrNode = prs::pull_request::PullRequest;

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

#[derive(Debug, Clone, Copy)]
enum PendingTask {
    MergeSelected,
    ApproveSelected,
    Reload,
    LoadBodyForSelected,
    LoadDiffForSelected,
    LoadCommitsForSelected,
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
    contrib_height: u16,
    contrib_title: String,
    pending_task: Option<PendingTask>,
}

impl App {
    async fn new(specs: Vec<Slug>) -> App {
        let prs = fetch_prs(&specs).await.expect("Failed to fetch PRs");
        let mut list_state = ListState::default();
        if !prs.is_empty() {
            list_state.select(Some(0));
        }
        App {
            prs,
            list_state,
            should_quit: false,
            status_message: None,
            status_clear_at: None,
            specs,
            cache: HashMap::new(),
            preview: Preview::default(),
            contrib_lines: None,
            contrib_height: 9,
            contrib_title: "Contributions".to_string(),
            pending_task: None,
        }
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
                        self.set_status(format!("✅ Merged PR {}.", pr.numslug()));
                        self.prs.remove(selected_index);
                        if self.prs.is_empty() {
                            self.list_state.select(None);
                        } else if selected_index >= self.prs.len() {
                            self.list_state.select(Some(self.prs.len() - 1));
                        }
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
                    self.pending_task = Some(PendingTask::Reload);
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
        self.set_status_persistent("🔄 Reloading...".to_string());
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
            .unwrap_or(self.prs.len());

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
                    let (r, g, b) = styling::hex_to_rgb(&d.color);
                    let fg = styling::contrast_fg(r, g, b);
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
        let line = pr.to_string();
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

fn render_contributions(f: &mut Frame, app: &mut App, area: Rect) {
    let contrib_block = Block::default()
        .borders(Borders::ALL)
        .title(app.contrib_title.clone());
    if let Some(lines) = &app.contrib_lines {
        let inner_width = area.width.saturating_sub(2);
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
        f.render_widget(contrib, area);
    } else {
        let contrib = Paragraph::new("Loading contributions...")
            .block(contrib_block)
            .wrap(Wrap { trim: true });
        f.render_widget(contrib, area);
    }
}

fn build_help_text(app: &App) -> String {
    if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        let base = "q:quit • ?:help • Enter/o:open • m:merge • a:approve • r:reload • ←/→:list/body/diff/graph";
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
}

fn render_help(f: &mut Frame, app: &App, area: Rect) {
    let help_text = build_help_text(app);
    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });
    f.render_widget(help, area);
}

fn ui(f: &mut Frame, app: &mut App) {
    let outer = layout_outer(f.area(), app.contrib_height);
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
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(specs).await;
    app.load_contributions().await?;
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
        match code {
            KeyCode::Char('q') => self.on_quit(),
            KeyCode::Down | KeyCode::Char('j') => self.on_down().await,
            KeyCode::Up | KeyCode::Char('k') => self.on_up().await,
            KeyCode::Enter | KeyCode::Char('o') => self.on_open(),
            KeyCode::Char('m') => self.on_merge_key(),
            KeyCode::Char('a') => self.on_approve_key(),
            KeyCode::Char('r') => self.on_reload_key(),
            KeyCode::Char('?') => self.on_clear_help(),
            KeyCode::Right => self.on_right(),
            KeyCode::Left => self.on_left(),
            _ => {}
        }
    }

    fn handle_mouse(&mut self, kind: MouseEventKind) {
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
        self.set_status_persistent("🔄 Reloading...".to_string());
        self.pending_task = Some(PendingTask::Reload);
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
        specs.push(Slug::from(slug.as_str()));
    }

    run_tui(specs).await.map_err(|e| {
        surf::Error::from_str(
            surf::StatusCode::InternalServerError,
            format!("TUI error: {}", e),
        )
    })?;
    Ok(())
}
