use anyhow::{anyhow, bail};
use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Rect, Layout, Constraint, Direction},
    prelude::Margin,
    style::{Color, Style},
    symbols,
    widgets::{Block, Borders, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use crate::app::{App, keymap::Input, runner::HandleInputReturnType};

use backend::DataProvider;
use tui_textarea::{CursorMove, Scrolling, TextArea};

use super::Styles;
use super::commands::ClipboardOperation;

/// Modes for the Content editor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Normal,
    Insert,
    Visual,
}

pub struct Editor<'a> {
    /// TextArea for the entry input (single-line)
    entry_area: TextArea<'a>,
    /// TextArea for the content (multiple past entries)
    content_area: TextArea<'a>,
    /// Tracks whether the entry box is currently active
    entry_active: bool,
    mode: EditorMode,
    is_active: bool,
    is_dirty: bool,
    has_unsaved: bool,
}

impl From<&Input> for KeyEvent {
    fn from(value: &Input) -> Self {
        KeyEvent {
            code: value.key_code,
            modifiers: value.modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }
}

impl<'a> Editor<'a> {
    pub fn new() -> Editor<'a> {
        let entry_area = TextArea::default();
        let content_area = TextArea::default();

        Editor {
            entry_area,
            content_area,
            entry_active: false,
            mode: EditorMode::Normal,
            is_active: false,
            is_dirty: false,
            has_unsaved: false,
        }
    }

    #[inline]
    pub fn is_insert_mode(&self) -> bool {
        self.mode == EditorMode::Insert
    }

    #[inline]
    pub fn is_visual_mode(&self) -> bool {
        self.mode == EditorMode::Visual
    }

    #[inline]
    pub fn is_prioritized(&self) -> bool {
        matches!(self.mode, EditorMode::Insert | EditorMode::Visual)
    }

    /// Set the current entry content into the editor (content_area)
    pub fn set_current_entry<D: DataProvider>(&mut self, entry_id: Option<u32>, app: &App<D>) {
        let (content_lines, date) = match entry_id {
            Some(id) => {
                if let Some(entry) = app.get_entry(id) {
                    let lines: Vec<String> = entry.content.lines().map(|line| line.to_owned()).collect();
                    let dt = entry.date;
                    self.is_dirty = false;
                    (lines, Some(dt))
                } else {
                    (vec![], None)
                }
            }
            None => (vec![], None),
        };

        let mut content_area = TextArea::new(content_lines);
        content_area.move_cursor(CursorMove::Bottom);
        content_area.move_cursor(CursorMove::End);

        self.content_area = content_area;
        self.entry_area = TextArea::default(); // clear entry box on new entry/day
        self.entry_active = false;
        self.mode = EditorMode::Normal;
        self.is_dirty = false;
        self.refresh_has_unsaved(app);
    }

