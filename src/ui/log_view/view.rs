//! Central log view: virtualized with `show_rows`, so a 5M-line file renders
//! the same ~40 visible rows per frame as a 40-line file. Each row shows the
//! line number, the Drain template ID, and filter-highlighted text.
//!
//! Also provides a right-click context menu (pin / add analysis) and a
//! scroll-position indicator bar on the right edge.

use std::cell::Cell;

use eframe::egui;
use egui::{Color32, FontId, Pos2, Rect, RichText, Stroke, StrokeKind};

use logotomy::core::document::LogDocument;
use logotomy::core::time::format_ms;

use crate::ui::app::model::{PinEntry, Filter, LogTab, TrimAction};
use crate::ui::icons::{self, Icon};
use crate::ui::theme::Theme;

/// Characters beyond this are cut off when rendering a single row.
/// (The full bytes stay in the mmap; we just don't paint a novel per frame.)
const MAX_DISPLAY_CHARS: usize = 2000;
/// Width of the scroll-position indicator bar.
const SCROLL_BAR_WIDTH: f32 = 8.0;
/// Minimum pointer displacement (px) to distinguish a drag from a click.
const DRAG_THRESHOLD: f32 = 3.0;

/// Action returned from a single row render, to be applied after the
/// scroll-area closure so we avoid borrow conflicts with `tab`.
enum RowAction {
    Select,
    Pin,
    TrimRight,
    TrimLeft,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn show(ui: &mut egui::Ui, tab: &mut LogTab, theme: &Theme) {
    let n = tab.doc.total_lines();
    if n == 0 {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Empty file. Nothing to heck about.").color(theme.placeholder));
        });
        return;
    }

    // ---- font size + row height ----
    // Log text uses the embedded Space Mono face (see `ui/fonts`).
    let font_id = crate::ui::fonts::log_font(tab.log_font_size);
    let row_height = ui.ctx().fonts_mut(|f| f.row_height(&font_id)) + 2.0;
    let avail_height = ui.available_height();
    let max_visible_lines = if row_height > 0.0 {
        (avail_height / row_height).floor() as usize
    } else {
        0
    };

    // ---- toolbar (font size controls + lines-visible label) ----
    show_toolbar(ui, tab, theme, max_visible_lines);

    // Deferred actions from context menu (avoid borrow conflicts inside show_rows).
    let mut context_pin: Option<usize> = None;
    let mut context_trim: Option<TrimAction> = None;
    let mut suppress_select: bool = false;

    let total_visible = match &tab.visible_lines {
        Some(vis) => vis.len(),
        None => n,
    };

    // Reserve space for the scroll bar on the right edge.
    let available = egui::Vec2::new(ui.available_width() - SCROLL_BAR_WIDTH - 4.0, ui.available_height());

    // Cell to capture the exact rendered range from show_rows, avoiding
    // offset-based approximation that can drift due to partial rows and egui buffering.
    let rendered_range: Cell<Option<(usize, usize)>> = Cell::new(None);
    // Take the pending scroll before entering the closure to avoid borrow conflicts.
    let pending = tab.pending_scroll.take();
    // One-shot preserve-anchor set by a filter change; top-aligns the viewport.
    let preserve_anchor = tab.preserve_anchor.take();

    // ---- scroll area setup (vertical + horizontal) ----
    // Compute the horizontal scroll extent from the longest line.
    let char_width = ui.ctx().fonts_mut(|f| f.glyph_width(&font_id, ' '));
    let max_line_px = tab.doc.max_line_width as f32 * char_width + 80.0; // 80px for line number gutter
    let mut scroll_area = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt("log_scroll")
        .max_width(max_line_px);

    if let Some(offset) = compute_pending_scroll_offset(tab, pending, row_height, avail_height, total_visible) {
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    } else if let Some(anchor) = preserve_anchor {
        // No pending scroll (diamond click etc.), so honor a filter-change
        // anchor by top-aligning the preserved reference line.
        if let Some(offset) = compute_preserve_anchor_offset(tab, anchor, row_height, total_visible) {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }
    }

    // ---- set item_spacing.y = 0.0 BEFORE show_rows so egui's internal
    //      row_height_with_spacing == row_height (no drift) ----
    ui.spacing_mut().item_spacing.y = 0.0;

    // Use an inner area with reduced width for the log content.
    let inner_resp = ui.allocate_ui_with_layout(
        available,
        egui::Layout::top_down_justified(egui::Align::LEFT),
        |ui| {
            let output = scroll_area.show_rows(ui, row_height, total_visible, |ui, range| {
                // Capture the exact range egui determined to be visible.
                // range.end is exclusive, so last = end - 1.
                if !range.is_empty() {
                    rendered_range.set(Some((range.start, range.end - 1)));
                }
                for vi in range {
                    let i = match &tab.visible_lines {
                        Some(vis) => vis[vi],
                        None => vi,
                    };
                    let selected = tab.context_line == Some(i);

                    let action = render_row(
                        ui,
                        &tab.doc,
                        &tab.filters,
                        tab.highlighter.as_deref(),
                        i,
                        selected,
                        font_id.clone(),
                        theme,
                        row_height,
                        tab.selection_range,
                    );

                    let is_select = matches!(action, Some(RowAction::Select));
                    match action {
                        Some(RowAction::Select) if !suppress_select => {
                            tab.context_line = Some(i);
                            tab.ensure_visible();
                        }
                        Some(RowAction::Pin) => {
                            context_pin = Some(i);
                        }
                        Some(RowAction::TrimRight) => {
                            context_trim = Some(TrimAction::TrimRight(i));
                        }
                        Some(RowAction::TrimLeft) => {
                            context_trim = Some(TrimAction::TrimLeft(i));
                        }
                        _ => {}
                    }

                    // If this row was a click but we later determine it was actually a drag,
                    // suppress the select action. For simplicity, we track whether any row
                    // received a click this frame and suppress on the next frame if drag was detected.
                    if is_select && tab.drag_selecting {
                        suppress_select = true;
                    }
                }
            });
            output
        },
    );
    let output = inner_resp.inner;

    // Apply deferred context menu actions.
    apply_context_actions(tab, context_pin, context_trim);

    // ---- Scroll-position indicator bar (overlay on the right edge) ----
    draw_scroll_indicator(ui, &output, theme);

    // ---- compute viewport_range for timeline shadow ----
    update_viewport_range(tab, &rendered_range, pending);

    // ---- pointer-driven drag selection ----
    let pointer = ui.input(|i| i.pointer.clone());
    let inner_rect = output.inner_rect;

    if pointer.primary_pressed() {
        if let Some(press_pos) = pointer.latest_pos() {
            if inner_rect.contains(press_pos) {
                // Don't start a new drag if a popup is visible — the press is on the buttons
                if tab.pending_selection.is_none() && tab.pin_modal.is_none() {
                    tab.drag_selecting = true;
                    tab.drag_start_pos = Some(press_pos);
                    if let Some(row) = row_under_pointer(ui, inner_rect, output.state.offset.y, row_height, total_visible, &tab.visible_lines) {
                        tab.drag_start_line = Some(row);
                        tab.drag_current_line = Some(row);
                        tab.selection_range = Some((row, row));
                    }
                }
            }
        }
    }

    if tab.drag_selecting {
        if let Some(row) = row_under_pointer(ui, inner_rect, output.state.offset.y, row_height, total_visible, &tab.visible_lines) {
            tab.drag_current_line = Some(row);
            if let (Some(start), Some(current)) = (tab.drag_start_line, tab.drag_current_line) {
                let lo = start.min(current);
                let hi = start.max(current);
                tab.selection_range = Some((lo, hi));
            }
        }

        // Use !primary_down() instead of primary_released() because
        // pointer.released can be consumed by show_rows or other widgets
        if !pointer.primary_down() {
            tab.drag_selecting = false;
            let release_pos = pointer.latest_pos();
            let is_drag = tab.drag_start_pos.zip(release_pos).is_some_and(|(start, end)| start.distance(end) >= DRAG_THRESHOLD);
            if is_drag {
                if let (Some(start), Some(end)) = (tab.drag_start_line, tab.drag_current_line) {
                    let lo = start.min(end);
                    let hi = start.max(end);
                    tab.pending_selection = Some((lo, hi));
                } else {
                    tab.selection_range = None;
                    tab.pending_selection = None;
                }
            } else {
                tab.selection_range = None;
                tab.pending_selection = None;
            }
            tab.drag_start_line = None;
            tab.drag_current_line = None;
            if tab.pending_selection.is_none() {
                tab.drag_start_pos = None;
            }
        }
    }

    // If the primary button was released outside a drag (no pending_selection),
    // clear transient selection so it doesn't linger.
    if !tab.drag_selecting && tab.selection_range.is_some() && tab.pending_selection.is_none() && !pointer.primary_down() {
        tab.selection_range = None;
    }

    // ---- selection popup (single "📌 Pin" button) ----
    if let Some(pending_range) = tab.pending_selection {
        let (start, end) = pending_range;
        let count = match &tab.visible_lines {
            Some(vis) => vis.iter().filter(|&&ln| ln >= start && ln <= end).count(),
            None => end - start + 1,
        };
 
        let popup_id = egui::Id::new("selection_popup");
        let popup_anchor_pos = tab.drag_start_pos.unwrap_or_else(|| {
            ui.input(|i| i.pointer.latest_pos().unwrap_or_default())
        });
        let popup_pos = popup_anchor_pos + egui::vec2(8.0, 8.0);

        let area = egui::Area::new(popup_id)
            .current_pos(popup_pos)
            .order(egui::Order::Foreground)
            .fixed_pos(popup_pos);
        let area_resp = area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(240.0);
                ui.label(RichText::new(format!("Selected: {} lines ({}-{})", count, start, end)).strong().size(13.0));
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Pin").clicked() {
                        tab.pin_modal = Some(pending_range);
                        tab.pin_comment.clear();
                        tab.pending_selection = None;
                        tab.drag_start_pos = None;
                    }
                });

                if ui.button("Cancel").clicked() {
                    tab.selection_range = None;
                    tab.pending_selection = None;
                    tab.drag_start_pos = None;
                }
            });
        });

        // Close on click outside
        if ui.input(|i| i.pointer.any_click()) {
            if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
                if !area_resp.response.rect.contains(click_pos) {
                    tab.selection_range = None;
                    tab.pending_selection = None;
                    tab.drag_start_pos = None;
                }
            }
        }
    }

    // (The pin modal moved to `pin_modal_ui`, drawn at the app level so it
    // works even when the Log view isn't the focused dock tab.)
}

