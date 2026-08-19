# logotomy — High-performance log analyzer & visualizer

## What it is
Rust tool that chews through 50MB+ log files, auto-detects the log format + timestamps, mines Drain templates, and shows an interactive timeline + virtualized log view. Also exposes an MCP server for AI assistants.

## Single binary
`logotomy` is a single binary with two modes:
- `logotomy` — GUI mode (eframe/egui, macOS native)
- `logotomy --mcp` — MCP server mode (stdio or HTTP, cross-platform)

The `gui` Cargo feature gates GUI dependencies. Release builds include full features (GUI + MCP) on all platforms.

## Repository structure
```
src/core/         — Library crate (document, drain, format, time, masking, search, timeline)
src/mcp.rs        — MCP server library (stdio + HTTP, shared by GUI and CLI)
src/ui/           — egui GUI (app, log_view, timeline_view, bottom_panel, filters, theme)
src/ui/fonts/     — embedded Space Mono monospace font (log text only, SIL OFL 1.1)
src/main.rs       — CLI dispatcher: --mcp → MCP mode, else → GUI
examples/         — bench.rs (performance benchmark), gen_ios_logs.rs (Rust iOS test log generator), profile_pipeline.rs (per-phase pipeline timing)
features/         — Feature retrospectives
```

## Tech stack
Rust, eframe/egui (GUI), memmap2 (mmap I/O), memchr (SIMD line indexing), aho-corasick / regex (search), chrono (timestamps), crossbeam-channel (background workers), Drain algorithm (template mining), log + env_logger (logging).

## Key features & related files
| Feature | Files |
|---|---|
| Memory-mapped loading + SIMD indexing | `src/core/document.rs` |
| Log format detection & normalization (JSON, CEF, RFC 5424, Apple ULS, logcat brief, OSLog console, plain) | `src/core/format/` |
| Auto timestamp detection (14 built-in families: ISO-8601, YYYY/MM/DD, MM/DD/YYYY, MM-DD-YYYY, DD-MM-YYYY, DD.MM.YYYY, YYYY.MM.DD, BSD syslog, Apache CLF, RFC 2822, epoch, logcat threadtime, glog, ISO-8601 12h AM/PM, + user-defined custom) | `src/core/time/` |
| Drain template mining (native Rust) | `src/core/drain.rs` |
| Pre-mining masking (IPs, UUIDs, paths, JSON, numbers → semantic placeholders) | `src/core/masking.rs` |
| Aho-Corasick multi-filter search | `src/core/search.rs` |
| Single-pattern find box / keyword highlight (`find_lines`, `build_find_automaton`) | `src/core/search.rs` |
| Persistent settings (JSON, ~/.logotomy/settings.json) | `src/core/settings.rs` |
| Custom date recognizers (regex + named groups, verified live, saved to ~/.logotomy/custom_date_format_list.json) | `src/core/time/custom.rs`, `src/core/settings.rs`, `src/ui/custom_date/` |
| Timeline histogram + filter lanes (zoom/pan, full-height, 1px density lines, eye-toggle + trash per lane, "Everything Else" lane, smart axis labels with duration, minimap, fixed-height always-visible panel, hover tooltip with full filter text + match count, bottom "Show/Hide all filters" + "Clear all filters" toolbar) | `src/core/timeline.rs`, `src/ui/timeline/` |
| Virtualized multi-tab log view | `src/ui/log_view/` |
| Log view find state + keyword highlight (`find_input`, `find_query`, `find_matches`, `find_pos`, `find_rx`, `keyword_highlight`, `keyword_automaton`) | `src/ui/app/model.rs` |
| Embedded Space Mono log font — baked into the binary (`include_bytes!`), registered under a dedicated egui family so only log text uses it (SIL OFL 1.1, `OFL.txt` in `src/ui/fonts/Space_Mono/`) | `src/ui/fonts/`, `src/ui/log_view/`, `src/ui/pin_viewer/` |
| Filter bookmarks (compact toolbar add + chip row; delete confirmations with optional "do not ask again") | `src/ui/filters.rs` |
| Pinned lines + analyses bottom panel (per-card Edit reopens the pin window) | `src/ui/bottom_panel.rs` |
| Scroll-position indicator bar + right-click context menu | `src/ui/log_view.rs` |
| Dark/light theme toggle | `src/ui/theme.rs`, `src/ui/app.rs` |
| MCP server (stdio + HTTP JSON-RPC) | `src/mcp.rs` |
| Compression-first MCP tools (summarize_log with budget bytes, get_timeline_histogram, get_template_anomalies, get_template, get_template_samples, log_sequence dense triples + collapse, raw_log line/time range; find_occurrences refs/context + first/last-seen) + filter tools (filters_get/filters_add/filters_remove) and per-tool `with_filtered_log` (default true = run on the filtered log; Everything Else lane excluded; zero filters + true short-circuits with a `{"comment":"no log"}` hint) | `src/mcp.rs` |
| MCP server GUI controls (in-process, top-bar start/stop + copy-instruction, dynamic port + per-session secret token, status, URL copy) | `src/ui/app/` |
| AI assistant integration popup | `src/ui/app.rs` |