    /// Handle prioritized input (e.g., insert mode, clipboard) for content
    pub fn handle_input_prioritized<D: DataProvider>(
        &mut self,
        input: &Input,
        app: &App<D>,
    ) -> anyhow::Result<HandleInputReturnType> {
        // If entry box is active, route to entry_area
        if self.entry_active {
            // SHIFT+Enter in entry_area adds a timestamped entry
            if input.key_code == KeyCode::Enter && input.modifiers.contains(KeyModifiers::SHIFT) {
                // Fetch current time
                let now = chrono::Local::now();
                let timestamp_full = now.format("%Y_%m_%d_%H_%M_%S").to_string();
                let timestamp_short = now.format("%H:%M").to_string();

                // Get entry text
                let entry_text = self.entry_area.lines().first().cloned().unwrap_or_default();
                if !entry_text.trim().is_empty() {
                    // Clear entry box
                    self.entry_area = TextArea::default();

                    // Append to content_area with timestamp
                    let mut lines = self.content_area.lines().to_vec();
                    let new_line = format!("{} {}", timestamp_short, entry_text);
                    lines.push(new_line);
                    let mut new_content = TextArea::new(lines);
                    new_content.move_cursor(CursorMove::Bottom);
                    new_content.move_cursor(CursorMove::End);
                    self.content_area = new_content;

                    self.is_dirty = true;
                    self.has_unsaved = true;
                }
                // Indicate handled
                return Ok(HandleInputReturnType::Handled);
            }

            // Otherwise, feed input to entry_area
            let key_event = KeyEvent::from(input);
            if self.entry_area.input(key_event) {
                self.is_dirty = true;
                self.has_unsaved = true;
            }
            return Ok(HandleInputReturnType::Handled);
        }

        // If content area is active and in insert mode
        if self.is_insert_mode() {
            // Clipboard operations (Cut/Copy/Paste)
            if app.settings.sync_os_clipboard {
                let has_ctrl = input.modifiers.contains(KeyModifiers::CONTROL);
                let handled = match input.key_code {
                    KeyCode::Char('x') if has_ctrl => {
                        self.exec_os_clipboard(ClipboardOperation::Cut)?;
                        true
                    }
                    KeyCode::Char('c') if has_ctrl => {
                        self.exec_os_clipboard(ClipboardOperation::Copy)?;
                        true
                    }
                    KeyCode::Char('v') if has_ctrl => {
                        self.exec_os_clipboard(ClipboardOperation::Paste)?;
                        true
                    }
                    _ => false,
                };
                if handled {
                    return Ok(HandleInputReturnType::Handled);
                }
            }

            // Default insert behavior for content_area
            let key_event = KeyEvent::from(input);
            if self.content_area.input(key_event) {
                self.is_dirty = true;
                self.refresh_has_unsaved(app);
            }
            return Ok(HandleInputReturnType::Handled);
        }

        Ok(HandleInputReturnType::NotFound)
    }

    /// Handle general input (navigation, vim motions, etc.)
    pub fn handle_input<D: DataProvider>(
        &mut self,
        input: &Input,
        app: &App<D>,
    ) -> anyhow::Result<HandleInputReturnType> {
        // If no entry selected, consume input
        if app.get_current_entry().is_none() {
            return Ok(HandleInputReturnType::Handled);
        }

        // SHIFT+Tab to switch focus between entry and content
        if input.key_code == KeyCode::BackTab {
            // Toggle focus
            self.entry_active = !self.entry_active;
            // If leaving content area, ensure mode resets
            if !self.entry_active {
                self.mode = EditorMode::Normal;
            }
            return Ok(HandleInputReturnType::Handled);
        }

        // If entry box is active, we already handled above; continue to content if not
        if !self.entry_active {
            let sync_os_clipboard = app.settings.sync_os_clipboard;
            // Default navigation
            if is_default_navigation(input) {
                let key_event = KeyEvent::from(input);
                self.content_area.input(key_event);
            } else if !self.is_visual_mode()
                || !self.handle_input_visual_only(input, sync_os_clipboard)?
            {
                self.handle_vim_motions(input, sync_os_clipboard)?;
            }

            // Exiting visual mode if necessary
            if !self.content_area.is_selecting() && self.is_visual_mode() {
                self.set_editor_mode(EditorMode::Normal);
            }
            self.is_dirty = true;
            self.refresh_has_unsaved(app);
        }

        Ok(HandleInputReturnType::Handled)
    }

