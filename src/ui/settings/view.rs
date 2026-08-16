use std::time::Duration;

use eframe::egui;
use egui::{Color32, RichText};

use crate::ui::icons::{self, Icon};
use crate::ui::theme::Theme;

use crate::ui::app::model::LogotomyApp;

/// Show the settings popup (Area-based, anchored to the settings button).
pub fn show_settings_popup(ui: &mut egui::Ui, app: &mut LogotomyApp) {
    if !app.show_settings_popup {
        return;
    }
    let Some(button_rect) = app.settings_button_rect else { return };

    let popup_id = egui::Id::new("settings_popup");
    let area = egui::Area::new(popup_id)
        .current_pos(button_rect.left_bottom())
        .order(egui::Order::Foreground)
        .fixed_pos(button_rect.left_bottom());
    let area_resp = area.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_min_width(300.0);
            ui.set_max_width(400.0);
            ui.horizontal(|ui| {
                let ctx = ui.ctx().clone();
                ui.add(icons::icon_image(&ctx, Icon::Settings, 14.0, app.theme.text));
                ui.label(RichText::new("Settings").strong().size(14.0));
            });
            ui.separator();

            // Dark mode
            let mut dark_mode = app.dark_mode;
            if ui.checkbox(&mut dark_mode, "Dark mode").clicked() {
                app.toggle_theme();
            }

            // Filter deletion confirmations
            let mut skip_confirm = app.settings.skip_filter_delete_confirm;
            if ui
                .checkbox(&mut skip_confirm, "Do not ask before deleting a filter")
                .on_hover_text("Deletes the filter (trash icon / 'Clear all filters') immediately, without the confirmation popup.")
                .changed()
            {
                app.settings.skip_filter_delete_confirm = skip_confirm;
                app.settings.save();
            }

            // Default template
            ui.horizontal(|ui| {
                ui.label("Default filter:");
                let selected_template = app.settings.default_filter.clone().unwrap_or_else(|| "None".to_string());
                egui::ComboBox::from_label("")
                    .selected_text(selected_template)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(app.settings.default_filter.is_none(), "None").clicked() {
                            app.settings.default_filter = None;
                            app.settings.save();
                        }
                        for filter_name in &app.available_filters {
                            if ui.selectable_label(app.settings.default_filter.as_deref() == Some(filter_name), filter_name).clicked() {
                                app.settings.default_filter = Some(filter_name.clone());
                                app.settings.save();
                            }
                        }
                    });
            });

            ui.separator();
            ui.horizontal(|ui| {
                let ctx = ui.ctx().clone();
                ui.add(icons::icon_image(&ctx, Icon::Log, 14.0, app.theme.text));
                ui.label(RichText::new("Log Parsing").strong().size(14.0));
            });
            ui.add_space(2.0);

            // Drain similarity threshold
            ui.horizontal(|ui| {
                ui.label("Similarity threshold:");
                let mut sim = app.settings.sim_threshold as f32;
                if ui.add(egui::DragValue::new(&mut sim).speed(0.01).range(0.3..=0.9)).changed() {
                    app.settings.sim_threshold = (sim as f64 * 100.0).round() / 100.0;
                    app.settings.save();
                }
            }).response.on_hover_text("Drain template merge threshold (0.3–0.9). Higher = stricter clustering, more templates. Applies to newly opened files.");

            // Header sample size
            ui.horizontal(|ui| {
                ui.label("Header sample lines:");
                let mut n = app.settings.header_sample_lines as u32;
                if ui.add(egui::DragValue::new(&mut n).speed(1).range(0..=2000)).changed() {
                    app.settings.header_sample_lines = n as usize;
                    app.settings.save();
                }
            }).response.on_hover_text("Leading lines sampled to learn the common log header (host/pid/thread slots). 0 disables header learning. Applies to newly opened files.");

            // Drain depth
            ui.horizontal(|ui| {
                ui.label("Drain depth:");
                let mut depth = app.settings.drain_depth as u32;
                if ui.add(egui::DragValue::new(&mut depth).speed(1).range(3..=8)).changed() {
                    app.settings.drain_depth = depth as usize;
                    app.settings.save();
                }
            }).response.on_hover_text("Drain parse-tree depth. Depth N = token count + (N-2) routing tokens. Higher = more precise routing but higher fragmentation risk. Default: 4. Applies to newly opened files.");

            ui.separator();
            ui.horizontal(|ui| {
                let ctx = ui.ctx().clone();
                ui.add(icons::icon_image(&ctx, Icon::Mcp, 14.0, app.theme.text));
                ui.label(RichText::new("MCP Server").strong().size(14.0));
            });
            ui.add_space(2.0);

            // Status indicator: green circle with pulse animation when running, gray when stopped
            let tooltip = if app.mcp_enabled {
                match app.mcp_connection_url() {
                    Some(url) => format!("MCP running at {url}"),
                    None => "MCP running".to_string(),
                }
            } else {
                "MCP not running".to_string()
            };
            ui.horizontal(|ui| {
                let color = if app.mcp_enabled {
                    let is_active = app.mcp_started_at.map(|t| t.elapsed() < Duration::from_secs(10)).unwrap_or(false);
                    let alpha = if is_active {
                        let t = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
                        let pulse = ((t * 2.0 * std::f64::consts::PI / 1.2).sin() * 0.3 + 0.7).clamp(0.4, 1.0);
                        (pulse * 255.0) as u8
                    } else { 255 };
                    let (gr, gg, gb) = if app.dark_mode { (0, 255, 0) } else { (0, 160, 0) };
                    Color32::from_rgba_unmultiplied(gr, gg, gb, alpha)
                } else {
                    app.theme.status_grey
                };
                ui.add(egui::Label::new(RichText::new("●").color(color).size(14.0)));
                ui.label(if app.mcp_enabled { "Running" } else { "Stopped" });
                if let Some(url) = app.mcp_connection_url() {
                    if ui.button("Copy").on_hover_text("Copy address to clipboard").clicked() {
                        ui.ctx().copy_text(url);
                    }
                }
            }).response.on_hover_text(tooltip);

            // Start / Stop buttons
            if app.mcp_enabled {
                // Start is disabled while a server is already running.
                ui.horizontal(|ui| {
                    ui.add_enabled(false, egui::Button::new("Start MCP Server"));
                    ui.label(RichText::new("MCP already running").small().color(app.theme.text_muted));
                });
                if ui.button("Stop MCP Server").clicked() {
                    app.stop_mcp();
                    app.show_toast("MCP server stopped".to_string());
                    app.show_settings_popup = false;
                }
            } else {
                let is_disabled = app.tabs.is_empty();
                let start_resp = ui.add_enabled(!is_disabled, egui::Button::new("Start MCP Server"));
                if is_disabled {
                    start_resp.clone().on_hover_text("Open a log file first");
                }
                if start_resp.clicked() {
                    app.start_mcp();
                    if app.mcp_enabled {
                        if let Some(instruction) = app.mcp_instruction() {
                            ui.ctx().copy_text(instruction);
                        }
                        app.show_toast("MCP started. MCP connection instruction copied to clipboard".to_string());
                    }
                    app.show_settings_popup = false;
                }
            }

            // Prompt AI assistant section
            ui.add_space(4.0);
            if let Some(instruction) = app.mcp_instruction() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Prompt AI assistant to connect with MCP").strong().size(13.0));
                    if ui.button("Copy").on_hover_text("Copy prompt to clipboard").clicked() {
                        ui.ctx().copy_text(instruction);
                    }
                });
                if let Some(url) = app.mcp_connection_url() {
                    ui.add(egui::Label::new(RichText::new(&url).monospace().size(11.0).color(app.theme.url_text)).sense(egui::Sense::click()));
                }
            } else {
                ui.label(RichText::new("Start MCP to connect AI assistant").strong().size(13.0).color(app.theme.text_muted));
            }

            ui.separator();

            // Integrate with AI Assistant button
            if ui.button("Integrate with AI Assistant").clicked() {
                app.show_integrate_popup = true;
            }

            ui.separator();

            // Report Bug — opens the project's issue tracker in the browser.
            if ui.button("Report Bug")
                .on_hover_text("Open https://github.com/muntasir-kabir/logotomy/issues")
                .clicked()
            {
                open_url("https://github.com/muntasir-kabir/logotomy/issues");
            }

            // About — opens the project's GitHub page in the browser.
            if ui.button("About")
                .on_hover_text("Open https://github.com/muntasir-kabir/logotomy")
                .clicked()
            {
                open_url("https://github.com/muntasir-kabir/logotomy");
            }
        });
    });

    // Close on click outside
    if ui.input(|i| i.pointer.any_click()) {
        if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
            let on_button = button_rect.contains(click_pos);
            let on_popup = area_resp.response.rect.contains(click_pos);
            if !on_button && !on_popup {
                app.show_settings_popup = false;
            }
        }
    }
}

