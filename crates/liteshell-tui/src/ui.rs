use crate::{CompletionSource, ScrollEdge, TuiState};
use liteshell_core::{AppMode, SemanticColor, StyledLine, TextStyle};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const COMPLETION_ROWS: usize = 8;

fn semantic_style(style: TextStyle) -> Style {
    let foreground = match style.foreground {
        SemanticColor::Default => Color::Reset,
        SemanticColor::Directory => Color::LightBlue,
        SemanticColor::Executable | SemanticColor::Added => Color::Green,
        SemanticColor::Symlink | SemanticColor::Command => Color::Cyan,
        SemanticColor::Metadata | SemanticColor::Comment => Color::DarkGray,
        SemanticColor::Heading | SemanticColor::Keyword => Color::Magenta,
        SemanticColor::Option | SemanticColor::String => Color::Yellow,
        SemanticColor::Path => Color::White,
        SemanticColor::Number => Color::LightCyan,
        SemanticColor::Punctuation => Color::Gray,
        SemanticColor::Removed => Color::Red,
    };
    let mut result = Style::default().fg(foreground);
    if style.bold {
        result = result.add_modifier(Modifier::BOLD);
    }
    if style.dim {
        result = result.add_modifier(Modifier::DIM);
    }
    if style.italic {
        result = result.add_modifier(Modifier::ITALIC);
    }
    result
}

fn styled_line(line: &StyledLine) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|span| Span::styled(span.text.clone(), semantic_style(span.style)))
            .collect::<Vec<_>>(),
    )
}

pub fn draw(
    frame: &mut Frame,
    state: &TuiState,
    prompt: &str,
    last_status: i32,
    show_statusline: bool,
) {
    if state.mode == AppMode::Pager {
        let area = frame.area();
        draw_pager(frame, state);
        draw_scroll_flash(
            frame,
            state,
            Rect {
                height: area.height.saturating_sub(1),
                ..area
            },
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_statusline {
            vec![Constraint::Min(1), Constraint::Length(1)]
        } else {
            vec![Constraint::Min(1), Constraint::Length(0)]
        })
        .split(frame.area());

    let prompt_row = draw_transcript(frame, state, prompt, last_status, chunks[0]);
    if show_statusline {
        draw_statusline(frame, state, prompt, last_status, chunks[1]);
    }

    if !state.completion.is_empty() {
        if let Some(prompt_row) = prompt_row {
            draw_completion_popup(frame, state, chunks[0], prompt_row);
        }
    }
    draw_scroll_flash(frame, state, chunks[0]);
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
    let maximum_offset = total.saturating_sub(available);
    let offset = state.output.offset.min(maximum_offset);
    let start = total.saturating_sub(available).saturating_sub(offset);
    let end = (start + available).min(total);
    let directory = prompt.lines().next().unwrap_or(prompt);
    let divider = "─".repeat(area.width as usize);
    let mut prompt_row = None;
    let mut items = Vec::with_capacity(end.saturating_sub(start));
    let mut item_widths = Vec::with_capacity(end.saturating_sub(start));

    for entry in history
        .iter()
        .take(end.min(history_len))
        .skip(start.min(history_len))
    {
        let item = if entry.divider {
            item_widths.push(area.width as usize);
            ListItem::new(divider.clone()).style(Style::default().fg(Color::DarkGray))
        } else if entry.error {
            item_widths.push(entry.text.width());
            ListItem::new(entry.text.clone()).style(Style::default().fg(Color::Red))
        } else {
            item_widths.push(entry.text.width());
            ListItem::new(Line::from(
                entry
                    .spans
                    .iter()
                    .map(|span| Span::styled(span.text.clone(), semantic_style(span.style)))
                    .collect::<Vec<_>>(),
            ))
        };
        items.push(item);
    }

    if show_prompt && start <= history_len && history_len < end {
        item_widths.push(directory.width());
        items.push(ListItem::new(Line::from(Span::styled(
            directory.to_owned(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))));
    }
    if show_prompt && start <= history_len + 1 && history_len + 1 < end {
        prompt_row = Some(area.y + items.len() as u16);
        item_widths.push(2 + state.editor.text.width());
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
    if let Some(selection) = state.transcript_selection.as_ref() {
        frame.render_widget(
            SelectionOverlay {
                selection,
                first_row: start,
                row_widths: &item_widths,
            },
            area,
        );
    }
    if let Some(row) = prompt_row {
        let cursor_x = area.x + 2 + state.editor.text[..state.editor.cursor].width() as u16;
        frame.set_cursor_position((cursor_x.min(area.right().saturating_sub(1)), row));
    }
    prompt_row
}

struct SelectionOverlay<'a> {
    selection: &'a crate::TranscriptSelection,
    first_row: usize,
    row_widths: &'a [usize],
}

impl Widget for SelectionOverlay<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let style = Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(25, 105, 205))
            .add_modifier(Modifier::BOLD);
        for (screen_row, width) in self.row_widths.iter().copied().enumerate() {
            let absolute_row = self.first_row + screen_row;
            let Some((start, end)) = self
                .selection
                .range_for_row(absolute_row, width.min(area.width as usize))
            else {
                continue;
            };
            buffer.set_style(
                Rect {
                    x: area.x + start as u16,
                    y: area.y + screen_row as u16,
                    width: (end - start) as u16,
                    height: 1,
                },
                style,
            );
        }
    }
}