    /// Handles input specialized for visual mode only (copy/cut)
    fn handle_input_visual_only(
        &mut self,
        input: &Input,
        sync_os_clipboard: bool,
    ) -> anyhow::Result<bool> {
        if !input.modifiers.is_empty() {
            return Ok(false);
        }
        match input.key_code {
            KeyCode::Char('d') => {
                if sync_os_clipboard {
                    self.exec_os_clipboard(ClipboardOperation::Cut)?;
                } else {
                    self.content_area.cut();
                }
                Ok(true)
            }
            KeyCode::Char('y') => {
                if sync_os_clipboard {
                    self.exec_os_clipboard(ClipboardOperation::Copy)?;
                } else {
                    self.content_area.copy();
                }
                self.set_editor_mode(EditorMode::Normal);
                Ok(true)
            }
            KeyCode::Char('c') => {
                if sync_os_clipboard {
                    self.exec_os_clipboard(ClipboardOperation::Copy)?;
                } else {
                    self.content_area.cut();
                }
                self.set_editor_mode(EditorMode::Insert);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Handles Vim-like cursor motions
    fn handle_vim_motions(&mut self, input: &Input, sync_os_clipboard: bool) -> anyhow::Result<()> {
        let has_control = input.modifiers.contains(KeyModifiers::CONTROL);
        match (input.key_code, has_control) {
            (KeyCode::Char('h'), false) => {
                self.content_area.move_cursor(CursorMove::Back);
            }
            (KeyCode::Char('j'), false) => {
                self.content_area.move_cursor(CursorMove::Down);
            }
            (KeyCode::Char('k'), false) => {
                self.content_area.move_cursor(CursorMove::Up);
            }
            (KeyCode::Char('l'), false) => {
                self.content_area.move_cursor(CursorMove::Forward);
            }
            (KeyCode::Char('w'), false) | (KeyCode::Char('e'), false) => {
                self.content_area.move_cursor(CursorMove::WordForward);
            }
            (KeyCode::Char('b'), false) => {
                self.content_area.move_cursor(CursorMove::WordBack);
            }
            (KeyCode::Char('^'), false) => {
                self.content_area.move_cursor(CursorMove::Head);
            }
            (KeyCode::Char('$'), false) => {
                self.content_area.move_cursor(CursorMove::End);
            }
            (KeyCode::Char('D'), false) => {
                self.content_area.delete_line_by_end();
                self.exec_os_clipboard(ClipboardOperation::Copy)?;
            }
            (KeyCode::Char('C'), false) => {
                self.content_area.delete_line_by_end();
                self.exec_os_clipboard(ClipboardOperation::Copy)?;
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('p'), false) => {
                if sync_os_clipboard {
                    self.exec_os_clipboard(ClipboardOperation::Paste)?;
                } else {
                    self.content_area.paste();
                }
            }
            (KeyCode::Char('u'), false) => {
                self.content_area.undo();
            }
            (KeyCode::Char('r'), true) => {
                self.content_area.redo();
            }
            (KeyCode::Char('x'), false) => {
                self.content_area.delete_next_char();
                self.exec_os_clipboard(ClipboardOperation::Copy)?;
            }
            (KeyCode::Char('i'), false) => self.mode = EditorMode::Insert,
            (KeyCode::Char('a'), false) => {
                self.content_area.move_cursor(CursorMove::Forward);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('A'), false) => {
                self.content_area.move_cursor(CursorMove::End);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('o'), false) => {
                self.content_area.move_cursor(CursorMove::End);
                self.content_area.insert_newline();
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('O'), false) => {
                self.content_area.move_cursor(CursorMove::Head);
                self.content_area.insert_newline();
                self.content_area.move_cursor(CursorMove::Up);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('I'), false) => {
                self.content_area.move_cursor(CursorMove::Head);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('d'), true) => {
                self.content_area.scroll(Scrolling::HalfPageDown);
            }
            (KeyCode::Char('u'), true) => {
                self.content_area.scroll(Scrolling::HalfPageUp);
            }
            (KeyCode::Char('f'), true) => {
                self.content_area.scroll(Scrolling::PageDown);
            }
            (KeyCode::Char('b'), true) => {
                self.content_area.scroll(Scrolling::PageUp);
            }
            _ => {}
        }
        Ok(())
    }

    /// Get the current editor mode
    pub fn get_editor_mode(&self) -> EditorMode {
        self.mode
    }

    /// Set the editor mode (switch between normal, insert, visual)
    pub fn set_editor_mode(&mut self, mode: EditorMode) {
        match (self.mode, mode) {
            (EditorMode::Normal, EditorMode::Visual) => {
                self.content_area.start_selection();
            }
            (EditorMode::Visual, EditorMode::Normal | EditorMode::Insert) => {
                self.content_area.cancel_selection();
            }
            _ => {}
        }
        self.mode = mode;
    }

    /// Render the widget, splitting into Entry and Content areas
    pub fn render_widget(&mut self, frame: &mut Frame, area: Rect, styles: &Styles) {
        // Split the area into two equal parts: top = entry box, bottom = content
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(area);

        // Render Entry box (single-line input)
        let mut entry_title = "Entry".to_owned();
        if self.entry_active {
            entry_title.push_str(" - EDIT");
        }
        if self.has_unsaved && self.entry_active {
            entry_title.push_str(" *");
        }
        let entry_block_style = if self.entry_active {
            styles.editor.block_insert
        } else {
            styles.editor.block_normal_inactive
        };
        self.entry_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .style(entry_block_style)
                .title(entry_title),
        );
        // Entry box should not show cursor if not active
        let entry_cursor_style = if self.entry_active {
            Style::from(styles.editor.cursor_insert)
        } else {
            Style::reset()
        };
        self.entry_area.set_cursor_style(entry_cursor_style);
        self.entry_area.render(frame, chunks[0]);

        // Render Content area (past entries)
        let mut content_title = "Content".to_owned();
        if !self.entry_active && self.is_active {
            let mode_caption = match self.mode {
                EditorMode::Normal => " - NORMAL",
                EditorMode::Insert => " - EDIT",
                EditorMode::Visual => " - Visual",
            };
            content_title.push_str(mode_caption);
        }
        if self.has_unsaved && !self.entry_active {
            content_title.push_str(" *");
        }

        let content_block_style = match (self.mode, self.is_active && !self.entry_active) {
            (EditorMode::Insert, _) => styles.editor.block_insert,
            (EditorMode::Visual, _) => styles.editor.block_visual,
            (EditorMode::Normal, true) => styles.editor.block_normal_active,
            (EditorMode::Normal, false) => styles.editor.block_normal_inactive,
        };

        self.content_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .style(content_block_style)
                .title(content_title),
        );

        let content_cursor_style = if !self.entry_active && self.is_active {
            let s = match self.mode {
                EditorMode::Normal => styles.editor.cursor_normal,
                EditorMode::Insert => styles.editor.cursor_insert,
                EditorMode::Visual => styles.editor.cursor_visual,
            };
            Style::from(s)
        } else {
            Style::reset()
        };
        self.content_area.set_cursor_style(content_cursor_style);
        self.content_area.render(frame, chunks[1]);

        // Render scrollbars only for content
        self.render_vertical_scrollbar(frame, chunks[1]);
        self.render_horizontal_scrollbar(frame, chunks[1]);
    }

    fn render_vertical_scrollbar(&mut self, frame: &mut Frame, area: Rect) {
        let lines_count = self.content_area.lines().len();
        if lines_count as u16 <= area.height - 2 {
            return;
        }
        let (row, _) = self.content_area.cursor();
        let mut state = ScrollbarState::default()
            .content_length(lines_count)
            .position(row);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some(symbols::line::VERTICAL))
            .thumb_symbol(symbols::block::FULL);

        let scroll_area = area.inner(Margin {
            horizontal: 0,
            vertical: 1,
        });
        frame.render_stateful_widget(scrollbar, scroll_area, &mut state);
    }