/// Show the integrate guide modal window.
pub fn show_integrate_popup(ui: &mut egui::Ui, app: &mut LogotomyApp) {
    if !app.show_integrate_popup {
        return;
    }
    let mut open = app.show_integrate_popup;
    egui::Window::new("Integrate with AI Coding Agents")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([560.0, 480.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new("Connect logotomy to your preferred AI assistant via the MCP server.").small().color(app.theme.text_muted));
            ui.separator();

            let exe_path = std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "/path/to/logotomy".to_string());

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(4.0);
                agent_section(ui, &app.theme, "Claude Desktop",
                    "Edit or create this file:", "~/Library/Application Support/Claude/claude_desktop_config.json",
                    &format!(r#"{{"mcpServers":{{"logotomy":{{"command":"{}","args":["--mcp"]}}}}}}"#, exe_path));
                agent_section(ui, &app.theme, "Claude Code (CLI)", "Run in your terminal:", "",
                    &format!("claude mcp add logotomy {} --mcp", exe_path));
                agent_section(ui, &app.theme, "Cline (VS Code)",
                    "1. Open Cline in VS Code → gear icon → MCP Server → Edit MCP Settings",
                    "2. Add this entry to `mcpServers`:",
                    &format!(r#"{{
  "mcpServers": {{
    "logotomy": {{
      "command": "{}",
      "args": ["--mcp"],
      "disabled": false,
      "autoApprove": []
    }}
  }}
}}"#, exe_path));
                agent_section(ui, &app.theme, "Cursor", "Edit or create this file:", "~/.cursor/mcp.json",
                    &format!(r#"{{"mcpServers":{{"logotomy":{{"command":"{}","args":["--mcp"]}}}}}}"#, exe_path));
                agent_section(ui, &app.theme, "GitHub Copilot Chat (VS Code)", "Edit or create this file in your project:", ".vscode/mcp.json",
                    &format!(r#"{{"servers":{{"logotomy":{{"command":"{}","args":["--mcp"]}}}}}}"#, exe_path));
                agent_section(ui, &app.theme, "OpenAI Codex (CLI)", "Run in your terminal (if MCP is supported):", "",
                    &format!("codex mcp add logotomy {} --mcp", exe_path));
            });
        });
    if !open {
        app.show_integrate_popup = false;
    }
}

fn agent_section(ui: &mut egui::Ui, theme: &Theme, agent: &str, instruction: &str, file_name: &str, code: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(agent).strong().size(13.0));
        if ui.button("Copy").on_hover_text(format!("Copy config for {}", agent)).clicked() {
            ui.ctx().copy_text(code.to_string());
        }
    });
    ui.label(RichText::new(instruction).small().color(theme.text_muted));
    if !file_name.is_empty() {
        ui.label(RichText::new(file_name).monospace().size(10.0).color(theme.url_text));
    }
    let code_lines: Vec<&str> = code.lines().collect();
    if code_lines.len() > 2 {
        let max_display_lines = 10usize.min(code_lines.len());
        let display_text: String = code_lines[..max_display_lines].join("\n");
        // Theme-aware code block background
        let code_bg = if theme.bg.r() > 128 {
            Color32::from_rgb(230, 230, 225) // light mode
        } else {
            Color32::from_rgb(35, 35, 45) // dark mode
        };
        let code_text_color = if theme.bg.r() > 128 {
            Color32::from_rgb(30, 30, 30)
        } else {
            Color32::from_rgb(200, 200, 200)
        };
        egui::Frame::default()
            .fill(code_bg)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&display_text).monospace().size(10.0).color(code_text_color)).sense(egui::Sense::click()));
            });
    } else {
        ui.add(egui::Label::new(RichText::new(code).monospace().size(10.0).color(theme.url_text)).sense(egui::Sense::click()));
    }
    ui.add_space(8.0);
}

/// Open a URL in the system default browser (cross-platform, no extra deps).
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}