/// Render the pin creation/editing modal (comment + log preview). Called from
/// the app level so it works regardless of which dock tab or detached
/// viewport is focused. Reuses the same window for creating a new pin and for
/// editing an existing one (when `tab.pin_edit_index` is set).
pub fn pin_modal_ui(ui: &mut egui::Ui, tab: &mut LogTab, theme: &Theme) {
    let Some(range) = tab.pin_modal else { return };
    let (start, end) = range;
    // Compute the actual visible lines within the range (respects text filters).
    let visible_in_range: Vec<usize> = match &tab.visible_lines {
        Some(vis) => vis.iter()
            .filter(|&&ln| ln >= start && ln <= end)
            .copied()
            .collect(),
        None => (start..=end).collect(),
    };
    let actual_count = visible_in_range.len();

    let ts = if tab.doc.ts_at_opt(start).is_some_and(|v| v >= 0) {
        format_ms(tab.doc.ts_at(start))
    } else {
        format!("line {}", start + 1)
    };

    let editing = tab.pin_edit_index.is_some();
    let title = if editing {
        format!("Edit pin — {} lines", actual_count)
    } else {
        format!("Pin — {} lines", actual_count)
    };
    let font_id = crate::ui::fonts::log_font(tab.log_font_size);

    let mut open = true;
    let mut do_save = false;
    let mut do_cancel = false;
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(500.0)
        .default_height(400.0)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                let ctx = ui.ctx().clone();
                ui.add(icons::icon_image(&ctx, Icon::Date, 13.0, theme.text));
                ui.label(RichText::new(format!("Viewed on {ts}")).strong().size(13.0));
            });
            ui.separator();

            ui.label(RichText::new("Comment (optional):").strong());
            let text_resp = ui.add_sized(
                egui::vec2(ui.available_width(), 60.0),
                egui::TextEdit::multiline(&mut tab.pin_comment)
                    .hint_text("Type your comment. Enter to save, Shift+Enter for newline, Esc to cancel…")
                    .desired_width(f32::INFINITY),
            );
            // Auto-focus the text input when the modal opens
            text_resp.request_focus();

            // Save on Enter (no Shift), Esc to cancel
            if text_resp.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.input(|i| i.modifiers.shift) {
                    do_save = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    do_cancel = true;
                }
            }

            // Also handle Enter/Esc on the TextEdit while focused (not just on lost_focus)
            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.input(|i| i.modifiers.shift);
            let esc_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if text_resp.has_focus() {
                if enter_pressed {
                    do_save = true;
                }
                if esc_pressed {
                    do_cancel = true;
                }
            }

            ui.add_space(8.0);
            ui.label(RichText::new("Selected lines:").strong().size(11.0));

            let max_preview = 20;
            if actual_count > max_preview {
                egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, false]).show(ui, |ui| {
                    for &line_idx in visible_in_range.iter().take(max_preview / 2) {
                        let job = line_job(&tab.doc, &tab.filters, tab.highlighter.as_deref(), line_idx, false, font_id.clone(), theme);
                        ui.add(egui::Label::new(job));
                    }
                    ui.label(RichText::new(format!("… {} more lines …", actual_count - max_preview)).italics().color(theme.text_muted));
                    for &line_idx in visible_in_range.iter().rev().take(max_preview / 2).rev() {
                        let job = line_job(&tab.doc, &tab.filters, tab.highlighter.as_deref(), line_idx, false, font_id.clone(), theme);
                        ui.add(egui::Label::new(job));
                    }
                });
            } else {
                egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, false]).show(ui, |ui| {
                    for &line_idx in &visible_in_range {
                        let job = line_job(&tab.doc, &tab.filters, tab.highlighter.as_deref(), line_idx, false, font_id.clone(), theme);
                        ui.add(egui::Label::new(job));
                    }
                });
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    do_save = true;
                }
                if ui.button("Cancel").clicked() {
                    do_cancel = true;
                }
            });
        });

    if do_save {
        save_pin(tab, range);
    } else if do_cancel || !open {
        tab.pin_modal = None;
        tab.pin_edit_index = None;
        tab.pin_comment.clear();
    }
}
/// Save the current pin modal range as a PinEntry and close the modal.
/// If `tab.pin_edit_index` is set, the matching entry is updated in place
/// instead of pushing a brand-new pin.
#[inline]
fn save_pin(tab: &mut LogTab, range: (usize, usize)) {
    let (start, end) = range;
    // When visible_lines is active (filtered view), only include lines that
    // are actually visible, since the user's selection spans virtual indices.
    let line_numbers: Vec<usize> = match &tab.visible_lines {
        Some(vis) => vis.iter()
            .filter(|&&ln| ln >= start && ln <= end)
            .copied()
            .collect(),
        None => (start..=end).collect(),
    };
    let start_ts = tab.doc.ts_at_opt(start).unwrap_or(-1);
    let end_ts = tab.doc.ts_at_opt(end).unwrap_or(-1);
    let comment = tab.pin_comment.trim().to_string();

    if let Some(idx) = tab.pin_edit_index {
        // Editing an existing pin: replace its content in place.
        if idx < tab.pins.len() {
            let p = &mut tab.pins[idx];
            p.start_line = start;
            p.line_numbers = line_numbers;
            p.start_ts = start_ts;
            p.end_ts = end_ts;
            p.comment = comment;
        }
    } else {
        tab.pins.push(PinEntry {
            start_line: start,
            line_numbers,
            start_ts,
            end_ts,
            comment,
        });
    }
    tab.pin_modal = None;
    tab.pin_edit_index = None;
    tab.pin_comment.clear();
    tab.bottom_panel_open = true;
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Font-size buttons, trim indicator, and "lines visible" label.
fn show_toolbar(ui: &mut egui::Ui, tab: &mut LogTab, theme: &Theme, _max_visible_lines: usize) {
    ui.horizontal(|ui| {
        if ui.button("A-").on_hover_text("Decrease text size").clicked() {
            tab.log_font_size = (tab.log_font_size - 1.0).max(8.0);
        }
        if ui.button("A+").on_hover_text("Increase text size").clicked() {
            tab.log_font_size = (tab.log_font_size + 1.0).min(24.0);
        }
        ui.label(RichText::new(format!("{:.0}px", tab.log_font_size)).monospace().color(theme.text_muted));
        ui.add_space(8.0);

        // Trim indicator + reset button
        if tab.doc.is_trimmed() {
            let total = tab.doc.total_lines_untrimmed();
            let current = tab.doc.total_lines();
            let ctx = ui.ctx().clone();
            ui.add(icons::icon_image(&ctx, Icon::Trim, 12.0, theme.warning));
            ui.label(RichText::new(format!("{} / {} lines", current, total)).color(theme.warning).size(11.0));
            if ui.button("Reset").on_hover_text("Reset trim to show all lines").clicked() {
                tab.handle_trim_reset();
            }
        }
        
        show_line_count(ui, tab, theme);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let scroll_bar_width = SCROLL_BAR_WIDTH;
            ui.add_space(scroll_bar_width + 8.0);
        });
    });
}