    fn render_horizontal_scrollbar(&mut self, frame: &mut Frame, area: Rect) {
        let max_width = self.content_area
            .lines()
            .iter()
            .map(|line| line.len())
            .max()
            .unwrap_or_default();
        if max_width as u16 <= area.width - 2 {
            return;
        }
        let (_, col) = self.content_area.cursor();
        let mut state = ScrollbarState::default()
            .content_length(max_width)
            .position(col);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
            .begin_symbol(Some("◄"))
            .end_symbol(Some("►"))
            .track_symbol(Some(symbols::line::HORIZONTAL))
            .thumb_symbol("🬋");

        let scroll_area = area.inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        frame.render_stateful_widget(scrollbar, scroll_area, &mut state);
    }

    pub fn set_active(&mut self, active: bool) {
        // If deactivating content, reset visual mode
        if !active && self.is_visual_mode() {
            self.set_editor_mode(EditorMode::Normal);
        }
        self.is_active = active;
        // When overall editor is active, default focus to entry box
        if active {
            self.entry_active = true;
        } else {
            self.entry_active = false;
        }
    }

    /// Get the entire content as a single String
    pub fn get_content(&self) -> String {
        let lines = self.content_area.lines().to_vec();
        lines.join("\n")
    }

    pub fn has_unsaved(&self) -> bool {
        self.has_unsaved
    }

