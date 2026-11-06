use crate::{cmd::contributions, styling};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

pub(super) struct ContributionView {
    pub(super) lines: Option<Vec<Line<'static>>>,
    pub(super) stats: Option<Vec<Line<'static>>>,
    pub(super) height: u16,
    pub(super) title: String,
}

impl Default for ContributionView {
    fn default() -> Self {
        Self {
            lines: None,
            stats: None,
            height: 9,
            title: "Contributions".to_string(),
        }
    }
}

impl ContributionView {
    pub(super) fn update(&mut self, res: &contributions::model::Res) {
        let cal = &res.data.user.contributions_collection.contribution_calendar;
        let weeks = &cal.weeks;
        let mut lines: Vec<Line> = Vec::new();
        self.title = format!("Contributions: total {}", cal.total_contributions);
        let dates = contributions::calendar_dates(res);
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
        self.lines = Some(lines);
        self.stats = Some(stats);
    }

    pub(super) fn show_load_error(&mut self) {
        if self.lines.is_none() {
            self.lines = Some(vec![Line::from(
                "Could not load contributions. Press c to retry.",
            )]);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