/// Display total and filtered line counts.
fn show_line_count(ui: &mut egui::Ui, tab: &LogTab, theme: &Theme) {
    
    let total_lines = tab.doc.total_lines();
    if let Some(visible) = &tab.visible_lines {
        ui.label(
            RichText::new(format!("{} / {} lines", visible.len(), total_lines))
                .small()
                .color(theme.text_muted),
        );
    } else {
        ui.label(RichText::new(format!("{} lines", total_lines)).small().color(theme.text_muted));
    }
}

/// Compute the deterministic scroll offset for a pending scroll-to-line,
/// centering the target line in the viewport. Returns `None` when no
/// pending scroll is needed.
fn compute_pending_scroll_offset(
    tab: &LogTab,
    pending: Option<usize>,
    row_height: f32,
    avail_height: f32,
    total_visible: usize,
) -> Option<f32> {
    let line = pending?;
    let vis_idx = match &tab.visible_lines {
        Some(ref vis) => match vis.binary_search(&line) {
            Ok(idx) => idx,
            Err(idx) => idx.min(vis.len().saturating_sub(1)),
        },
        None => line.min(total_visible.saturating_sub(1)),
    };
    let center_offset = vis_idx as f32 * row_height - (avail_height - row_height) * 0.5;
    Some(center_offset.max(0.0))
}

