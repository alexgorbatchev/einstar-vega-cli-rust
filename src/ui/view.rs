use super::Model;
use crate::output;
use einstar_vega::{NodeKind, Value};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Row, Table, Wrap},
};

pub fn render(model: &mut Model, frame: &mut Frame<'_>) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Percentage(25),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(areas[0]);
    let rows = model.visible_rows();
    let show_sizes = top[0].width >= 35;
    let items = rows
        .iter()
        .map(|row| {
            let mark = if model.selected.contains(&row.node.id) {
                "[x]"
            } else {
                "[ ]"
            };
            let branch = if row.node.kind == NodeKind::File {
                " "
            } else if model.expanded.contains(&row.node.id) {
                "-"
            } else {
                "+"
            };
            let mut cells = vec![format!(
                "{}{} {} {}",
                "  ".repeat(row.depth),
                mark,
                branch,
                output::safe(&row.node.name)
            )];
            if show_sizes {
                cells.push(output::size(row.node.size));
            }
            Row::new(cells).style(
                Style::default().fg(if model.selected.contains(&row.node.id) {
                    Color::Yellow
                } else if row.node.kind == NodeKind::Directory {
                    Color::Cyan
                } else {
                    Color::White
                }),
            )
        })
        .collect::<Vec<_>>();
    let count = items.len();
    let widths = if show_sizes {
        vec![Constraint::Min(1), Constraint::Length(10)]
    } else {
        vec![Constraint::Min(1)]
    };
    let table = Table::new(items, widths)
        .block(Block::default().borders(Borders::ALL).title(" Files "))
        .row_highlight_style(Style::default().bg(Color::DarkGray));
    model
        .table
        .select((count > 0).then_some(model.focus.min(count.saturating_sub(1))));
    frame.render_stateful_widget(table, top[0], &mut model.table);
    let mut info = Vec::new();
    for (key, value) in &model.info {
        match (key.as_str(), value) {
            ("batteryValue", Value::Float(value)) => {
                info.push(Line::from(format!("Battery: {:.0}%", value * 100.0)))
            }
            ("deviceParams", Value::Text(params)) => {
                for pair in params.split('_') {
                    if let Some((name, value)) = pair.split_once(':') {
                        info.push(Line::from(output::safe(&format!("{name}: {value}"))));
                    }
                }
            }
            ("boardInfo", Value::Map(map)) => {
                let mut fields = map
                    .iter()
                    .map(|(key, value)| format!("{}: {}", output::value(key), output::value(value)))
                    .collect::<Vec<_>>();
                fields.sort();
                info.extend(fields.iter().map(|line| Line::from(output::safe(line))));
            }
            _ => info.push(Line::from(output::safe(&format!(
                "{key}: {}",
                output::value(value)
            )))),
        }
    }
    frame.render_widget(
        Paragraph::new(info).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Device Info "),
        ),
        top[1],
    );
    let available = areas[1].height.saturating_sub(2) as usize;
    let mut logs = model
        .logs
        .iter()
        .rev()
        .take(available)
        .cloned()
        .collect::<Vec<_>>();
    logs.reverse();
    if let Some(progress) = &model.progress {
        if logs.len() == available && !logs.is_empty() {
            logs.remove(0);
        }
        logs.push(progress.clone());
    }
    frame.render_widget(
        Paragraph::new(logs.into_iter().map(Line::from).collect::<Vec<_>>())
            .block(Block::default().borders(Borders::ALL).title(" Logs ")),
        areas[1],
    );
    frame.render_widget(
        Paragraph::new("Space Select | Enter Expand | d Download | r Rename | x Delete | q Quit"),
        areas[2],
    );
}
