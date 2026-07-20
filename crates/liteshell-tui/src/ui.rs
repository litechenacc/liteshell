use crate::{CompletionSource, TuiState};
use liteshell_core::AppMode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

const COMPLETION_ROWS: usize = 8;

pub fn draw(frame: &mut Frame, state: &TuiState, prompt: &str, last_status: i32) {
    if state.mode == AppMode::Pager {
        draw_pager(frame, state);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let prompt_row = draw_transcript(frame, state, prompt, last_status, chunks[0]);
    draw_statusline(frame, state, prompt, last_status, chunks[1]);

    if !state.completion.is_empty() {
        if let Some(prompt_row) = prompt_row {
            draw_completion_popup(frame, state, chunks[0], prompt_row);
        }
    }
}

/// Render history and the active prompt as one continuous shell transcript.
/// With little history the first prompt starts at the top; once the transcript
/// fills the viewport it naturally scrolls upward like a conventional shell.
fn draw_transcript(
    frame: &mut Frame,
    state: &TuiState,
    prompt: &str,
    last_status: i32,
    area: Rect,
) -> Option<u16> {
    let history: Vec<_> = state.output.lines().collect();
    let history_len = history.len();
    let show_prompt = state.mode != AppMode::RunningTask;
    let total = history_len + if show_prompt { 2 } else { 0 };
    let available = area.height as usize;
    let end = total.saturating_sub(state.output.offset.min(total.saturating_sub(1)));
    let start = end.saturating_sub(available);
    let directory = prompt.lines().next().unwrap_or(prompt);
    let divider = "─".repeat(area.width as usize);
    let mut prompt_row = None;
    let mut items = Vec::with_capacity(end.saturating_sub(start));

    for entry in history
        .iter()
        .take(end.min(history_len))
        .skip(start.min(history_len))
    {
        let item = if entry.divider {
            ListItem::new(divider.clone()).style(Style::default().fg(Color::DarkGray))
        } else {
            ListItem::new(entry.text.clone()).style(if entry.error {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            })
        };
        items.push(item);
    }

    if show_prompt && start <= history_len && history_len < end {
        items.push(ListItem::new(Line::from(Span::styled(
            directory.to_owned(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))));
    }
    if show_prompt && start <= history_len + 1 && history_len + 1 < end {
        prompt_row = Some(area.y + items.len() as u16);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                "❯ ",
                Style::default()
                    .fg(if last_status == 0 {
                        Color::Green
                    } else {
                        Color::Red
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(state.editor.text.clone()),
        ])));
    }

    frame.render_widget(List::new(items), area);
    if let Some(row) = prompt_row {
        let cursor_x = area.x + 2 + state.editor.text[..state.editor.cursor].width() as u16;
        frame.set_cursor_position((cursor_x.min(area.right().saturating_sub(1)), row));
    }
    prompt_row
}

fn draw_statusline(
    frame: &mut Frame,
    state: &TuiState,
    prompt: &str,
    last_status: i32,
    area: Rect,
) {
    let completion_active = state.mode == AppMode::Completion;
    let running = state.mode == AppMode::RunningTask;
    let mode = if running {
        " RUN "
    } else if completion_active {
        " COMPLETE "
    } else {
        " EDIT "
    };
    let mode_style = if running {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else if completion_active {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::Black).bg(Color::Green)
    }
    .add_modifier(Modifier::BOLD);
    let status_style = if last_status == 0 {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    };
    let directory = prompt.lines().next().unwrap_or(prompt);
    let message = if state.status.is_empty() {
        "Tab: complete  Ctrl-R: history  PgUp/PgDn: scroll"
    } else {
        &state.status
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(mode, mode_style),
            Span::styled(
                format!("  {directory}  "),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
            Span::styled(
                format!(" exit:{last_status} "),
                status_style.bg(Color::DarkGray),
            ),
            Span::styled(
                format!("  {message}"),
                Style::default().fg(Color::Gray).bg(Color::DarkGray),
            ),
        ]))
        .style(Style::default().bg(Color::DarkGray)),
        area,
    );
}

