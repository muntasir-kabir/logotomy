# AI_ASSISTANT.md — logotomy

High-performance Rust log analyzer & visualizer (GUI + MCP server for AI assistants).

## Strict rules
1. **Big changes** — read `UserGuide.md` + `AI_ASSISTANT_DETAIL.md` first. Update them if structure/features/behavior change. Keep `AI_ASSISTANT_DETAIL.md` concise.
2. **Fixes** — add a 1-2 sentence entry at the top of `changes.md` describing what changed and why.
3. **Tests** — run `cargo test` after code changes; all must pass. Always add test for new feature or bug changes
4. **No Python** — template mining is native Rust (Drain). Never shell out.

## Vital facts
- Multi platform support **MUST**: Windows, MAC (Intel+Apple Silicon), Linux (x86_64), Ubuntu
- **Windows file-lock semantics**: `LockFile` is mandatory (blocks writers), so `load_inner()` skips `try_lock_shared()` on Windows; the mmap itself prevents truncation (`ERROR_USER_MAPPED_FILE`). Tests that shrink files are `#[cfg(not(target_os = "windows"))]`; same-size content changes use in-place writes instead of `std::fs::write`.
- Single binary: `logotomy` (GUI) / `logotomy --mcp` (MCP server, stdio or HTTP).
- `gui` Cargo feature gates GUI deps; Linux/Windows build MCP-only with `--no-default-features`.
- Stack: Rust, eframe/egui, memmap2, memchr, aho-corasick, chrono, crossbeam-channel.
- MCP tools: load_log, list_logs, close_log, filters_get / filters_add / filters_remove (filter keyword sets), find_occurrences (format=refs, context=N, after/before window, first/last-seen), plus the canonical stateless set: summarize_log (range + budget bytes), get_timeline_histogram (range), get_template_anomalies (range), get_template, get_template_samples, log_sequence (dense triples + collapse), raw_log (line/time range), and GUI-only trim. All analysis tools take `with_filtered_log` (default true) to run on the union of the filter set (Everything Else lane excluded); when true with zero filters they reply `{"comment":"no log",…}` until a filter is added or `with_filtered_log:false` is passed.

## Structure (short)
- `src/core/` — library: document (mmap + SIMD index), drain (template mining), masking (pre-mining dynamic value masking), format (log-format detect/normalize), time (timestamp detect), search (Aho-Corasick), timeline, settings
- `src/ui/` — egui GUI modules (each folder = model.rs + view.rs):
  - `app/` — LogotomyApp state + main UI loop
  - `log_view/` — virtualized log rows, selection, pin modal
  - `timeline/` — density histogram, filter lanes, zoom/pan
  - `pin_viewer/` — pinned lines panel
  - `filters/` — filter input strip
  - `settings/` — settings + integrate popups
  - `icons.rs` + `icons/` — SVG icon system · `theme.rs` — dark/light palette
  - `fonts/` — embedded Space Mono monospace font for log text (SIL OFL 1.1, baked into binary)
- `src/mcp.rs` — MCP server (stdio + HTTP) · `src/main.rs` — CLI dispatcher (--mcp → MCP, else GUI)

## Commands
- `cargo test` — 191 unit tests
- `cargo run --release` — GUI
- `cargo run --release --example bench -- [logfile] [filters...]` — benchmark (no args → 64MB/787k-line synthetic log; pass a path to bench a real file, e.g. an iOS log)
- `cargo run --release --example gen_ios_logs -- [SIZES...] [--all] [--seed N]` — generate deterministic iOS test logs (seeded PCG64; iOS-1K/10K/100K/1M; `--all` for all, `--seed N` to override the fixed RNG)
- `cargo run --release --example profile_pipeline -- [logfile]` — per-phase pipeline timing (no args → synthetic ~64MB log; pass a path to profile a real file)

## Docs
- `AI_ASSISTANT_DETAIL.md` — detailed project reference (features, tests, builds)
- `UserGuide.md` — full user guide · `feature.md` — feature inventory · `changes.md` — changelog