/// Compute a top-aligned scroll offset that places the preserved anchor line
/// at the top of the viewport. Used after a lane-filter change to keep the
/// previously visible log content in the window.
fn compute_preserve_anchor_offset(
    tab: &LogTab,
    anchor: usize,
    row_height: f32,
    total_visible: usize,
) -> Option<f32> {
    let vis_idx = match &tab.visible_lines {
        Some(ref vis) => match vis.binary_search(&anchor) {
            Ok(idx) => idx,
            Err(idx) => idx.min(vis.len().saturating_sub(1)),
        },
        None => anchor.min(total_visible.saturating_sub(1)),
    };
    Some(vis_idx as f32 * row_height)
}

/// Calculate which real log line index is under the pointer, if any.
fn row_under_pointer(
    ui: &egui::Ui,
    inner_rect: Rect,
    scroll_offset: f32,
    row_height: f32,
    total_visible: usize,
    visible_lines: &Option<Vec<usize>>,
) -> Option<usize> {
    let pointer = ui.input(|i| i.pointer.latest_pos())?;
    if pointer.x < inner_rect.left() || pointer.x > inner_rect.right()
        || pointer.y < inner_rect.top() || pointer.y > inner_rect.bottom() {
        return None;
    }
    let relative_y = pointer.y - inner_rect.top() + scroll_offset;
    let virtual_idx = (relative_y / row_height).floor() as usize;
    let virtual_idx = virtual_idx.min(total_visible.saturating_sub(1));
    match visible_lines {
        Some(vis) => vis.get(virtual_idx).copied(),
        None => Some(virtual_idx),
    }
}