## Tests
191 unit tests in `src/core/` (document, drain, format, time, masking, search, timeline), `src/mcp.rs` (tool payloads, dense log_sequence, collapse, histogram, anomalies, samples, trim, range restriction, filters_get/add/remove, with_filtered_log filtered-view semantics + zero-filter "no log" short-circuit), and the UI models; plus `examples/gen_ios_logs.rs` (determinism, prefix property, token scanner). Run with `cargo test`.

## Benchmark
`cargo run --release --example bench -- [logfile] [filters...]` — with no args, generates a 64MB/787k-line synthetic log (load ~3.3s, 3-filter scan ~0.7s, timeline build ~13ms). Pass a path to bench a real file, e.g. `cargo run --release --example bench -- examples/iOS-100K.log ERROR user_id`.

## Test logs
`cargo run --release --example gen_ios_logs -- [SIZES...]` generates deterministic iOS-style app logs. The generator is pure Rust (seeded PCG64), so the same `--seed` produces byte-identical output on every platform — no Python, no cross-version drift. It produces realistic iOS-style logs with level distribution (5% ERROR, 1% FAULT, 10% WARNING, 14% NOTICE, 45% INFO, 25% DEBUG), ~200 token-parametrized message templates, bursty timestamps, multiple PIDs/threads, and multi-frame FAULT stack traces. Outputs `iOS-1K.log` (~1K lines), `iOS-10K.log` (~10K lines), `iOS-100K.log` (~100K lines), `iOS-1M.log` (~1M lines). Every size starts from the same seed, so smaller files are exact prefixes of larger ones. Pass sizes as args (`cargo run --release --example gen_ios_logs -- 100K 1M`), use `--all` for all four, or `--seed N` to override the RNG. `cargo run --release --example profile_pipeline -- [logfile]` times each pipeline phase (slice → utf8 → ts extract → ts strip → mask → drain) on a file, or on a synthetic ~64MB log when no path is given.

## Cross-platform builds
Releases are built automatically via the GitHub Actions workflow (`.github/workflows/release.yml`) when a `v*` tag is pushed. It builds full-featured binaries (GUI + MCP) natively on each platform's own runner (no cross-compilation), for:
- **Windows** x86_64 (`windows-latest`) → **NSIS installer** `logotomy-<version>-setup.exe` (the runnable `.exe` is `logotomy.exe`, icon + version info embedded at build time)
- **Linux** x86_64 (`ubuntu-latest`) → **`.deb` + `.AppImage`** installers
- **macOS** Apple Silicon (`macos-latest`) + Intel → **`.dmg`** installers (+ `.app` bundle)

Installers replace the old raw `tar.gz`/`zip` archives. Packaging is done by [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) using the `[package.metadata.packager]` table in `Cargo.toml`; the app icon (128×128, plus 256/512 and `logotomy.ico`) lives in `assets/icons/`. Manual per-OS "build binary → make installer" steps and output locations: see `docs/release.md` (`scripts/package-release.sh` shells out to the same two commands).

Each release build also runs the full test suite (main app + `gen_ios_logs` example) and a benchmark against a generated `iOS-100K.log` before packaging. CI (`.github/workflows/rust.yml`) runs the same checks on `ubuntu-latest`/`macos-latest`/`windows-latest` for every push/PR to `main`.

### Windows file-lock semantics
Windows `LockFile` is mandatory (blocks all writers), unlike Unix advisory locks. `load_inner()` therefore skips `try_lock_shared()` on Windows — the mmap itself prevents truncation (`ERROR_USER_MAPPED_FILE`). Tests that need to shrink files are gated with `#[cfg(not(target_os = "windows"))]`; same-size content-change tests use in-place writes (`OpenOptions::new().write(true)`) instead of `std::fs::write` (which truncates).
