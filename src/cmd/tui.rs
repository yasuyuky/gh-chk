use crate::cmd::prs::{MergeStateStatus, ReviewDecision};
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
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
    pub review_decision: Option<ReviewDecision>,
    pub reviewers: Vec<String>,
}

impl PrData {
    pub fn display_line(&self) -> String {
        let review_str = match &self.review_decision {
            Some(ReviewDecision::Approved) => " [approved]".to_string(),
            Some(ReviewDecision::ChangesRequested) => " [changes requested]".to_string(),
            Some(ReviewDecision::ReviewRequired) => " [review required]".to_string(),
            None => String::default(),
        };
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
            "#{} {} {} {}{}{} ({})",
            self.number,
            self.merge_state_status.to_emoji(),
            self.slug,
            self.title,
            review_str,
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
        review_decision: pr.review_decision.clone(),
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

#[derive(Debug, Clone, Copy)]
enum PendingTask {
    MergeSelected,
    ApproveSelected,
    Reload,
    LoadPreviewForSelected,
    LoadDiffForSelected,
}

struct App {
    prs: Vec<PrData>,
    list_state: ListState,
    should_quit: bool,
    status_message: Option<String>,
    status_clear_at: Option<Instant>,
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
    pending_task: Option<PendingTask>,
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
            status_clear_at: None,
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
            pending_task: None,
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
        if let Some(selected_index) = self.list_state.selected()
            && let Some(pr) = self.prs.get(selected_index).cloned()
        {
            if pr.merge_state_status == MergeStateStatus::Clean {
                self.set_status_persistent(format!("Merging PR #{} in {}...", pr.number, pr.slug));
                match crate::cmd::prs::merge_pr(&pr.id).await {
                    Ok(_) => {
                        self.set_status(format!("✅ Merged PR #{} in {}", pr.number, pr.slug));
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
                        self.set_status(format!(
                            "❌ Failed to merge PR #{} in {}: {}",
                            pr.number, pr.slug, e
                        ));
                    }
                }
            } else {
                self.set_status(format!(
                    "Cannot merge PR #{} in {}: not in clean state",
                    pr.number, pr.slug
                ));
            }
        }
    }
    async fn approve_selected(&mut self) {
        if let Some(selected_index) = self.list_state.selected()
            && let Some(pr) = self.prs.get(selected_index).cloned()
        {
            self.set_status_persistent(format!("Approving PR #{} in {}...", pr.number, pr.slug));
            match approve_pr(&pr.id).await {
                Ok(_) => {
                    self.set_status_persistent(format!(
                        "✅ Approved PR #{} in {}. Reloading...",
                        pr.number, pr.slug
                    ));
                    self.pending_task = Some(PendingTask::Reload);
                }
                Err(e) => {
                    self.set_status(format!(
                        "❌ Failed to approve PR #{} in {}: {}",
                        pr.number, pr.slug, e
                    ));
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
        self.set_status_persistent(format!("🔎 Loading preview for #{}...", pr.number));
        let (owner, name) = match split_slug(&pr.slug) {
            Some(x) => x,
            None => return Ok(()),
        };
        let body = fetch_pr_body(&owner, &name, pr.number).await?;
        self.preview_cache.insert(pr.id.clone(), body);
        self.set_status(format!("✅ Loaded preview for #{}", pr.number));
        Ok(())
    }

    async fn load_diff_for(&mut self, pr: &PrData) -> surf::Result<()> {
        self.set_status_persistent(format!("🔎 Loading diff for #{}...", pr.number));
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
        self.set_status(format!("✅ Loaded diff for #{}", pr.number));
        Ok(())
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
        self.set_status_persistent("🔄 Reloading...".to_string());
        let (new_list, any_err) = self.fetch_all_prs().await;
        if let Some(err) = any_err {
            self.set_status(format!("❌ Reload error: {}", err));
            return;
        }

        self.apply_pr_list_and_restore_selection(new_list);
        self.refresh_preview_if_open().await;
        self.set_status(format!("✅ Reloaded. {} PRs.", self.prs.len()));

        if let Err(e) = self.load_contributions().await {
            self.set_status(format!("❌ Contrib load error: {}", e));
        }
    }

    async fn fetch_all_prs(&self) -> (Vec<PrData>, Option<String>) {
        let mut new_list: Vec<PrData> = Vec::new();
        let mut any_err: Option<String> = None;
        for spec in self.specs.clone() {
            match spec {
                SlugSpec::Owner(owner) => match fetch_owner_prs(&owner).await {
                    Ok(mut prs) => new_list.append(&mut prs),
                    Err(e) => {
                        any_err = Some(format!("Failed to fetch {}: {}", owner, e));
                        break;
                    }
                },
                SlugSpec::Repo { owner, name } => match fetch_repo_prs(&owner, &name).await {
                    Ok(mut prs) => new_list.append(&mut prs),
                    Err(e) => {
                        any_err = Some(format!("Failed to fetch {}/{}: {}", owner, name, e));
                        break;
                    }
                },
            }
        }
        (new_list, any_err)
    }

    fn apply_pr_list_and_restore_selection(&mut self, new_list: Vec<PrData>) {
        let sel = self.list_state.selected().unwrap_or(0);
        self.prs = new_list;
        if self.prs.is_empty() {
            self.list_state.select(None);
        } else {
            let new_sel = sel.min(self.prs.len().saturating_sub(1));
            self.list_state.select(Some(new_sel));
        }
    }

    async fn refresh_preview_if_open(&mut self) {
        if !self.preview_open {
            return;
        }
        if let Some(pr) = self.get_selected_pr().cloned() {
            match self.preview_mode {
                PreviewMode::Body => {
                    let _ = self.load_preview_for(&pr).await;
                }
                PreviewMode::Diff => {
                    let _ = self.load_diff_for(&pr).await;
                }
            }
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

fn make_diff_text(diff: &str) -> Text<'static> {
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

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::default();
    for (i, ch) in s.chars().enumerate() {
        if i >= max.saturating_sub(1) {
            // leave room for '…'
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn make_preview_block_title(app: &App, area_width: u16, total_lines: u16) -> String {
    if let Some(pr) = app.get_selected_pr() {
        let mode = match app.preview_mode {
            PreviewMode::Body => "Body",
            PreviewMode::Diff => "Diff",
        };
        // Reserve a bit for borders/padding
        let w = area_width.saturating_sub(4) as usize;
        // Base info
        let base = format!("#{} {} • {}", pr.number, pr.slug, mode);
        // Try to include a shortened PR title if space allows
        let mut title = base.clone();
        if w > base.len() + 3 {
            let remain = w - base.len() - 3;
            let short = ellipsize(&pr.title, remain);
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

fn style_linkish(s: &str) -> Vec<Span<'static>> {
    let mut out: Vec<Span> = Vec::new();
    let mut rest = s;
    while let Some(idx) = rest.find("http://").or_else(|| rest.find("https://")) {
        let (pre, link_start) = rest.split_at(idx);
        if !pre.is_empty() {
            out.push(Span::raw(pre.to_string()));
        }
        let mut end = link_start.len();
        for (i, ch) in link_start.char_indices() {
            if ch.is_whitespace() {
                end = i;
                break;
            }
        }
        let (url_part, tail) = link_start.split_at(end);
        out.push(Span::styled(
            url_part.to_string(),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::UNDERLINED),
        ));
        rest = tail;
        if rest.is_empty() {
            break;
        }
    }
    if !rest.is_empty() {
        out.push(Span::raw(rest.to_string()));
    }
    out
}

fn style_inline_code_and_links(s: &str) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let mut in_code = false;
    let mut buf = String::default();
    for ch in s.chars() {
        if ch == '`' {
            if !buf.is_empty() {
                if in_code {
                    spans.push(Span::styled(
                        buf.clone(),
                        Style::default()
                            .bg(Color::Rgb(40, 40, 40))
                            .fg(Color::Yellow),
                    ));
                } else {
                    // process links in normal text
                    spans.extend(style_linkish(&buf));
                }
                buf.clear();
            }
            in_code = !in_code;
        } else {
            buf.push(ch);
        }
    }
    if !buf.is_empty() {
        if in_code {
            spans.push(Span::styled(
                buf,
                Style::default()
                    .bg(Color::Rgb(40, 40, 40))
                    .fg(Color::Yellow),
            ));
        } else {
            spans.extend(style_linkish(&buf));
        }
    }
    Line::from(spans)
}

fn prettify_pr_preview(title: &str, url: &str, body: &str) -> Text<'static> {
    let mut text = Text::default();

    // Title
    text.lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    // URL
    text.lines.push(Line::from(Span::styled(
        url.to_string(),
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
    )));
    text.lines.push(Line::from(""));

    // Body
    let mut in_fenced_code = false;
    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim_start();
        // Toggle fenced code blocks
        if trimmed.starts_with("```") {
            in_fenced_code = !in_fenced_code;
            continue;
        }
        if in_fenced_code {
            text.lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().bg(Color::Rgb(35, 35, 35)).fg(Color::White),
            )));
            continue;
        }

        // Headings
        if let Some(rest) = trimmed.strip_prefix("###### ") {
            text.lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        } else if let Some(rest) = trimmed.strip_prefix("##### ") {
            text.lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        } else if let Some(rest) = trimmed.strip_prefix("#### ") {
            text.lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        } else if let Some(rest) = trimmed.strip_prefix("### ") {
            text.lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            text.lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        } else if let Some(rest) = trimmed.strip_prefix("# ") {
            text.lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        // Unordered lists
        if let Some(rest) = trimmed.strip_prefix("- ") {
            text.lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(Color::Green)),
                Span::raw(rest.to_string()),
            ]));
            continue;
        } else if let Some(rest) = trimmed.strip_prefix("* ") {
            text.lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(Color::Green)),
                Span::raw(rest.to_string()),
            ]));
            continue;
        }

        // Normal line with inline code and links
        text.lines.push(style_inline_code_and_links(line));
    }

    text
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