fn transcript_line_text(
    state: &TuiState,
    prompt: &str,
    width: usize,
    row: usize,
) -> Option<String> {
    let history_len = state.output.lines().count();
    if row < history_len {
        let entry = state.output.lines().nth(row)?;
        return Some(if entry.divider {
            "─".repeat(width)
        } else {
            entry.text.clone()
        });
    }
    if state.mode == AppMode::RunningTask {
        return None;
    }
    match row.saturating_sub(history_len) {
        0 => Some(prompt.lines().next().unwrap_or(prompt).to_owned()),
        1 => Some(format!("❯ {}", state.editor.text)),
        _ => None,
    }
}

fn slice_display_columns(text: &str, start: usize, end: usize) -> String {
    let mut column: usize = 0;
    text.graphemes(true)
        .filter(|grapheme| {
            let width = grapheme.width();
            let grapheme_start = column;
            let grapheme_end = column.saturating_add(width);
            column = grapheme_end;
            grapheme_end > start && grapheme_start < end
        })
        .collect()
}

pub fn selected_transcript_text(state: &TuiState, prompt: &str, width: usize) -> Option<String> {
    let selection = state.transcript_selection.as_ref()?;
    let (start, end) = selection.ordered();
    let mut lines = Vec::with_capacity(end.row.saturating_sub(start.row) + 1);
    for row in start.row..=end.row {
        let text = transcript_line_text(state, prompt, width, row)?;
        let selected = selection
            .range_for_row(row, text.width())
            .map(|(from, to)| slice_display_columns(&text, from, to))
            .unwrap_or_default();
        lines.push(selected);
    }
    let text = lines.join("\r\n");
    (!text.is_empty()).then_some(text)
}

pub fn selected_pager_text(state: &TuiState, width: usize) -> Option<String> {
    let selection = state.pager_selection.as_ref()?;
    let visual_lines = state.pager_visual_lines(width);
    let (start, end) = selection.ordered();
    let mut selected_lines = Vec::with_capacity(end.row.saturating_sub(start.row) + 1);
    for row in start.row..=end.row {
        let text = visual_lines.get(row)?.text();
        let selected = selection
            .range_for_row(row, text.width())
            .map(|(from, to)| slice_display_columns(&text, from, to))
            .unwrap_or_default();
        selected_lines.push(selected);
    }
    let text = selected_lines.join("\r\n");
    (!text.is_empty()).then_some(text)
}

