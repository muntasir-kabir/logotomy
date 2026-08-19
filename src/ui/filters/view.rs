//! Filter bookmark strip: add/remove filters, see live match counts.
//! Matches land on the timeline as colored markers once the background
//! scan finishes (a spinner shows while it's chewing).
//!
//! `add_filter_ui` renders a compact one-line input with an accent highlight,
//! meant to go in the toolbar. `show` renders the filter chip row.

use eframe::egui;
use egui::{Color32, RichText};

use crate::ui::app::model::{LogTab, MAX_FILTERS};
use crate::ui::icons::{self, Icon};
use crate::ui::theme::Theme;

/// Compact one-line Add Filter section — call this from the toolbar.
/// The input is disabled once the filter cap (MAX_FILTERS) is reached.
pub fn add_filter_ui(ui: &mut egui::Ui, tab: &mut LogTab, theme: &Theme) {
    let at_cap = tab.filters.len() >= MAX_FILTERS;
    let bg =
        Color32::from_rgba_unmultiplied(theme.accent.r(), theme.accent.g(), theme.accent.b(), 30);
    let border =
        Color32::from_rgba_unmultiplied(theme.accent.r(), theme.accent.g(), theme.accent.b(), 100);
    egui::Frame::default()
        .fill(bg)
        .stroke(egui::Stroke::new(1.0_f32, border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let ctx = ui.ctx().clone();
                ui.add_space(2.0);
                ui.add(icons::icon_image(&ctx, Icon::Key, 15.0, theme.text));
                let resp = ui
                    .add_enabled_ui(!at_cap, |ui| {
                        ui.add_sized(
                            egui::vec2(225.0, 0.0),
                            egui::TextEdit::singleline(&mut tab.filter_input)
                                .hint_text(if at_cap {
                                    "Max 20 filters"
                                } else {
                                    "filter + Enter"
                                })
                                .desired_width(225.0),
                        )
                    })
                    .inner;
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let add_clicked = ui
                    .add_enabled(
                        !at_cap,
                        egui::Button::new(RichText::new("Add").strong().size(13.0))
                            .fill(theme.accent)
                            .corner_radius(4.0),
                    )
                    .clicked();
                if (enter || add_clicked) && !tab.filter_input.trim().is_empty() {
                    let text = tab.filter_input.trim().to_string();
                    let color = theme.filter_colors[tab.filters.len() % theme.filter_colors.len()];
                    tab.push_filter(&text, color);
                    tab.filter_input.clear();
                    resp.request_focus();
                }
                if tab.search_rx.is_some() {
                    ui.spinner();
                }
            });
        });
}
