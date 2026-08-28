use nicer_proto::payload::TransportMode;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::theme;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::new().fg(theme::RAIL))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            " NICE/1 ",
            Style::new().fg(theme::NICE).bold(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let transport = app.status.as_ref().map(|s| s.transport);
    let (transport_label, transport_color) = match transport {
        Some(TransportMode::Secure) => ("SECURE", theme::NICE),
        Some(TransportMode::Plaintext) => ("PLAINTEXT", theme::CUT),
        None => ("...", theme::SLATE),
    };

    let device = if app.device.is_empty() { "connecting…" } else { &app.device };
    let short = app.fingerprint.map(|f| f.short()).unwrap_or_else(|| "…".into());
    let mut top = vec![
        Span::styled(format!(" {device} "), Style::new().fg(theme::BONE).bold()),
        Span::styled(short, Style::new().fg(theme::SLATE)),
        Span::raw("  "),
        Span::styled(format!(" {transport_label} "), Style::new().fg(theme::VOID).bg(transport_color)),
    ];
    if let Some(listen) = app.listen {
        top.push(Span::raw("  "));
        top.push(Span::styled(format!("{listen}"), Style::new().fg(theme::SIGNAL)));
    }
    if let Some(status) = &app.status {
        top.push(Span::raw("  "));
        top.push(Span::styled(
            format!(
                "discovery:{}",
                if status.discovery { "on" } else { "off" }
            ),
            Style::new().fg(theme::SLATE),
        ));
        top.push(Span::raw("  "));
        top.push(Span::styled(
            format!("peers:{}  conns:{}", status.paired_peers, status.connections),
            Style::new().fg(theme::SLATE),
        ));
    }

    let mut ticker_spans = vec![Span::styled("wire ", Style::new().fg(theme::DIM))];
    let width = inner.width as usize;
    let mut used = ticker_spans[0].content.len();
    for tick in &app.ticker {
        let tag = tick.opcode.to_string();
        let detail = if tick.detail.is_empty() {
            String::new()
        } else {
            format!("({}) ", truncate(&tick.detail, 24))
        };
        let piece_len = tag.len() + detail.len() + 2;
        if used + piece_len > width {
            break;
        }
        used += piece_len;
        ticker_spans.push(Span::styled(tag, Style::new().fg(tick.color).bold()));
        ticker_spans.push(Span::raw(" "));
        if !detail.is_empty() {
            ticker_spans.push(Span::styled(detail, Style::new().fg(theme::SLATE)));
        }
    }

    let lines = vec![Line::from(top), Line::from(ticker_spans)];
    frame.render_widget(Paragraph::new(lines).style(Style::new().bg(theme::PANEL)), inner);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}