fn draw_scroll_flash(frame: &mut Frame, state: &TuiState, area: Rect) {
    let Some(flash) = state.scroll_flash.as_ref() else {
        return;
    };
    if area.is_empty() {
        return;
    }
    let (label, y) = match flash.edge {
        ScrollEdge::Top => (" ▲ TOP ", area.y),
        ScrollEdge::Bottom => (" ▼ BOTTOM ", area.bottom().saturating_sub(1)),
    };
    let width = (label.width() as u16).min(area.width);
    let flash_on = (flash.started.elapsed().as_millis() / 90) % 2 == 0;
    let style = Style::default()
        .fg(Color::Black)
        .bg(if flash_on { Color::Cyan } else { Color::White })
        .add_modifier(Modifier::BOLD);
    let badge = Rect {
        x: area.right().saturating_sub(width),
        y,
        width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(label).style(style), badge);
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
        "Tab: complete  Drag: select  Right: copy/paste  Wheel: scroll"
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
        CompletionSource::DeepPath => format!("{position} recursive search{overflow}"),
        CompletionSource::Native => format!("{position} command completion{overflow}"),
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
    let visual_lines = state.pager_visual_lines(area.width as usize);
    let top = pager.top.min(visual_lines.len().saturating_sub(height));
    let body = visual_lines
        .iter()
        .skip(top)
        .take(height)
        .map(styled_line)
        .collect::<Vec<_>>();
    let row_widths: Vec<_> = body.iter().map(|line| line.width()).collect();
    frame.render_widget(Paragraph::new(body), area);
    if let Some(selection) = state.pager_selection.as_ref() {
        frame.render_widget(
            SelectionOverlay {
                selection,
                first_row: top,
                row_widths: &row_widths,
            },
            Rect {
                height: area.height.saturating_sub(1),
                ..area
            },
        );
    }
    let status = Rect {
        x: area.x,
        y: area.bottom().saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(format!(
            " {}  {}/{}  {} ",
            pager.title,
            (top + height).min(visual_lines.len()),
            visual_lines.len(),
            if state.status.is_empty() {
                "(q to quit)"
            } else {
                state.status.as_str()
            }
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
    fn semantic_palette_maps_directory_color() {
        let style = semantic_style(TextStyle::foreground(SemanticColor::Directory));
        assert_eq!(style.fg, Some(Color::LightBlue));
    }

    #[test]
    fn narrow_frame_renders() {
        let backend = TestBackend::new(24, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = TuiState::new(100, 4096);
        terminal
            .draw(|frame| draw(frame, &state, "C:\\work\n❯ ", 0, true))
            .unwrap();
        assert_eq!(terminal.backend().buffer().area.width, 24);
    }

    #[test]
    fn statusline_can_be_hidden_without_reserving_a_row() {
        let backend = TestBackend::new(40, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = TuiState::new(100, 4096);
        terminal
            .draw(|frame| draw(frame, &state, "C:\\work\n❯ ", 0, false))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!rendered.contains("Tab: complete"));
    }

    #[test]
    fn completion_is_an_overlay() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = TuiState::new(100, 4096);
        state.completion.push(("README.md".into(), "file".into()));
        terminal
            .draw(|frame| draw(frame, &state, "~\\work\n❯ ", 0, true))
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

    #[test]
    fn oldest_transcript_page_stays_full_and_starts_with_first_line() {
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = TuiState::new(100, 4096);
        for index in 0..9 {
            state.output.push_text(&format!("line {index}\n"), false);
        }
        state.output.scroll_up(usize::MAX, 5, 2);

        terminal
            .draw(|frame| draw(frame, &state, "C:\\work\n❯ ", 0, false))
            .unwrap();

        let rows: Vec<String> = terminal
            .backend()
            .buffer()
            .content
            .chunks(30)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect();
        assert!(rows[0].starts_with("line 0"));
        assert!(rows[4].starts_with("line 4"));
    }

    #[test]
    fn scroll_boundary_flash_is_drawn_at_the_relevant_edge() {
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = TuiState::new(100, 4096);
        state.flash_scroll_top();

        terminal
            .draw(|frame| draw(frame, &state, "C:\\work\n❯ ", 0, false))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("▲ TOP"));
    }

    #[test]
    fn transcript_selection_extracts_forward_reverse_and_unicode_ranges() {
        let mut state = TuiState::new(100, 4096);
        state.output.push_text("hello\nworld\na中b\n", false);

        assert!(state.begin_transcript_selection(1, 0, 8));
        state.update_transcript_selection(3, 0, 8);
        assert!(state.finish_transcript_selection());
        assert_eq!(
            selected_transcript_text(&state, "C:\\work\n❯ ", 40).as_deref(),
            Some("ell")
        );

        assert!(state.begin_transcript_selection(2, 1, 8));
        state.update_transcript_selection(1, 0, 8);
        assert!(state.finish_transcript_selection());
        assert_eq!(
            selected_transcript_text(&state, "C:\\work\n❯ ", 40).as_deref(),
            Some("ello\r\nwor")
        );

        assert!(state.begin_transcript_selection(1, 2, 8));
        state.update_transcript_selection(2, 2, 8);
        assert!(state.finish_transcript_selection());
        assert_eq!(
            selected_transcript_text(&state, "C:\\work\n❯ ", 40).as_deref(),
            Some("中")
        );
    }

    #[test]
    fn transcript_selection_is_highlighted_in_the_rendered_buffer() {
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = TuiState::new(100, 4096);
        state.output.push_text("hello\n", false);
        state.begin_transcript_selection(1, 0, 5);
        state.update_transcript_selection(3, 0, 5);
        state.finish_transcript_selection();

        terminal
            .draw(|frame| draw(frame, &state, "C:\\work\n❯ ", 0, false))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].bg, Color::Reset);
        assert_eq!(buffer[(1, 0)].bg, Color::Rgb(25, 105, 205));
        assert_eq!(buffer[(1, 0)].fg, Color::White);
        assert_eq!(buffer[(3, 0)].bg, Color::Rgb(25, 105, 205));
        assert_eq!(buffer[(4, 0)].bg, Color::Reset);
    }

    #[test]
    fn pager_selection_uses_visible_rows_and_extracts_text() {
        let mut state = TuiState::new(100, 4096);
        state.apply(vec![liteshell_core::OutputEvent::Pager {
            title: "test".to_owned(),
            lines: vec![StyledLine::plain("alpha"), StyledLine::plain("bravo")],
        }]);

        assert!(state.begin_pager_selection(1, 0, 4, 40));
        state.update_pager_selection(2, 1, 4, 40);
        assert!(state.finish_pager_selection());
        assert_eq!(
            selected_pager_text(&state, 40).as_deref(),
            Some("lpha\r\nbra")
        );
    }

    #[test]
    fn pager_selection_tracks_wrapped_visual_lines() {
        let mut state = TuiState::new(100, 4096);
        state.apply(vec![liteshell_core::OutputEvent::Pager {
            title: "test".to_owned(),
            lines: vec![StyledLine::plain("abcdefgh")],
        }]);

        assert_eq!(state.pager_visual_line_count(4), 2);
        assert!(state.begin_pager_selection(0, 1, 3, 4));
        state.update_pager_selection(2, 1, 3, 4);
        assert!(state.finish_pager_selection());
        assert_eq!(selected_pager_text(&state, 4).as_deref(), Some("efg"));
    }

    #[test]
    fn transcript_selection_coordinates_follow_the_scrolled_viewport() {
        let mut state = TuiState::new(100, 4096);
        for index in 0..10 {
            state.output.push_text(&format!("line {index}\n"), false);
        }

        state.output.scroll_up(usize::MAX, 5, 2);
        assert!(state.begin_transcript_selection(0, 0, 5));
        state.update_transcript_selection(3, 0, 5);
        assert!(state.finish_transcript_selection());
        assert_eq!(
            selected_transcript_text(&state, "C:\\work\n❯ ", 40).as_deref(),
            Some("line")
        );
    }

    #[test]
    fn a_plain_click_does_not_leave_a_one_cell_selection() {
        let mut state = TuiState::new(100, 4096);
        state.output.push_text("hello\n", false);
        state.begin_transcript_selection(2, 0, 5);

        assert!(!state.finish_transcript_selection());
        assert!(!state.has_transcript_selection());
    }
}
