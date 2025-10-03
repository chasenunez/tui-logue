use chrono::Local;
use anyhow::{anyhow, bail};
use arboard::Clipboard;
use std::path::PathBuf;
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

/// Result of processing an input in the Entry box.
pub enum EntryBoxInputResult {
    /// Entry box consumed the input (no submission).
    Handled,
    /// Entry box submitted a line (Enter pressed) — returned string is the submitted text.
    Submitted(String),
}

/// A small single-line input that sits above the Content editor.
pub struct EntryBox<'a> {
    input_area: TextArea<'a>,
    is_active: bool,
}

impl<'a> EntryBox<'a> {
    pub fn new() -> Self {
        // initialize with an empty single line
        let ta = TextArea::new(vec!["".to_string()]);
        Self {
            input_area: ta,
            is_active: true,
        }
    }

    /// Handle a single input event. Returns:
    /// - `Submitted(text)` when Enter was pressed (and text was non-empty),
    /// - `Handled` if the entry box consumed the input (but didn't submit),
    /// - (we don't return NotHandled here — caller can decide to route input elsewhere if desired)
    pub fn handle_input(&mut self, input: &Input) -> EntryBoxInputResult {
        let key_event = KeyEvent::from(input);

        match key_event.code {
            KeyCode::Enter => {
                // Grab the first line (this box is single-line) and trim it.
                let line = self
                    .input_area
                    .lines()
                    .first()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();

                // clear the input area
                self.input_area = TextArea::new(vec!["".to_string()]);

                if !line.is_empty() {
                    EntryBoxInputResult::Submitted(line)
                } else {
                    EntryBoxInputResult::Handled
                }
            }
            _ => {
                // Let the textarea consume anything else (typing, cursor movement inside the box).
                // This makes the box behave like a normal input field when active.
                let _ = self.input_area.input(key_event);
                EntryBoxInputResult::Handled
            }
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, is_active: bool) {
        // If you want to show active/inactive styles later, you can branch on is_active.
        self.input_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Entry"),
        );

        // Optionally set cursor style only when active
        if is_active {
            self.input_area.set_cursor_line_style(Style::reset());
        } else {
            self.input_area.set_cursor_line_style(Style::default());
        }

        frame.render_widget(&self.input_area, area);
    }

    pub fn set_active(&mut self, active: bool) {
        self.is_active = active;
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Normal,
    Insert,
    Visual,
}

pub struct Editor<'a> {
    text_area: TextArea<'a>,

    /// The small entry box rendered above the content area.
    entry_box: EntryBox<'a>,

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
        let text_area = TextArea::default();
        let entry_box = EntryBox::new();