/// Render a single log row, returning an action to be applied by the caller.
/// The line number is shown as a separate non-interactive label (not selectable
/// during drag), followed by the log content with filter highlighting.
fn render_row(
    ui: &mut egui::Ui,
    doc: &LogDocument,
    filters: &[Filter],
    highlighter: Option<&aho_corasick::AhoCorasick>,
    idx: usize,
    selected: bool,
    font_id: FontId,
    theme: &Theme,
    row_height: f32,
    selection_range: Option<(usize, usize)>,
) -> Option<RowAction> {
    let mut action: Option<RowAction> = None;

    let in_selection = selection_range.is_some_and(|(lo, hi)| idx >= lo && idx <= hi);
    let bg = if selected {
        theme.selection_bg
    } else if in_selection {
        theme.selection_range_bg
    } else {
        Color32::TRANSPARENT
    };

    // Build the log content job (no line number, no color marker).
    let job = line_job(doc, filters, highlighter, idx, selected, font_id.clone(), theme);

    // Use a horizontal layout: line number (non-interactive, not selectable) + log content (clickable).
    ui
        .allocate_ui_with_layout(
            egui::vec2(ui.available_width(), row_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                // Line number label — non-interactive, not selectable during drag.
                let line_num_fmt = egui::text::TextFormat {
                    font_id: font_id.clone(),
                    color: theme.gutter,
                    background: bg,
                    ..Default::default()
                };
                let mut line_num_job = egui::text::LayoutJob::default();
                line_num_job.append(&format!("{:>7}: ", idx + 1), 0.0, line_num_fmt);
                ui.add(egui::Label::new(line_num_job).selectable(false));

                // Log content label — clickable for selection.
                let mut content_job = job;
                for galley in &mut content_job.sections {
                    galley.format.background = bg;
                }
                let content_resp = ui.add(
                    egui::Label::new(content_job)
                        .sense(egui::Sense::click()),
                );

                // Left-click: select line
                if content_resp.clicked() {
                    action = Some(RowAction::Select);
                }

                // Right-click: context menu (on the whole row area)
                content_resp.context_menu(|ui| {
                    ui.set_min_width(160.0);
                        if ui.button("Pin").clicked() {
                            action = Some(RowAction::Pin);
                            ui.close();
                    }
                    ui.separator();
                    if ui.button("Trim top").on_hover_text("Remove all lines before this one").clicked() {
                        action = Some(RowAction::TrimLeft);
                        ui.close();
                    }
                    if ui.button("Trim bottom").on_hover_text("Remove all lines after this one").clicked() {
                        action = Some(RowAction::TrimRight);
                        ui.close();
                    }
                });

                if content_resp.hovered() {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                }
            },
        )
        .inner;

    action
}

