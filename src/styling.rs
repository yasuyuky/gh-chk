use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

pub fn hex_to_rgb(s: &str) -> (u8, u8, u8) {
    let hex = s.trim_start_matches('#');
    if hex.len() < 6 {
        return (0, 0, 0);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    (r, g, b)
}

pub fn contrast_fg(r: u8, g: u8, b: u8) -> Color {
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

pub fn ellipsize(s: &str, max: usize) -> String {
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

pub fn make_diff_text(diff: &str) -> Text<'static> {
    let mut text = Text::default();
    for line in diff.lines() {
        let style = if line.starts_with("===") {
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        } else if line.starts_with("@@") {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if line.starts_with('+') {
            Style::default().fg(Color::Green)
        } else if line.starts_with('-') {
            Style::default().fg(Color::Red)
        } else {
            Style::default()
        };
        let styled = Line::from(Span::styled(line.to_owned(), style));
        text.lines.push(styled);
    }
    text
}