    pub fn refresh_has_unsaved<D: DataProvider>(&mut self, app: &App<D>) {
        self.has_unsaved = match self.is_dirty {
            true => {
                if let Some(entry) = app.get_current_entry() {
                    self.is_dirty && entry.content != self.get_content()
                } else {
                    false
                }
            }
            false => false,
        }
    }

    pub fn set_entry_content<D: DataProvider>(&mut self, entry_content: &str, app: &App<D>) {
        self.is_dirty = true;
        let lines = entry_content.lines().map(|line| line.to_owned()).collect();
        let mut text_area = TextArea::new(lines);
        text_area.move_cursor(CursorMove::Bottom);
        text_area.move_cursor(CursorMove::End);

        self.content_area = text_area;
        self.refresh_has_unsaved(app);
    }

    pub fn exec_os_clipboard(
        &mut self,
        operation: ClipboardOperation,
    ) -> anyhow::Result<HandleInputReturnType> {
        let mut clipboard = Clipboard::new().map_err(map_clipboard_error)?;
        match operation {
            ClipboardOperation::Copy => {
                self.content_area.copy();
                let selected_text = self.content_area.yank_text();
                clipboard
                    .set_text(selected_text)
                    .map_err(map_clipboard_error)?;
            }
            ClipboardOperation::Cut => {
                if self.content_area.cut() {
                    self.is_dirty = true;
                    self.has_unsaved = true;
                }
                let selected_text = self.content_area.yank_text();
                clipboard
                    .set_text(selected_text)
                    .map_err(map_clipboard_error)?;
            }
            ClipboardOperation::Paste => {
                let content = clipboard.get_text().map_err(map_clipboard_error)?;
                if content.is_empty() {
                    return Ok(HandleInputReturnType::Handled);
                }
                if !self.content_area.insert_str(content) {
                    bail!("Text can't be pasted into editor")
                }
                self.is_dirty = true;
                self.has_unsaved = true;
            }
        }
        Ok(HandleInputReturnType::Handled)
    }
}

fn is_default_navigation(input: &Input) -> bool {
    let has_control = input.modifiers.contains(KeyModifiers::CONTROL);
    let has_alt = input.modifiers.contains(KeyModifiers::ALT);
    match input.key_code {
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::PageUp
        | KeyCode::PageDown => true,
        KeyCode::Char('p') if has_control || has_alt => true,
        KeyCode::Char('n') if has_control || has_alt => true,
        KeyCode::Char('f') if !has_control && has_alt => true,
        KeyCode::Char('b') if !has_control && has_alt => true,
        KeyCode::Char('e') if has_control || has_alt => true,
        KeyCode::Char('a') if has_control || has_alt => true,
        KeyCode::Char('v') if has_control || has_alt => true,
        _ => false,
    }
}

fn map_clipboard_error(err: arboard::Error) -> anyhow::Error {
    anyhow!(
        "Error while communicating with the operating system clipboard.\nError Details: {}",
        err.to_string()
    )
}
