use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::theme;
use crate::ui::block;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let b = block("Log", true);
    let inner = b.inner(area);
    frame.render_widget(b, area);

    let lines: Vec<Line> = app
        .log
        .iter()
        .map(|l| Line::from(Span::styled(l.text.clone(), Style::new().fg(l.color))))
        .collect();

    let height = inner.height as usize;
    let total = lines.len();
    let from_bottom = app.log_scroll as usize;
    let skip = total.saturating_sub(height).saturating_sub(from_bottom.min(total));

    frame.render_widget(
        Paragraph::new(lines).scroll((skip as u16, 0)).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}