/// Apply the deferred pin / trim actions from the context menu.
/// Single-line right-click "📌 Pin" opens the pin modal.
fn apply_context_actions(tab: &mut LogTab, context_pin: Option<usize>, context_trim: Option<TrimAction>) {
    if let Some(line) = context_pin {
        tab.pin_modal = Some((line, line));
        tab.pin_comment.clear();
        tab.bottom_panel_open = true;
    }
    if let Some(action) = context_trim {
        tab.handle_trim(action);
    }
}

/// Draw the scroll-position indicator bar on the right edge of the log view.
fn draw_scroll_indicator(
    ui: &egui::Ui,
    output: &egui::scroll_area::ScrollAreaOutput<()>,
    theme: &Theme,
) {
    let viewport_h = output.inner_rect.height();
    let content_h = output.content_size.y;
    let bar_left = ui.available_width() - SCROLL_BAR_WIDTH;

    let track_rect = Rect::from_min_size(
        Pos2::new(bar_left, ui.min_rect().top()),
        egui::vec2(SCROLL_BAR_WIDTH, viewport_h),
    );
    let painter = ui.painter_at(track_rect);
    painter.rect_filled(track_rect, egui::CornerRadius::same(2), theme.minimap_bg);

    if content_h > 0.0 && content_h > viewport_h {
        let offset = output.state.offset.y;
        let thumb_height = (viewport_h / content_h) * viewport_h;
        let thumb_top = (offset / (content_h - viewport_h)) * (viewport_h - thumb_height);
        let thumb_rect = Rect::from_min_size(
            Pos2::new(bar_left, ui.min_rect().top() + thumb_top),
            egui::vec2(SCROLL_BAR_WIDTH, thumb_height.max(4.0)),
        );
        // Draw the indicator with a dark fill
        painter.rect_filled(thumb_rect, egui::CornerRadius::same(3), Color32::BLACK);
        // Draw a subtle border
        painter.rect_stroke(
            thumb_rect,
            egui::CornerRadius::same(3),
            Stroke::new(0.5_f32, theme.text_muted),
            StrokeKind::Middle,
        );
    }
}

