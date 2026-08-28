mod chat;
mod connections;
mod header;
mod log;
mod modal;
mod peers;
mod transfers;

use ratatui::layout::{Constraint, Direction as LayoutDirection, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, Tab};
use crate::theme;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    // `Block`'s own style-only fill recolors cells but leaves their glyphs alone, and
    // ratatui does not clear the buffer between frames — without this, a tab whose content
    // is shorter than the previous frame's leaves stale characters behind.
    frame.render_widget(Clear, area);
    frame.render_widget(ratatui::widgets::Block::new().style(Style::new().bg(theme::VOID)), area);

    let rows = Layout::new(
        LayoutDirection::Vertical,
        [
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ],
    )
    .split(area);

    header::draw(frame, app, rows[0]);
    draw_tabs(frame, app, rows[1]);
    draw_body(frame, app, rows[2]);
    draw_status_bar(frame, app, rows[3]);

    modal::draw(frame, app, area);
}

fn draw_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!(" {} {} ", i + 1, t.title())))
        .collect();
    let selected = Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::new().fg(theme::SLATE).bg(theme::VOID))
        .highlight_style(Style::new().fg(theme::VOID).bg(theme::NICE).bold())
        .divider(" ");
    frame.render_widget(tabs, area);
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    match app.tab {
        Tab::Peers => peers::draw(frame, app, area),
        Tab::Connections => connections::draw(frame, app, area),
        Tab::Transfers => transfers::draw(frame, app, area),
        Tab::Chat => chat::draw(frame, app, area),
        Tab::Log => log::draw(frame, app, area),
    }
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let hint = match app.tab {
        Tab::Peers => "h/l pane  j/k move  Enter connect  u unpair  f/c toggle auto-accept  n new  ? help  q quit",
        Tab::Connections => "j/k move  p pair  d disconnect  s resync  f send file  v/V clipboard  g chat  n new  ? help",
        Tab::Transfers => "j/k move  Enter respond to a pending offer  ? help",
        Tab::Chat => "h/l switch peer  j/k scroll  i chat  ? help",
        Tab::Log => "j/k scroll  PgUp/PgDn  ? help",
    };

    let line = match &app.status_msg {
        Some((text, color)) => Line::from(vec![
            Span::styled(format!(" {text} "), Style::new().fg(theme::VOID).bg(*color)),
            Span::raw("  "),
            Span::styled(hint, Style::new().fg(theme::DIM)),
        ]),
        None => Line::from(Span::styled(format!(" {hint}"), Style::new().fg(theme::DIM))),
    };

    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(theme::VOID)),
        area,
    );
}

pub fn block(title: &str, focused: bool) -> Block<'static> {
    let border_color = if focused { theme::NICE } else { theme::RAIL };
    Block::new()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::new().fg(border_color))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(theme::BONE).bold(),
        ))
        .style(Style::new().bg(theme::PANEL))
}
