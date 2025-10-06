use chrono::Datelike;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    prelude::Margin,
    style::Style,
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};

use backend::DataProvider;

use crate::app::App;
use crate::settings::DatumVisibility;

use super::Styles;

const LIST_INNER_MARGIN: usize = 5;

#[derive(Debug)]
pub struct EntriesList {
    pub state: ListState,
    is_active: bool,
    pub multi_select_mode: bool,
}

impl EntriesList {
    pub fn new() -> Self {
        Self {
            state: ListState::default(),
            is_active: false,
            multi_select_mode: false,
        }
    }

    /// Set the active state
    pub fn set_active(&mut self, active: bool) {
        self.is_active = active;
    }

    /// Render the widget (called from higher-level UI)
    pub fn render_widget<D: DataProvider>(
        &mut self,
        frame: &mut Frame,
        app: &App<D>,
        area: Rect,
        styles: &Styles,
    ) {
        self.render_list(frame, app, area, styles);
    }

    /// Internal method to render the list
    fn render_list<D: DataProvider>(
        &mut self,
        frame: &mut Frame,
        app: &App<D>,
        area: Rect,
        styles: &Styles,
    ) {
        let jstyles = &styles.journals_list;
        let mut lines_count = 0;

        let mut prev_date: Option<(i32, u32, u32)> = None;
        let items: Vec<ListItem> = app
            .get_active_entries()
            .map(|entry| {
                let current_date = (
                    entry.date.year(),
                    entry.date.month(),
                    entry.date.day(),
                );

                let mut title_text = entry.title.to_string();
                if let Some(pd) = prev_date {
                    if pd == current_date {
                        title_text.insert_str(0, "    ");
                    }
                }
                prev_date = Some(current_date);

                let title_lines =
                    textwrap::wrap(&title_text, area.width as usize - LIST_INNER_MARGIN);
                lines_count += title_lines.len();

                let highlight_selected =
                    self.multi_select_mode && app.selected_entries.contains(&entry.id);
                let title_style = match (self.is_active, highlight_selected) {
                    (_, true) => jstyles.title_selected,
                    (true, _) => jstyles.title_active,
                    (false, _) => jstyles.title_inactive,
                };
                let mut spans: Vec<Line> = title_lines
                    .iter()
                    .map(|line| Line::from(Span::styled(line.to_string(), title_style)))
                    .collect();

                // Date and priority
                let date_priority_lines = match (app.settings.datum_visibility, entry.priority) {
                    (DatumVisibility::Show, Some(prio)) => {
                        let oneliner = format!(
                            "{},{},{} | Priority: {}",
                            entry.date.day(),
                            entry.date.month(),
                            entry.date.year(),
                            prio
                        );
                        if oneliner.len() > area.width as usize - LIST_INNER_MARGIN {
                            vec![
                                format!(
                                    "{},{},{}",
                                    entry.date.day(),
                                    entry.date.month(),
                                    entry.date.year()
                                ),
                                format!("Priority: {prio}"),
                            ]
                        } else {
                            vec![oneliner]
                        }
                    }
                    (DatumVisibility::Show, None) => {
                        vec![format!(
                            "{},{},{}",
                            entry.date.day(),
                            entry.date.month(),
                            entry.date.year()
                        )]
                    }
                    (DatumVisibility::Hide, None) => Vec::new(),
                    (DatumVisibility::EmptyLine, None) => vec![String::new()],
                    (_, Some(prio)) => vec![format!("Priority: {}", prio)],
                };

                let date_lines = date_priority_lines
                    .iter()
                    .map(|line| Line::from(Span::styled(line.to_string(), jstyles.date_priority)));
                spans.extend(date_lines);
                lines_count += date_priority_lines.len();

                // Tags
                if !entry.tags.is_empty() {
                    const TAGS_SEPARATOR: &str = " | ";
                    let tags_default_style: Style = jstyles.tags_default.into();
                    let mut added_lines = 1;
                    spans.push(Line::default());

                    for tag in entry.tags.iter() {
                        let mut last_line = spans.last_mut().unwrap();
                        let allowd_width = area.width as usize - LIST_INNER_MARGIN;
                        if !last_line.spans.is_empty() {
                            if last_line.width() + TAGS_SEPARATOR.len() > allowd_width {
                                added_lines += 1;
                                spans.push(Line::default());
                                last_line = spans.last_mut().unwrap();
                            }
                            last_line.push_span(Span::styled(TAGS_SEPARATOR, tags_default_style))
                        }

                        let style = app
                            .get_color_for_tag(tag)
                            .map(|c| Style::default().bg(c.background).fg(c.foreground))
                            .unwrap_or(tags_default_style);
                        let span_to_add = Span::styled(tag.to_owned(), style);
                        if last_line.width() + tag.len() < allowd_width {
                            last_line.push_span(span_to_add);
                        } else {
                            added_lines += 1;
                            spans.push(Line::from(span_to_add));
                        }
                    }
                    lines_count += added_lines;
                }

                ListItem::new(spans)
            })
            .collect();

        let items_count = items.len();
        let highlight_style = if self.is_active {
            jstyles.highlight_active
        } else {
            jstyles.highlight_inactive
        };

        let list = List::new(items)
            .block(self.get_list_block(app.filter.is_some(), Some(items_count), styles))
            .highlight_style(highlight_style)
            .highlight_symbol("> ");

        frame.render_stateful_widget(list, area, &mut self.state);

        if lines_count > area.height as usize - 2 {
            let avg_item_height = lines_count / items_count;
            self.render_scrollbar(
                frame,
                area,
                self.state.selected().unwrap_or(0),
                items_count,
                avg_item_height,
            );
        }
    }

    /// Returns the block widget for the list
    fn get_list_block(
        &self,
        filtered: bool,
        _items_count: Option<usize>,
        styles: &Styles,
    ) -> Block {
        let title = if filtered { "Filtered Entries" } else { "Entries" };
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(styles.journals_list.title_inactive)
    }

    /// Scrollbar rendering stub (fill in actual logic later)
    fn render_scrollbar(
        &self,
        _frame: &mut Frame,
        _area: Rect,
        _selected_index: usize,
        _items_count: usize,
        _avg_item_height: usize,
    ) {
        // TODO: implement actual scrollbar rendering
    }
}