/// Update `tab.viewport_range` for the timeline shadow, using the exact
/// rendered range from `show_rows`. If a pending scroll was just processed,
/// force the range to include the target line so the shadow immediately
/// covers the selection marker.
fn update_viewport_range(
    tab: &mut LogTab,
    rendered_range: &Cell<Option<(usize, usize)>>,
    pending: Option<usize>,
) {
    let mut forced_range: Option<(usize, usize)> = None;
    if pending.is_some() {
        if let Some(line) = tab.context_line {
            forced_range = Some((line, line));
        }
    }
    if let Some((first_virtual, last_virtual)) = rendered_range.get() {
        let map_to_real = |vi: usize| -> usize {
            match &tab.visible_lines {
                Some(vis) => {
                    if vi < vis.len() {
                        vis[vi]
                    } else {
                        vis.last().copied().unwrap_or(0)
                    }
                }
                None => vi,
            }
        };
        let first_real = map_to_real(first_virtual);
        let last_real = map_to_real(last_virtual);
        let merged = match (forced_range, first_real <= last_real) {
            (Some((f, l)), true) => Some((first_real.min(f), last_real.max(l))),
            (Some(range), false) => Some(range),
            (None, true) => Some((first_real, last_real)),
            (None, false) => None,
        };
        if let Some((fr, lr)) = merged {
            tab.viewport_range = Some((fr, lr));
        }
    } else if let Some(range) = forced_range {
        tab.viewport_range = Some(range);
    }

    // If the visible range has scrolled fully out of the current timeline view,
    // recenter the timeline zoom window on the shadow so the user never loses
    // their position (timeline "zoom slider" stays in sync with log scroll).
    tab.ensure_viewport_visible();
}

// ---------------------------------------------------------------------------
// Row styling (unchanged)
// ---------------------------------------------------------------------------

/// Build the styled LayoutJob for one log line: filter-highlighted text spans.
/// The line number and color marker are rendered separately in `render_row`.
/// Shared by the log view and context panel.
///
/// TODO: Make matched filters bold (requires per-span FontId changes in egui).
pub fn line_job(
    doc: &LogDocument,
    filters: &[Filter],
    highlighter: Option<&aho_corasick::AhoCorasick>,
    idx: usize,
    selected: bool,
    font_id: FontId,
    theme: &Theme,
) -> egui::text::LayoutJob {
    let bg = if selected {
        theme.selection_bg
    } else {
        Color32::TRANSPARENT
    };
    let mut job = egui::text::LayoutJob::default();

    let fmt = |color: Color32| egui::text::TextFormat {
        font_id: font_id.clone(),
        color,
        background: bg,
        ..Default::default()
    };

    let line = doc.line(idx);
    /*
    // For debug purpose 
    job.append(
        &format!("T{:<3} ", doc.template_at(idx)),
        0.0,
        fmt(theme.template_id),
        
    );
    */

    let truncated;
    let text: &str = if line.len() > MAX_DISPLAY_CHARS {
        truncated = format!(
            "{}  …[{} bytes total, truncated]",
            line.get(..MAX_DISPLAY_CHARS).unwrap_or(&line),
            line.len()
        );
        &truncated
    } else {
        &line
    };

    let base = theme.log_text;
    match highlighter {
        Some(ac) => append_highlighted(&mut job, text, ac, filters, fmt(base), font_id),
        None => {
            job.append(text, 0.0, fmt(base));
        }
    }
    job
}