fn draw_completion_popup(
    frame: &mut Frame,
    state: &TuiState,
    transcript_area: Rect,
    prompt_row: u16,
) {
    let desired_rows = state.completion.len().min(COMPLETION_ROWS);
    let desired_height = desired_rows as u16 + 2;
    let below = transcript_area.bottom().saturating_sub(prompt_row + 1);
    let above = prompt_row.saturating_sub(transcript_area.y);
    let (y, height) = if below >= 3 {
        (prompt_row + 1, desired_height.min(below))
    } else if above >= 3 {
        let height = desired_height.min(above);
        (prompt_row.saturating_sub(height), height)
    } else {
        return;
    };
    let visible_rows = height.saturating_sub(2) as usize;
    if visible_rows == 0 {
        return;
    }
    let available_width = transcript_area.width;
    let width = if available_width >= 20 {
        available_width.saturating_sub(4).clamp(20, 72)
    } else {
        available_width
    };
    let x = transcript_area.x + (available_width.saturating_sub(width) / 2);
    let area = Rect {
        x,
        y,
        width,
        height,
    };
    let first = if state.selected >= visible_rows {
        state.selected - visible_rows + 1
    } else {
        0
    };
    let last = (first + visible_rows).min(state.completion.len());
    let position = format!(
        " {start}-{last}/{total} ",
        start = first + 1,
        total = state.completion.len()
    );
    let hidden_above = first;
    let hidden_below = state.completion.len().saturating_sub(last);
    let overflow = match (hidden_above, hidden_below) {
        (0, 0) => String::new(),
        (0, below) => format!(" ↓{below} more "),
        (above, 0) => format!(" ↑{above} more "),
        (above, below) => format!(" ↑{above}  ↓{below} more "),
    };
    let title = match state.completion_source {
        CompletionSource::Path => format!("{position} fuzzy search{overflow}"),
        CompletionSource::History => {
            format!("{position} history: {}{overflow}", state.completion_query)
        }
    };
    let rows = state
        .completion
        .iter()
        .enumerate()
        .skip(first)
        .take(visible_rows)
        .map(|(index, (label, detail))| {
            ListItem::new(Line::from(vec![
                Span::raw(label),
                Span::raw("  "),
                Span::styled(detail, Style::default().fg(Color::DarkGray)),
            ]))
            .style(if index == state.selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            })
        });

    frame.render_widget(Clear, area);
    frame.render_widget(
        List::new(rows).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(title),
        ),
        area,
    );
}

fn draw_pager(frame: &mut Frame, state: &TuiState) {
    let pager = state
        .pager
        .as_ref()
        .expect("pager mode requires pager state");
    let area = frame.area();
    let height = area.height.saturating_sub(1) as usize;
    let body = pager
        .lines
        .iter()
        .skip(pager.top)
        .take(height)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), area);
    let status = Rect {
        x: area.x,
        y: area.bottom().saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(format!(
            " {}  {}/{}  (q to quit) ",
            pager.title,
            (pager.top + height).min(pager.lines.len()),
            pager.lines.len()
        ))
        .style(Style::default().fg(Color::Black).bg(Color::White)),
        status,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn narrow_frame_renders() {
        let backend = TestBackend::new(24, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = TuiState::new(100, 4096);
        terminal
            .draw(|frame| draw(frame, &state, "C:\\work\n❯ ", 0))
            .unwrap();
        assert_eq!(terminal.backend().buffer().area.width, 24);
    }

    #[test]
    fn completion_is_an_overlay() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = TuiState::new(100, 4096);
        state.completion.push(("README.md".into(), "file".into()));
        terminal
            .draw(|frame| draw(frame, &state, "~\\work\n❯ ", 0))
            .unwrap();
        let rendered =
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .fold(String::new(), |mut text, cell| {
                    text.push_str(cell.symbol());
                    text
                });
        assert!(rendered.contains("fuzzy search"));
    }
}