fn layout_main_chunks(area: Rect, preview_open: bool) -> Rc<[Rect]> {
    if preview_open {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(area)
    } else {
        vec![area].into()
    }
}

fn build_pr_list(app: &App) -> List<'static> {
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

    List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Pull Requests"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ")
}

fn build_preview_text(app: &App) -> Text<'static> {
    if let Some(pr) = app.get_selected_pr() {
        match app.preview_mode {
            PreviewMode::Body => match app.preview_cache.get(&pr.id) {
                Some(body) => prettify_pr_preview(&pr.title, &pr.url, body),
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
        .scroll((app.preview_scroll, 0));
    app.preview_area_height = area.height;
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
    let main_chunks = layout_main_chunks(outer[0], app.preview_open);

    render_pr_list(f, app, main_chunks[0]);
    if app.preview_open {
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

#[derive(Deserialize, Default)]
struct CommitAuthor {
    login: Option<String>,
}

#[derive(Deserialize, Default)]
struct CommitPerson {
    name: Option<String>,
    date: Option<String>,
}

#[derive(Deserialize, Default)]
struct CommitDetail {
    message: String,
    #[serde(default)]
    author: Option<CommitPerson>,
}

#[derive(Deserialize, Default)]
struct CommitParent {
    sha: String,
}

#[derive(Deserialize, Default)]
struct PrCommitRes {
    sha: String,
    commit: CommitDetail,
    #[serde(default)]
    parents: Vec<CommitParent>,
    #[serde(default)]
    author: Option<CommitAuthor>,
}

#[derive(Clone)]
struct PrCommit {
    sha: String,
    summary: String,
    author: Option<String>,
    date: Option<String>,
    parents: Vec<String>,
}

impl From<PrCommitRes> for PrCommit {
    fn from(res: PrCommitRes) -> Self {
        let PrCommitRes {
            sha,
            commit,
            parents,
            author: user_author,
        } = res;
        let CommitDetail {
            message,
            author: commit_author,
        } = commit;
        let mut summary = message.lines().next().unwrap_or("").trim().to_string();
        if summary.len() > 80 {
            summary.truncate(77);
            summary.push_str("...");
        }
        let (commit_author_name, commit_author_date) = match commit_author {
            Some(person) => (person.name, person.date),
            None => (None, None),
        };
        let login = user_author.and_then(|a| a.login);
        let display_author = login.or(commit_author_name);
        let date = commit_author_date.and_then(|d| d.split('T').next().map(str::to_string));
        let parents = parents.into_iter().map(|p| p.sha).collect();
        PrCommit {
            sha,
            summary,
            author: display_author,
            date,
            parents,
        }
    }
}

#[derive(Clone)]
struct CommitGraphEntry {
    graph: String,
    short_sha: String,
    summary: String,
    author: Option<String>,
    date: Option<String>,
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

async fn fetch_pr_commits(owner: &str, name: &str, number: usize) -> surf::Result<Vec<PrCommit>> {
    let path = format!("repos/{}/{}/pulls/{}/commits", owner, name, number);
    let q: rest::QueryMap = rest::QueryMap::default();
    let res: Vec<PrCommitRes> = rest::get(&path, 1, &q).await?;
    Ok(res.into_iter().map(PrCommit::from).collect())
}

fn build_commit_graph_entries(commits: &[PrCommit]) -> Vec<CommitGraphEntry> {
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

        let graph_prefix = build_graph_prefix(&active);
        let short_sha = commit.sha.chars().take(7).collect::<String>();

        lines.push(CommitGraphEntry {
            graph: graph_prefix,
            short_sha,
            summary: commit.summary.clone(),
            author: commit.author.clone(),
            date: commit.date.clone(),
        });

        // Remove the commit itself and add parents to track branch lines.
        active.remove(0);
        for (idx, parent) in commit.parents.iter().enumerate() {
            if let Some(existing) = active.iter().position(|sha| sha == parent) {
                let sha = active.remove(existing);
                active.insert(idx, sha);
            } else {
                active.insert(idx, parent.clone());
            }
        }
        dedup_branches(&mut active);
    }

    lines
}

fn build_graph_prefix(active: &[String]) -> String {
    let mut prefix = String::new();
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
        KeyCode::Char('q') => on_quit(app),
        KeyCode::Down | KeyCode::Char('j') => on_down(app).await,
        KeyCode::Up | KeyCode::Char('k') => on_up(app).await,
        KeyCode::Enter | KeyCode::Char('o') => on_open(app),
        KeyCode::Char('m') => on_merge_key(app),
        KeyCode::Char('a') => on_approve_key(app),
        KeyCode::Char('r') => on_reload_key(app),
        KeyCode::Char('?') => on_clear_help(app),
        KeyCode::Right => on_right(app),
        KeyCode::Left => on_left(app),
        _ => {}
    }
}

fn on_quit(app: &mut App) {
    app.should_quit = true;
}

async fn on_down(app: &mut App) {
    if app.preview_open {
        app.scroll_preview_down(1);
    } else {
        app.next();
        app.maybe_prefetch_on_move().await;
    }
}

async fn on_up(app: &mut App) {
    if app.preview_open {
        app.scroll_preview_up(1);
    } else {
        app.previous();
        app.maybe_prefetch_on_move().await;
    }
}

fn on_open(app: &mut App) {
    app.open_url();
}

fn on_merge_key(app: &mut App) {
    if let Some(pr) = app.get_selected_pr() {
        app.set_status_persistent(format!("Merging PR #{} in {}...", pr.number, pr.slug));
        app.pending_task = Some(PendingTask::MergeSelected);
    }
}

fn on_approve_key(app: &mut App) {
    if let Some(pr) = app.get_selected_pr() {
        app.set_status_persistent(format!("Approving PR #{} in {}...", pr.number, pr.slug));
        app.pending_task = Some(PendingTask::ApproveSelected);
    }
}

fn on_reload_key(app: &mut App) {
    app.set_status_persistent("🔄 Reloading...".to_string());
    app.pending_task = Some(PendingTask::Reload);
}

fn on_clear_help(app: &mut App) {
    app.status_message = None;
    app.status_clear_at = None;
}

fn queue_preview_if_needed(app: &mut App) {
    if let Some(pr) = app.get_selected_pr().cloned()
        && !app.preview_cache.contains_key(&pr.id)
    {
        app.set_status_persistent(format!("🔎 Loading preview for #{}...", pr.number));
        app.pending_task = Some(PendingTask::LoadPreviewForSelected);
    }
}

fn queue_diff_if_needed(app: &mut App) {
    if let Some(pr) = app.get_selected_pr().cloned()
        && !app.diff_cache.contains_key(&pr.id)
    {
        app.set_status_persistent(format!("🔎 Loading diff for #{}...", pr.number));
        app.pending_task = Some(PendingTask::LoadDiffForSelected);
    }
}

fn on_right(app: &mut App) {
    // Right: closed -> Body, Body -> Diff
    if !app.preview_open {
        app.preview_open = true;
        app.preview_mode = PreviewMode::Body;
        queue_preview_if_needed(app);
    } else if app.preview_mode == PreviewMode::Body {
        app.preview_mode = PreviewMode::Diff;
        queue_diff_if_needed(app);
    }
}

fn on_left(app: &mut App) {
    // Left: Diff -> Body -> Close
    if !app.preview_open {
        return;
    }
    match app.preview_mode {
        PreviewMode::Diff => {
            app.preview_mode = PreviewMode::Body;
            queue_preview_if_needed(app);
        }
        PreviewMode::Body => app.preview_open = false,
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

        // If a long-running task is queued, redraw once to show the status
        // immediately, then execute the task.
        if app.pending_task.is_some() {
            terminal.draw(|f| ui(f, app))?;
            let task = app.pending_task.take();
            if let Some(task) = task {
                match task {
                    PendingTask::MergeSelected => async_std::task::block_on(app.merge_selected()),
                    PendingTask::ApproveSelected => {
                        async_std::task::block_on(app.approve_selected())
                    }
                    PendingTask::Reload => async_std::task::block_on(app.reload()),
                    PendingTask::LoadPreviewForSelected => async_std::task::block_on(async {
                        if let Some(pr) = app.get_selected_pr().cloned() {
                            let _ = app.load_preview_for(&pr).await;
                        }
                    }),
                    PendingTask::LoadDiffForSelected => async_std::task::block_on(async {
                        if let Some(pr) = app.get_selected_pr().cloned() {
                            let _ = app.load_diff_for(&pr).await;
                        }
                    }),
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
