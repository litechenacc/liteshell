mod scrollback;
mod terminal_session;
mod ui;

pub use scrollback::Scrollback;
pub use terminal_session::TerminalSession;
pub use ui::{draw, selected_pager_text, selected_transcript_text};

use liteshell_core::{AppMode, Editor, OutputEvent, OutputSink, StyledLine, StyledSpan};
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const SCROLL_FLASH_DURATION: Duration = Duration::from_millis(650);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScrollEdge {
    Top,
    Bottom,
}

pub(crate) struct ScrollFlash {
    pub edge: ScrollEdge,
    pub started: Instant,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SelectionPoint {
    pub row: usize,
    pub column: usize,
}

pub(crate) struct TranscriptSelection {
    pub anchor: SelectionPoint,
    pub head: SelectionPoint,
    pub dragging: bool,
}

impl TranscriptSelection {
    pub(crate) fn ordered(&self) -> (SelectionPoint, SelectionPoint) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub(crate) fn range_for_row(&self, row: usize, width: usize) -> Option<(usize, usize)> {
        let (start, end) = self.ordered();
        if row < start.row || row > end.row || width == 0 {
            return None;
        }
        let from = if row == start.row {
            start.column.min(width)
        } else {
            0
        };
        let to = if row == end.row {
            end.column.saturating_add(1).min(width)
        } else {
            width
        };
        (from < to).then_some((from, to))
    }
}

#[derive(Default)]
pub struct EventBuffer(pub Vec<OutputEvent>);

impl OutputSink for EventBuffer {
    fn emit(&mut self, event: OutputEvent) {
        self.0.push(event);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CompletionSource {
    #[default]
    Path,
    DeepPath,
    Native,
    History,
}

pub struct TuiState {
    pub mode: AppMode,
    pub editor: Editor,
    pub output: Scrollback,
    pub status: String,
    pub completion: Vec<(String, String)>,
    pub completion_source: CompletionSource,
    pub completion_query: String,
    pub history_hint: Option<String>,
    pub selected: usize,
    pub pager: Option<Pager>,
    pub(crate) scroll_flash: Option<ScrollFlash>,
    pub(crate) transcript_selection: Option<TranscriptSelection>,
    pub(crate) pager_selection: Option<TranscriptSelection>,
}

pub struct Pager {
    pub title: String,
    pub lines: Vec<StyledLine>,
    pub top: usize,
}

impl Pager {
    fn visual_lines(&self, width: usize) -> Vec<StyledLine> {
        if width == 0 {
            return Vec::new();
        }
        let mut visual = Vec::new();
        for line in &self.lines {
            let mut current = StyledLine::default();
            let mut current_width: usize = 0;
            for span in &line.spans {
                for grapheme in span.text.graphemes(true) {
                    let grapheme_width = grapheme.width();
                    if current_width > 0 && current_width.saturating_add(grapheme_width) > width {
                        visual.push(std::mem::take(&mut current));
                        current_width = 0;
                    }
                    if let Some(last) = current
                        .spans
                        .last_mut()
                        .filter(|last| last.style == span.style)
                    {
                        last.text.push_str(grapheme);
                    } else {
                        current.spans.push(StyledSpan::new(grapheme, span.style));
                    }
                    current_width = current_width.saturating_add(grapheme_width);
                }
            }
            visual.push(current);
        }
        visual
    }
}

impl TuiState {
    pub fn new(lines: usize, bytes: usize) -> Self {
        Self {
            mode: AppMode::Editing,
            editor: Editor::default(),
            output: Scrollback::new(lines, bytes),
            status: String::new(),
            completion: Vec::new(),
            completion_source: CompletionSource::Path,
            completion_query: String::new(),
            history_hint: None,
            selected: 0,
            pager: None,
            scroll_flash: None,
            transcript_selection: None,
            pager_selection: None,
        }
    }

    pub fn flash_scroll_top(&mut self) {
        self.scroll_flash = Some(ScrollFlash {
            edge: ScrollEdge::Top,
            started: Instant::now(),
        });
    }

    pub fn flash_scroll_bottom(&mut self) {
        self.scroll_flash = Some(ScrollFlash {
            edge: ScrollEdge::Bottom,
            started: Instant::now(),
        });
    }

    pub fn update_scroll_flash(&mut self) {
        if self
            .scroll_flash
            .as_ref()
            .is_some_and(|flash| flash.started.elapsed() >= SCROLL_FLASH_DURATION)
        {
            self.scroll_flash = None;
        }
    }

    pub fn scroll_flash_active(&self) -> bool {
        self.scroll_flash.is_some()
    }

    fn transcript_window(&self, visible_lines: usize) -> (usize, usize) {
        let trailing_lines = usize::from(self.mode != AppMode::RunningTask) * 2;
        let total = self.output.lines().count().saturating_add(trailing_lines);
        let maximum_offset = total.saturating_sub(visible_lines);
        let offset = self.output.offset.min(maximum_offset);
        let start = total.saturating_sub(visible_lines).saturating_sub(offset);
        (start, (start + visible_lines).min(total))
    }

    fn selection_point(
        &self,
        column: u16,
        screen_row: u16,
        visible_lines: usize,
    ) -> Option<SelectionPoint> {
        let (start, end) = self.transcript_window(visible_lines);
        let row = start.saturating_add(screen_row as usize);
        (row < end).then_some(SelectionPoint {
            row,
            column: column as usize,
        })
    }

    pub fn begin_transcript_selection(
        &mut self,
        column: u16,
        screen_row: u16,
        visible_lines: usize,
    ) -> bool {
        let Some(point) = self.selection_point(column, screen_row, visible_lines) else {
            self.transcript_selection = None;
            return false;
        };
        self.transcript_selection = Some(TranscriptSelection {
            anchor: point,
            head: point,
            dragging: true,
        });
        true
    }

    pub fn update_transcript_selection(
        &mut self,
        column: u16,
        screen_row: u16,
        visible_lines: usize,
    ) {
        let Some(point) = self.selection_point(column, screen_row, visible_lines) else {
            return;
        };
        if let Some(selection) = self.transcript_selection.as_mut() {
            if selection.dragging {
                selection.head = point;
            }
        }
    }

    pub fn finish_transcript_selection(&mut self) -> bool {
        let Some(selection) = self.transcript_selection.as_mut() else {
            return false;
        };
        selection.dragging = false;
        if selection.anchor == selection.head {
            self.transcript_selection = None;
            false
        } else {
            true
        }
    }

    pub fn clear_transcript_selection(&mut self) {
        self.transcript_selection = None;
    }

    pub fn has_transcript_selection(&self) -> bool {
        self.transcript_selection.is_some()
    }

    fn pager_selection_point(
        &self,
        column: u16,
        screen_row: u16,
        visible_lines: usize,
        width: usize,
    ) -> Option<SelectionPoint> {
        let pager = self.pager.as_ref()?;
        let line_count = pager.visual_lines(width).len();
        let first = pager.top.min(line_count.saturating_sub(visible_lines));
        let row = first.saturating_add(screen_row as usize);
        (row < line_count).then_some(SelectionPoint {
            row,
            column: column as usize,
        })
    }

    pub fn begin_pager_selection(
        &mut self,
        column: u16,
        screen_row: u16,
        visible_lines: usize,
        width: usize,
    ) -> bool {
        let Some(point) = self.pager_selection_point(column, screen_row, visible_lines, width)
        else {
            self.pager_selection = None;
            return false;
        };
        self.pager_selection = Some(TranscriptSelection {
            anchor: point,
            head: point,
            dragging: true,
        });
        true
    }

    pub fn update_pager_selection(
        &mut self,
        column: u16,
        screen_row: u16,
        visible_lines: usize,
        width: usize,
    ) {
        let Some(point) = self.pager_selection_point(column, screen_row, visible_lines, width)
        else {
            return;
        };
        if let Some(selection) = self.pager_selection.as_mut() {
            if selection.dragging {
                selection.head = point;
            }
        }
    }

    pub fn finish_pager_selection(&mut self) -> bool {
        let Some(selection) = self.pager_selection.as_mut() else {
            return false;
        };
        selection.dragging = false;
        if selection.anchor == selection.head {
            self.pager_selection = None;
            false
        } else {
            true
        }
    }

    pub fn clear_pager_selection(&mut self) {
        self.pager_selection = None;
    }

    pub fn has_pager_selection(&self) -> bool {
        self.pager_selection.is_some()
    }

    pub fn pager_visual_line_count(&self, width: usize) -> usize {
        self.pager
            .as_ref()
            .map(|pager| pager.visual_lines(width).len())
            .unwrap_or_default()
    }

    pub(crate) fn pager_visual_lines(&self, width: usize) -> Vec<StyledLine> {
        self.pager
            .as_ref()
            .map(|pager| pager.visual_lines(width))
            .unwrap_or_default()
    }

    pub fn apply(&mut self, events: Vec<OutputEvent>) {
        for event in events {
            match event {
                OutputEvent::Text(text) => self.output.push_text(&text, false),
                OutputEvent::Styled(text) => self.output.push_styled(&text, false),
                OutputEvent::Error(text) => self.output.push_text(&text, true),
                OutputEvent::Clear => self.output.clear(),
                OutputEvent::Status(status) => self.status = status,
                OutputEvent::Pager { title, lines } => {
                    self.pager_selection = None;
                    self.status.clear();
                    self.pager = Some(Pager {
                        title,
                        lines,
                        top: 0,
                    });
                    self.mode = AppMode::Pager;
                }
            }
        }
    }
}
