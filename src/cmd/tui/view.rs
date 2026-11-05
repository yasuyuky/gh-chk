use super::{App, AppMode, PrNode, PreviewMode, SearchFocus, prs, styling};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::rc::Rc;

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
        items.push(ListItem::new(pr_list_line(pr)));
    }
    let mut title = format!("Pull Requests: total {}", app.prs.len());
    if let Some(auto_reload) = &app.auto_reload {
        title.push_str(&format!(" • auto {}s", auto_reload.interval.as_secs()));
    }
    let block = Block::default().borders(Borders::ALL).title(title);
    let highlight_style = Style::default().add_modifier(Modifier::BOLD);
    List::new(items)
        .block(block)
        .highlight_style(highlight_style)
        .highlight_symbol(">> ")
}

pub(super) fn pr_list_line(pr: &PrNode) -> Line<'static> {
    let style = Style::default().fg(pr.merge_state_status.to_color());
    let mut spans = Vec::new();
    push_pr_field(&mut spans, format!("#{}", pr.number), style);
    push_pr_field(&mut spans, pr.merge_state_status.to_emoji(), style);
    push_pr_field(&mut spans, pr.slug(), style);
    push_pr_field(&mut spans, pr.title.clone(), style);
    push_pr_field(
        &mut spans,
        pr.review_status(),
        review_status_style(pr, style),
    );
    let alert = pr.dependabot_alert_status();
    if !alert.is_empty() {
        push_pr_span(
            &mut spans,
            Span::styled(alert, Style::default().fg(Color::Cyan)),
            style,
        );
    }
    push_pr_field(
        &mut spans,
        pr.review_requests(),
        Style::default().fg(Color::Yellow),
    );
    push_pr_field(&mut spans, format!("({})", pr.created_date()), style);
    push_pr_field(&mut spans, pr.ci_status(), style);
    Line::from(spans)
}

fn review_status_style(pr: &PrNode, fallback: Style) -> Style {
    match pr.review_decision.as_ref() {
        Some(prs::ReviewDecision::Approved) => Style::default().fg(Color::Magenta),
        Some(prs::ReviewDecision::ChangesRequested) => Style::default().fg(Color::Red),
        Some(prs::ReviewDecision::ReviewRequired) => Style::default().fg(Color::Yellow),
        None => fallback,
    }
}

fn push_pr_field(spans: &mut Vec<Span<'static>>, text: impl Into<String>, style: Style) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    push_pr_span(spans, Span::styled(text, style), style);
}

fn push_pr_span(spans: &mut Vec<Span<'static>>, span: Span<'static>, style: Style) {
    if !spans.is_empty() {
        spans.push(Span::styled(" ", style));
    }
    spans.push(span);
}

pub(super) fn build_preview_text(app: &App) -> Text<'static> {
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

pub(super) fn build_search_input_help(owner: &str) -> Text<'static> {
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

pub(super) fn render_pr_list(f: &mut Frame, app: &mut App, area: Rect) {
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
                let base = prs_help_base();
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

pub(super) fn prs_help_base() -> &'static str {
    "q:quit • s:search • ?:help • Enter/o:open • p:profile • m:merge • a:approve • r:reload PR • R:reload all • c:reload contrib • ←/→:list/body/diff/graph"
}

pub(super) fn search_help_base(focus: SearchFocus) -> &'static str {
    match focus {
        SearchFocus::Input => "Enter:search • ↑/↓:history • Esc:back",
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

pub(super) fn ui(f: &mut Frame, app: &mut App) {
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
