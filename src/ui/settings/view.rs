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
    let Some(button_rect) = app.settings_button_rect else {
        return;
    };

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

            // Custom date recognizers (opened from Settings; was in the top bar)
            ui.horizontal(|ui| {
                let ctx = ui.ctx().clone();
                ui.add(icons::icon_image(&ctx, Icon::Date, 14.0, app.theme.text));
                if ui.button("Custom date recognizers")
                    .on_hover_text("Add / manage user-defined date/time recognizers")
                    .clicked()
                {
                    app.show_custom_date_popup = !app.show_custom_date_popup;
                }
            });

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
                ui.label(if app.mcp_enabled { "Running · GUI mode" } else { "Stopped" });
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
                ui.label(RichText::new("Security: this local URL contains a temporary secret. Anyone who obtains it can query the active log until MCP is stopped.").small().color(app.theme.text_muted));
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
            ui.label(RichText::new("Connect logotomy to an AI assistant via MCP.").small().color(app.theme.text_muted));
            ui.label(RichText::new("These snippets start a separate local stdio server. The server will not inherit the file currently open in the GUI; ask the agent to call load_log with the log path. For the GUI's live document, use the Start MCP button and copy its temporary HTTP URL instead.").small().color(app.theme.text_muted));
            ui.separator();

            let exe_path = std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "/path/to/logotomy".to_string());
            let stdio_json = mcp_stdio_json(&exe_path);
            let cline_json = mcp_cline_json(&exe_path);
            let copilot_json = mcp_copilot_json(&exe_path);
            let codex_toml = mcp_codex_toml(&exe_path);

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(4.0);
                agent_section(ui, &app.theme, "Claude Desktop",
                    "Open Claude Desktop → Settings → Developer → Edit Config, then merge this entry:", claude_desktop_config_path(),
                    &stdio_json);
                agent_section(ui, &app.theme, "Claude Code (CLI)", "Run in your terminal, then verify with `claude mcp list`:", "",
                    &format!("claude mcp add logotomy -- {} --mcp", shell_quote(&exe_path)));
                agent_section(ui, &app.theme, "Cline (VS Code)",
                    "1. Open Cline in VS Code → gear icon → MCP Server → Edit MCP Settings",
                    "2. Add this entry to `mcpServers`:",
                    &cline_json);
                agent_section(ui, &app.theme, "Cursor", "Edit or create this file:", "~/.cursor/mcp.json",
                    &stdio_json);
                agent_section(ui, &app.theme, "GitHub Copilot Chat (VS Code)", "Open `.vscode/mcp.json` (or the user MCP configuration) and add this under `servers`; VS Code requires `type`:", ".vscode/mcp.json or ~/.copilot/mcp-config.json",
                    &copilot_json);
                agent_section(ui, &app.theme, "OpenAI Codex (CLI)", "Add this to `~/.codex/config.toml`, then restart Codex or reload MCP servers:", "~/.codex/config.toml",
                    &codex_toml);
            });
        });
    if !open {
        app.show_integrate_popup = false;
    }
}

/// JSON configuration shared by Claude Desktop and Cursor. Serialize the path
/// instead of interpolating it so spaces, quotes, and Windows separators are
/// escaped correctly.
fn mcp_stdio_json(exe_path: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "logotomy": { "command": exe_path, "args": ["--mcp"] }
        }
    }))
    .unwrap_or_default()
}

fn mcp_cline_json(exe_path: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "logotomy": {
                "command": exe_path,
                "args": ["--mcp"],
                "disabled": false,
                "autoApprove": []
            }
        }
    }))
    .unwrap_or_default()
}

fn mcp_copilot_json(exe_path: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "servers": {
            "logotomy": {
                "type": "stdio",
                "command": exe_path,
                "args": ["--mcp"]
            }
        }
    }))
    .unwrap_or_default()
}

fn mcp_codex_toml(exe_path: &str) -> String {
    // JSON string escaping is compatible with TOML basic strings for paths and
    // gives us a safe quoted command on every supported platform.
    let command = serde_json::to_string(exe_path).unwrap_or_else(|_| "\"logotomy\"".to_string());
    format!(
        "[mcp_servers.logotomy]\ncommand = {command}\nargs = [\"--mcp\"]\nstartup_timeout_ms = 20000"
    )
}

#[cfg(target_os = "macos")]
fn claude_desktop_config_path() -> &'static str {
    "~/Library/Application Support/Claude/claude_desktop_config.json"
}

#[cfg(target_os = "windows")]
fn claude_desktop_config_path() -> &'static str {
    "%APPDATA%\\Claude\\claude_desktop_config.json"
}

#[cfg(target_os = "linux")]
fn claude_desktop_config_path() -> &'static str {
    "~/.config/Claude/claude_desktop_config.json"
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn claude_desktop_config_path() -> &'static str {
    "Claude Desktop → Settings → Developer → Edit Config"
}

/// Quote a local executable for the user's shell when generating a Claude
/// Code command. The JSON-based integrations use serializers above instead.
#[cfg(not(target_os = "windows"))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "windows")]
fn shell_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn agent_section(
    ui: &mut egui::Ui,
    theme: &Theme,
    agent: &str,
    instruction: &str,
    file_name: &str,
    code: &str,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(agent).strong().size(13.0));
        if ui
            .button("Copy")
            .on_hover_text(format!("Copy config for {}", agent))
            .clicked()
        {
            ui.ctx().copy_text(code.to_string());
        }
    });
    ui.label(RichText::new(instruction).small().color(theme.text_muted));
    if !file_name.is_empty() {
        ui.label(
            RichText::new(file_name)
                .monospace()
                .size(10.0)
                .color(theme.url_text),
        );
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
                ui.add(
                    egui::Label::new(
                        RichText::new(&display_text)
                            .monospace()
                            .size(10.0)
                            .color(code_text_color),
                    )
                    .sense(egui::Sense::click()),
                );
            });
    } else {
        ui.add(
            egui::Label::new(
                RichText::new(code)
                    .monospace()
                    .size(10.0)
                    .color(theme.url_text),
            )
            .sense(egui::Sense::click()),
        );
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_json_configs_escape_executable_paths() {
        let path = if cfg!(windows) {
            r#"C:\Program Files\Logotomy\logotomy.exe"#
        } else {
            "/Applications/Logotomy\" nightly/logotomy"
        };
        for config in [
            mcp_stdio_json(path),
            mcp_cline_json(path),
            mcp_copilot_json(path),
        ] {
            let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
            let command = parsed
                .pointer("/mcpServers/logotomy/command")
                .or_else(|| parsed.pointer("/servers/logotomy/command"))
                .and_then(serde_json::Value::as_str);
            assert_eq!(command, Some(path));
        }
    }

    #[test]
    fn generated_agent_configs_use_expected_transport_shapes() {
        let path = "/tmp/logotomy";
        let copilot: serde_json::Value = serde_json::from_str(&mcp_copilot_json(path)).unwrap();
        assert_eq!(copilot["servers"]["logotomy"]["type"], "stdio");
        assert_eq!(copilot["servers"]["logotomy"]["args"][0], "--mcp");

        let codex = mcp_codex_toml(path);
        assert!(codex.contains("[mcp_servers.logotomy]"));
        assert!(codex.contains("command = \"/tmp/logotomy\""));
        assert!(codex.contains("args = [\"--mcp\"]"));
    }
}

/// Open a URL in the system default browser (cross-platform, no extra deps).
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}