/// Append `text`, coloring every filter occurrence with that filter's color.
/// First match wins when spans overlap.
fn append_highlighted(
    job: &mut egui::text::LayoutJob,
    text: &str,
    ac: &aho_corasick::AhoCorasick,
    filters: &[Filter],
    base_fmt: egui::text::TextFormat,
    font_id: FontId,
) {
    // Per-byte color assignment (lines are short; this is a couple KB max).
    let mut owner: Vec<Option<usize>> = vec![None; text.len()];
    let mut any = false;
    for m in ac.find_iter(text) {
        let kw = m.pattern().as_usize();
        for slot in &mut owner[m.start()..m.end()] {
            if slot.is_none() {
                *slot = Some(kw);
            }
        }
        any = true;
    }
    if !any {
        job.append(text, 0.0, base_fmt);
        return;
    }

    let mut pos = 0;
    while pos < text.len() {
        let cur = owner[pos];
        let mut end = pos + 1;
        while end < text.len() && owner[end] == cur {
            end += 1;
        }
        // Byte spans from Aho-Corasick are pattern-aligned, but grouping may
        // split a multibyte char — slice defensively.
        if let Some(seg) = text.get(pos..end) {
            match cur {
                Some(kw) => {
                    let color = filters
                        .get(kw)
                        .map(|k| k.color)
                        .unwrap_or(Color32::YELLOW);
                    // Use the filter color at ~0.2 alpha for the background.
                    let bg_alpha = Color32::from_rgba_unmultiplied(
                        color.r(),
                        color.g(),
                        color.b(),
                        51, // 51/255 ≈ 0.2
                    );
                    job.append(
                        seg,
                        0.0,
                        egui::text::TextFormat {
                            font_id: font_id.clone(),
                            color,
                            background: bg_alpha,
                            ..Default::default()
                        },
                    );
                }
                None => {
                    job.append(seg, 0.0, base_fmt.clone());
                }
            }
        }
        pos = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(content: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "logotomy_log_view_test_{}_{}.log",
            std::process::id(),
            n
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn save_pin_edits_existing_entry_in_place() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z INFO alpha\n\
             2026-07-19T10:00:01.000Z WARN beta\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);
        tab.pins.push(PinEntry {
            start_line: 0,
            line_numbers: vec![0, 1],
            start_ts: 0,
            end_ts: 1,
            comment: "old comment".into(),
        });

        // Simulate the pin-viewer edit flow: pre-fill the comment + flags, then save.
        tab.pin_comment = "updated comment".into();
        tab.pin_edit_index = Some(0);
        save_pin(&mut tab, (0, 1));

        assert_eq!(tab.pins.len(), 1, "editing must not add a second pin");
        assert_eq!(tab.pins[0].comment, "updated comment");
        assert_eq!(tab.pins[0].line_numbers, vec![0, 1]);
        assert!(tab.pin_modal.is_none());
        assert!(tab.pin_edit_index.is_none());
        assert!(tab.pin_comment.is_empty());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn save_pin_without_edit_flag_creates_a_new_pin() {
        let path = write_temp("2026-07-19T10:00:00.000Z INFO alpha\n");
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);
        tab.pin_comment = "fresh comment".into();
        save_pin(&mut tab, (0, 0));

        assert_eq!(tab.pins.len(), 1);
        assert_eq!(tab.pins[0].comment, "fresh comment");
        assert_eq!(tab.pins[0].line_numbers, vec![0]);

        std::fs::remove_file(path).ok();
    }
}