        Editor {
            text_area,
            entry_box,
            mode: EditorMode::Normal,
            is_active: false,
            is_dirty: false,
            has_unsaved: false,
        }
    }

    /// Append a line to the content with a local timestamp "[HH:MM] Entry"
    pub fn append_entry<D: DataProvider>(&mut self, entry: &str, app: &App<D>) {
        let now = Local::now();
        let timestamp = now.format("[%H:%M]").to_string();
        let line = format!("{} {}", timestamp, entry);

        // Insert the line and a newline after it
        if !self.text_area.insert_str(&line) {
            // If insertion failed for some reason, try re-creating the TextArea.
            let mut lines: Vec<String> = self.text_area.lines().iter().cloned().collect();
            lines.push(line);
            self.text_area = TextArea::new(lines);
        } else {
            self.text_area.insert_newline();
        }

        self.is_dirty = true;
        self.refresh_has_unsaved(app);
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

    pub fn set_current_entry<D: DataProvider>(&mut self, entry_id: Option<u32>, app: &App<D>) {
        let text_area = match entry_id {
            Some(id) => {
                if let Some(entry) = app.get_entry(id) {
                    self.is_dirty = false;
                    let lines = entry.content.lines().map(|line| line.to_owned()).collect();
                    let mut text_area = TextArea::new(lines);
                    text_area.move_cursor(tui_textarea::CursorMove::Bottom);
                    text_area.move_cursor(tui_textarea::CursorMove::End);
                    text_area
                } else {
                    TextArea::default()
                }
            }
            None => TextArea::default(),
        };

        self.text_area = text_area;

        self.refresh_has_unsaved(app);

        let repo_path = PathBuf::from("/Users/nunezcha/Documents/log_cold_storage");
        let message = format!("Auto commit: new entry at {}", Local::now().to_rfc3339());
        {
    let repo = repo_path.clone();
    let msg = message.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::app::git::commit_and_push(&repo, &msg) {
            eprintln!("auto git commit failed: {:?}", e);
                }
            });
        }
    }

    pub fn handle_input_prioritized<D: DataProvider>(
        &mut self,
        input: &Input,
        app: &App<D>,
    ) -> anyhow::Result<HandleInputReturnType> {
        // If the entry box is active, route input to it first.
        if self.entry_box.is_active() {
            match self.entry_box.handle_input(input) {
                EntryBoxInputResult::Submitted(line) => {
                    self.append_entry(&line, app);
                    return Ok(HandleInputReturnType::Handled);
                }
                EntryBoxInputResult::Handled => {
                    return Ok(HandleInputReturnType::Handled);
                }
            }
        }

        if self.is_insert_mode() {
            // We must handle clipboard operation separately if sync with system clipboard is activated
            if app.settings.sync_os_clipboard {
                let has_ctrl = input.modifiers.contains(KeyModifiers::CONTROL);
                // Keymaps are taken from `text_area` source code
                let handled = match input.key_code {
                    KeyCode::Char('x') if has_ctrl => {
                        self.exec_os_clipboard(ClipboardOperation::Cut)?;
                        true
                    }
                    KeyCode::Char('c') if has_ctrl => {
                        self.exec_os_clipboard(ClipboardOperation::Copy)?;
                        true
                    }
                    KeyCode::Char('y') if has_ctrl => {
                        self.exec_os_clipboard(ClipboardOperation::Paste)?;
                        true
                    }
                    _ => false,
                };

                if handled {
                    return Ok(HandleInputReturnType::Handled);
                }
            }

            // give the input to the editor
            let key_event = KeyEvent::from(input);
            if self.text_area.input(key_event) {
                self.is_dirty = true;
                self.refresh_has_unsaved(app);
            }

            return Ok(HandleInputReturnType::Handled);
        }

        Ok(HandleInputReturnType::NotFound)
    }

    pub fn handle_input<D: DataProvider>(
        &mut self,
        input: &Input,
        app: &App<D>,
    ) -> anyhow::Result<HandleInputReturnType> {
        debug_assert!(!self.is_insert_mode());

        // If the entry box is active, route input to it (it will consume everything while active).
        if self.entry_box.is_active() {
            match self.entry_box.handle_input(input) {
                EntryBoxInputResult::Submitted(line) => {
                    self.append_entry(&line, app);
                    return Ok(HandleInputReturnType::Handled);
                }
                EntryBoxInputResult::Handled => {
                    return Ok(HandleInputReturnType::Handled);
                }
            }
        }

        if app.get_current_entry().is_none() {
            return Ok(HandleInputReturnType::Handled);
        }

        let sync_os_clipboard = app.settings.sync_os_clipboard;

        if is_default_navigation(input) {
            let key_event = KeyEvent::from(input);
            self.text_area.input(key_event);
        } else if !self.is_visual_mode()
            || !self.handle_input_visual_only(input, sync_os_clipboard)?
        {
            self.handle_vim_motions(input, sync_os_clipboard)?;
        }

        // Check if the input led the editor to leave the visual mode and make the corresponding UI changes
        if !self.text_area.is_selecting() && self.is_visual_mode() {
            self.set_editor_mode(EditorMode::Normal);
        }

        self.is_dirty = true;
        self.refresh_has_unsaved(app);

        Ok(HandleInputReturnType::Handled)
    }

    /// Handles input specialized for visual mode only like cut and copy
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
                    self.text_area.cut();
                }
                Ok(true)
            }
            KeyCode::Char('y') => {
                if sync_os_clipboard {
                    self.exec_os_clipboard(ClipboardOperation::Copy)?;
                } else {
                    self.text_area.copy();
                }
                self.set_editor_mode(EditorMode::Normal);
                Ok(true)
            }
            KeyCode::Char('c') => {
                if sync_os_clipboard {
                    self.exec_os_clipboard(ClipboardOperation::Copy)?;
                } else {
                    self.text_area.cut();
                }
                self.set_editor_mode(EditorMode::Insert);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn handle_vim_motions(&mut self, input: &Input, sync_os_clipboard: bool) -> anyhow::Result<()> {
        let has_control = input.modifiers.contains(KeyModifiers::CONTROL);

        match (input.key_code, has_control) {
            (KeyCode::Char('h'), false) => {
                self.text_area.move_cursor(CursorMove::Back);
            }
            (KeyCode::Char('j'), false) => {
                self.text_area.move_cursor(CursorMove::Down);
            }
            (KeyCode::Char('k'), false) => {
                self.text_area.move_cursor(CursorMove::Up);
            }
            (KeyCode::Char('l'), false) => {
                self.text_area.move_cursor(CursorMove::Forward);
            }
            (KeyCode::Char('w'), false) | (KeyCode::Char('e'), false) => {
                self.text_area.move_cursor(CursorMove::WordForward);
            }
            (KeyCode::Char('b'), false) => {
                self.text_area.move_cursor(CursorMove::WordBack);
            }
            (KeyCode::Char('^'), false) => {
                self.text_area.move_cursor(CursorMove::Head);
            }
            (KeyCode::Char('$'), false) => {
                self.text_area.move_cursor(CursorMove::End);
            }
            (KeyCode::Char('D'), false) => {
                self.text_area.delete_line_by_end();
                self.exec_os_clipboard(ClipboardOperation::Copy)?;
            }
            (KeyCode::Char('C'), false) => {
                self.text_area.delete_line_by_end();
                self.exec_os_clipboard(ClipboardOperation::Copy)?;
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('p'), false) => {
                if sync_os_clipboard {
                    self.exec_os_clipboard(ClipboardOperation::Paste)?;
                } else {
                    self.text_area.paste();
                }
            }
            (KeyCode::Char('u'), false) => {
                self.text_area.undo();
            }
            (KeyCode::Char('r'), true) => {
                self.text_area.redo();
            }
            (KeyCode::Char('x'), false) => {
                self.text_area.delete_next_char();
                self.exec_os_clipboard(ClipboardOperation::Copy)?;
            }
            (KeyCode::Char('i'), false) => self.mode = EditorMode::Insert,
            (KeyCode::Char('a'), false) => {
                self.text_area.move_cursor(CursorMove::Forward);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('A'), false) => {
                self.text_area.move_cursor(CursorMove::End);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('o'), false) => {
                self.text_area.move_cursor(CursorMove::End);
                self.text_area.insert_newline();
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('O'), false) => {
                self.text_area.move_cursor(CursorMove::Head);
                self.text_area.insert_newline();
                self.text_area.move_cursor(CursorMove::Up);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('I'), false) => {
                self.text_area.move_cursor(CursorMove::Head);
                self.mode = EditorMode::Insert;
            }
            (KeyCode::Char('d'), true) => {
                self.text_area.scroll(Scrolling::HalfPageDown);
            }
            (KeyCode::Char('u'), true) => {
                self.text_area.scroll(Scrolling::HalfPageUp);
            }
            (KeyCode::Char('f'), true) => {
                self.text_area.scroll(Scrolling::PageDown);
            }
            (KeyCode::Char('b'), true) => {
                self.text_area.scroll(Scrolling::PageUp);
            }
            _ => {}
        }

        Ok(())
    }

    pub fn get_editor_mode(&self) -> EditorMode {
        self.mode
    }

    pub fn set_editor_mode(&mut self, mode: EditorMode) {
        match (self.mode, mode) {
            (EditorMode::Normal, EditorMode::Visual) => {
                self.text_area.start_selection();
            }
            (EditorMode::Visual, EditorMode::Normal | EditorMode::Insert) => {
                self.text_area.cancel_selection();
            }
            _ => {}
        }

        // When switching to non-normal modes we don't automatically change the Entry box state.
        // Entry box remains controlled by explicit toggles to avoid interfering with editor keybindings.
        self.mode = mode;
    }

    /// Renders the Entry box (single row) on top and the editor content below.
    /// This keeps the external caller unchanged — just pass the same `area` you used to pass.
    pub fn render_widget(&mut self, frame: &mut Frame, area: Rect, styles: &Styles) {
        // split area: small fixed height for Entry (3) and the rest for the content editor
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(area);

        // Render entry box in the top chunk.
        self.entry_box.render(frame, chunks[0], self.entry_box.is_active());

        // The rest of the rendering for the editor stays the same, but uses chunks[1] now.
        let mut title = "Content".to_owned();
        if self.is_active {
            let mode_caption = match self.mode {
                EditorMode::Normal => " - NORMAL",
                EditorMode::Insert => " - EDIT",
                EditorMode::Visual => " - Visual",
            };
            title.push_str(mode_caption);
        }
        if self.has_unsaved {
            title.push_str(" *");
        }

        let estyles = &styles.editor;

        let text_block_style = match (self.mode, self.is_active) {
            (EditorMode::Insert, _) => estyles.block_insert,
            (EditorMode::Visual, _) => estyles.block_visual,
            (EditorMode::Normal, true) => estyles.block_normal_active,
            (EditorMode::Normal, false) => estyles.block_normal_inactive,
        };

        self.text_area.set_block(
            Block::default()
                .borders(Borders::ALL)
                .style(text_block_style)
                .title(title),
        );

        let cursor_style = if self.is_active {
            let s = match self.mode {
                EditorMode::Normal => estyles.cursor_normal,
                EditorMode::Insert => estyles.cursor_insert,
                EditorMode::Visual => estyles.cursor_visual,
            };
            Style::from(s)
        } else {
            Style::reset()
        };
        self.text_area.set_cursor_style(cursor_style);

        self.text_area.set_cursor_line_style(Style::reset());

        self.text_area.set_style(Style::reset());

        self.text_area
            .set_selection_style(Style::default().bg(Color::White).fg(Color::Black));

        // Render the content into the lower chunk
        frame.render_widget(&self.text_area, chunks[1]);

        self.render_vertical_scrollbar(frame, chunks[1]);
        self.render_horizontal_scrollbar(frame, chunks[1]);
    }

    pub fn render_vertical_scrollbar(&mut self, frame: &mut Frame, area: Rect) {
        let lines_count = self.text_area.lines().len();

        if lines_count as u16 <= area.height - 2 {
            return;
        }

        let (row, _) = self.text_area.cursor();

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

    pub fn render_horizontal_scrollbar(&mut self, frame: &mut Frame, area: Rect) {
        let max_width = self
            .text_area
            .lines()
            .iter()
            .map(|line| line.len())
            .max()
            .unwrap_or_default();

        if max_width as u16 <= area.width - 2 {
            return;
        }

        let (_, col) = self.text_area.cursor();

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
        if !active && self.is_visual_mode() {
            self.set_editor_mode(EditorMode::Normal);
        }

        // when losing overall focus we also make sure the entry box is deactivated
        if !active {
            self.entry_box.set_active(false);
        }

        self.is_active = active;
    }

    /// Programmatically activate/deactivate the Entry box (useful for wiring a toggle key).
    ///pub fn set_entry_active(&mut self, active: bool) {
    ///    self.entry_box.set_active(active);
    ///}

    /// Toggle entry box active state.
    ///pub fn toggle_entry_active(&mut self) {
    ///    let new_state = !self.entry_box.is_active();
    ///    self.entry_box.set_active(new_state);
    ///}

    pub fn get_content(&self) -> String {
        let lines = self.text_area.lines().to_vec();

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
        text_area.move_cursor(tui_textarea::CursorMove::Bottom);
        text_area.move_cursor(tui_textarea::CursorMove::End);

        self.text_area = text_area;

        self.refresh_has_unsaved(app);
    }

    pub fn exec_os_clipboard(
        &mut self,
        operation: ClipboardOperation,
    ) -> anyhow::Result<HandleInputReturnType> {
        let mut clipboard = Clipboard::new().map_err(map_clipboard_error)?;

        match operation {
            ClipboardOperation::Copy => {
                self.text_area.copy();
                let selected_text = self.text_area.yank_text();
                clipboard
                    .set_text(selected_text)
                    .map_err(map_clipboard_error)?;
            }
            ClipboardOperation::Cut => {
                if self.text_area.cut() {
                    self.is_dirty = true;
                    self.has_unsaved = true;
                }
                let selected_text = self.text_area.yank_text();
                clipboard
                    .set_text(selected_text)
                    .map_err(map_clipboard_error)?;
            }
            ClipboardOperation::Paste => {
                let content = clipboard.get_text().map_err(map_clipboard_error)?;
                if content.is_empty() {
                    return Ok(HandleInputReturnType::Handled);
                }

                if !self.text_area.insert_str(content) {
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
        "Error while communicating with the operation system clipboard.\nError Details: {}",
        err.to_string()
    )
}
