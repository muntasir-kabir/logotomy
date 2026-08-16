//! logotomy — high-performance log analysis & visualization.
//! 50MB log file? logotomy happened in there — let's find out.
//!
//! Usage:
//!   logotomy              — launch the GUI
//!   logotomy --mcp        — run MCP server (stdio mode)
//!   logotomy --mcp --port 9876          — MCP server (HTTP mode)
//!   logotomy --mcp --status-file /tmp/s — MCP server (HTTP + status file)
//!   logotomy --help       — show this help

#![cfg_attr(
    all(not(debug_assertions), feature = "gui"),
    windows_subsystem = "windows"
)]

use std::sync::atomic::AtomicBool;
use std::io::Write;
use std::sync::{Arc, Mutex};

const USAGE: &str = r#"logotomy — high-performance log analyzer

USAGE:
  logotomy              Launch the GUI
  logotomy --mcp        Run MCP server (stdio mode)
  logotomy --mcp --port <PORT>   Run MCP server (HTTP mode)
  logotomy --help       Show this help message

MCP server options:
  --port <PORT>      Run in HTTP mode on the given port (default: stdio)
  --status-file <P>  Write PORT, READY, LOG status to a file (HTTP mode)
"#;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Check for --help first
    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("{USAGE}");
        return;
    }

    // Check for --mcp flag
    let is_mcp = args.iter().any(|a| a == "--mcp");

    if is_mcp {
        // Parse optional --port and --status-file
        let port = args.iter().position(|a| a == "--port")
            .and_then(|pos| args.get(pos + 1))
            .and_then(|s| s.parse::<u16>().ok());

        let state: Arc<Mutex<logotomy::mcp::ServerState>> = Arc::default();

        match port {
            Some(p) => {
                // Headless mode: create a shutdown flag that is never set (process termination handles cleanup)
                let shutdown = Arc::new(AtomicBool::new(false));
                let _ = logotomy::mcp::run_http(p, state, shutdown, None, None);
            }
            None => {
                logotomy::mcp::run_stdio(state);
            }
        }
    } else {
        // GUI mode — only available when the "gui" feature is enabled
        run_gui();
    }
}

#[cfg(feature = "gui")]
fn run_gui() {
    // Set up file logging in ~/.logotomy/logs/
    let log_dir = logotomy::core::settings::Settings::log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("logotomy.log");

    // Truncate if over 10MB
    if let Ok(metadata) = std::fs::metadata(&log_path) {
        if metadata.len() > 10 * 1024 * 1024 {
            let _ = std::fs::write(&log_path, "");
        }
    }

    // Open log file for appending
    let log_file = std::fs::File::options()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    let file_logger = log_file.map(|f| Arc::new(Mutex::new(f)));

    // Custom logger: writes to both stderr (via env_logger) and the log file
    struct DualLogger {
        file: Option<Arc<Mutex<std::fs::File>>>,
    }

    impl log::Log for DualLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            // Write to stderr
            eprintln!("{} [{}] {} — {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                record.level(),
                record.target(),
                record.args());

            // Write to file
            if let Some(ref file) = self.file {
                if let Ok(mut f) = file.lock() {
                    let _ = writeln!(f, "{} [{}] {}:{} — {}",
                        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                        record.level(),
                        record.file().unwrap_or("<unknown>"),
                        record.line().unwrap_or(0),
                        record.args());
                }
            }
        }

        fn flush(&self) {
            if let Some(ref file) = self.file {
                if let Ok(mut f) = file.lock() {
                    let _ = f.flush();
                }
            }
        }
    }

    let level = log::LevelFilter::Info;
    log::set_boxed_logger(Box::new(DualLogger { file: file_logger }))
        .map(|()| log::set_max_level(level))
        .ok();

    log::info!("logotomy GUI starting — logs also written to {}", log_path.display());

    // Decode the embedded app icon (logotomy_256.png) into RGBA for the native
    // window icon. On Windows and Linux this sets the window/taskbar icon; on
    // macOS the Dock icon is governed by the .icns bundle instead.
    fn app_icon() -> std::sync::Arc<eframe::egui::IconData> {
        let bytes: &[u8] = include_bytes!("ui/icons/logotomy_256.png");
        match image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                std::sync::Arc::new(eframe::egui::IconData {
                    rgba: rgba.into_raw(),
                    width: w,
                    height: h,
                })
            }
            Err(e) => {
                log::warn!("failed to decode app icon: {e}");
                std::sync::Arc::new(eframe::egui::IconData { rgba: Vec::new(), width: 0, height: 0 })
            }
        }
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 620.0])
            .with_title("logotomy — log analyzer")
            .with_icon(app_icon())
            .with_drag_and_drop(true),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "logotomy",
        options,
        Box::new(|cc| Ok(Box::new(ui::app::model::LogotomyApp::new(cc)))),
    );
}

#[cfg(not(feature = "gui"))]
fn run_gui() {
    eprintln!("logotomy: GUI mode is not available in this build.");
    eprintln!("Rebuild with the 'gui' feature enabled, or use 'logotomy --mcp' for the MCP server.");
    eprintln!("{USAGE}");
    std::process::exit(1);
}

/// The GUI module — only compiled when the "gui" feature is enabled.
/// Points to the existing `src/ui/` directory.
#[cfg(feature = "gui")]
mod ui;