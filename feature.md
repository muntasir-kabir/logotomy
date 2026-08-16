# logotomy — Feature Inventory

Everything currently implemented, and what's deliberately not (yet).

## ✅ Ingestion & parsing core (`src/core/`)

| Feature | Detail |
|---|---|
| Any text file | `.log`, `.txt`, `.out`, `.csv`, … — extension-agnostic |
| Memory-mapped I/O | `memmap2`; 50MB+ files never fully materialized |
| SIMD line index | one `memchr` pass builds the line-offset table |
| Real progress reporting | 2 stages (Indexing / Analyzing), byte-accurate %, cancellable |
| Log format detection | Pluggable recognizer: JSON, CEF, RFC 5424, Apple Unified Logging (`log show`), logcat brief, iOS OSLog console — one file per format (`src/core/format/`), registry-extensible, `plain` fallback |
| Timestamp auto-detection | Pluggable: ISO-8601 (`Z`, offsets, comma/dot millis, space/`T` separator), `YYYY/MM/DD`, syslog `Jan  5`, Apache `10/Oct/2024:13:55:36 -0700`, epoch s/ms, logcat threadtime, glog — one file per family (`src/core/time/`) |
| Forward-filled timestamps | stack traces & continuation lines inherit previous timestamp |
| Native Drain template mining | fixed-depth parse tree, `<*>` wildcards, per-line template ID, occurrence counts, example line — zero Python |
| Robustness | CRLF, missing trailing newline, blank lines, invalid UTF-8 (lossy), multi-MB single lines, timeless files |
| Filter engine | Aho-Corasick, case-sensitive, all filters in a single pass, background thread + cancellation |
| Timeline bucketing | 2048-bucket density histogram; time domain or line-sequence fallback; per-filter lanes + match points |
| Timeline lane toggles | Per-filter eye (visible/invisible) toggles + "Everything Else" lane; log view filters to active lanes only; trash icon per lane removes filter with confirmation |
| Smart axis labels | Shorthand: same hour → `MM:SS.ms`, same date → `HH:MM:SS.ms`, multi-day → full; duration label between ticks |
| Time-window analytics | count-in-window, first/last occurrence helpers |

## ✅ Desktop GUI (`src/ui/`, egui/eframe)

| Feature | Detail |
|---|---|
| Drag & drop | drop any file anywhere; "Drop it" overlay while hovering files |
| Open dialog | native picker via `rfd` |
| Progress bar | per-loading-file card with stage label + % + cancel |
| Multi-tab | open/close/switch many files; re-dropping an open file focuses its tab |
| Virtualized log view | renders only visible rows; line # + template ID gutter; filter highlight; color marker per line |
| Log font | embedded Space Mono monospace (SIL OFL 1.1) — log text only, rest of UI stays on default fonts |
| Log font size controls | A− / A+ buttons in both log view and context panel (8–24px range) |
| Lane-filtered log view | toggling timeline checkboxes filters the log view to only active lanes |
| Filter bookmarks | add/remove chips, per-filter color, live match count, background rescan |
| Timeline view | density histogram + per-filter colored lanes, hover tooltip, selection marker |
| Timeline lane toggles | per-filter eye (visible/invisible) toggle + "Everything Else" lane; grays out disabled lanes; trash icon removes filter with confirmation |
| Timeline legend | 120px left column, 10.5pt monospace labels, 14-char truncation with full-name tooltip |
| Smart axis labels | shorthand: same hour → `MM:SS.ms`, same date → `HH:MM:SS.ms`, multi-day → full; duration label between ticks |
| Timeline navigation | click → nearest filter match (or approx. position) |
| Context panel | selected line ± 5 (radius adjustable 1–50), timestamp + template header, jump-to-full-view, clear |
| Template browser | right panel, mined patterns sorted by frequency, click → example line |
| Dark/light mode | semantic colour palette, 🌙/☀️ toggle in toolbar, all panels themed |
| UX details | clickable everything, pointer cursors, drag overlay, status bar, humor |
| Status bar format readout | shows the active log's detected format + date format (`format: json · date: field-based`) |

## ✅ MCP server (`src/mcp.rs`, `logotomy --mcp`)

Embedded in the same binary; runs via `logotomy --mcp` (stdio) or `logotomy --mcp --port <N>` (HTTP).
Newline-delimited JSON-RPC 2.0 over stdio **or HTTP**; initialize / ping / tools/list / tools/call;
multiple protocol versions (`2024-11-05`, `2025-03-26`, `2025-06-18`).
In HTTP mode, writes `PORT <N>`, `READY`, and `LOG <path>` to a status file for GUI monitoring.
Can run **in-process** from the GUI — shares loaded documents with the UI via `Arc<Mutex<ServerState>>`.

| Tool | Purpose |
|---|---|
| `load_log` | index a file → `log_id` + stats |
| `list_logs` | loaded documents + stats |
| `close_log` | drop a document + invalidate its caches |
| `find_occurrences` | keyword hits with offset/max_results pagination, `format="refs"` anchors-only mode, `context=N` collapsed context; returns total count + first/last-seen, optional `after`/`before` window |
| `summarize_log` | one-call orientation over an optional line/time range: stats, error-ish templates, time gaps, densest minute, plus byte-size budget estimates (`template_size_bytes`, `sequence_estimate_bytes`) |
| `get_timeline_histogram` | tiny distribution histogram (whole log / keyword / template), optional range |
| `get_template_anomalies` | rare / first-seen-late / bursty templates, optional range |
| `get_template` | resolve template ids to `{pattern, count, example_line}`; omit `ids` for all |
| `get_template_samples` | a few concrete lines per template |
| `log_sequence` | dense `[[epoch_ms\|null, line, template_id], …]` over a line/time range; optional `collapse`, `truncated` flag |
| `raw_log` | raw lines over a line/time range, `max_lines` + `truncated` flag |
| `trim` (GUI mode) | focus the active document's visible window to a line/time range |

Match results are cached per (log, keyword); time params accept RFC3339 /
`YYYY-MM-DD[ HH:MM[:SS]]` / epoch s/ms. `start`/`end` range bounds accept a
1-based line number (integer) or a time (string).

### GUI MCP Server controls
- **🖧 MCP** dropdown button in the top-right toolbar
- Green/gray status indicator (blinks green on activity within 10s)
- URL display with 📋 Copy button
- Stdio command path with 📋 Copy button (`logotomy --mcp`)
- Configurable port selector (1000-65535)
- Start/Stop toggle button
- Runs **in-process** on a background thread — shares loaded documents with the GUI
- Status bar updates on start/stop/ready/error

## ✅ Logging & observability

| Feature | Detail |
|---|---|
| Application-wide logging | `log` + `env_logger` crate: GUI logs to stderr, MCP server logs to both stderr and `logotomy.log` |
| Configurable via `RUST_LOG` | standard env-var filtering: `RUST_LOG=debug`, `RUST_LOG=warn`, etc. |
| Panic hook | MCP server captures panics with location info to `logotomy.log` |
| Key events logged | file open/close, tab switch, MCP start/stop, filter scan, load errors |

## ✅ Quality & verification

- 53 unit tests (drain, document, timestamp, search, timeline, mcp) — `cargo test`
- Release benchmark harness — `cargo run --release --example bench -- <file> [kws]`
  (generates a 64MB synthetic log when run without args)

## 🔮 Deliberate next steps (not implemented)

- Filter view (show only matching lines) & regex filters
- Timeline zoom/pan with bucket re-resolution
- Export of filtered ranges / templates report
- Persistent filter sets per file
- Multi-file merged timeline
- Resources/prompts in the MCP server (tools only, today)

## 🗑 Removed

- `src/python/parser_ml_microservice.py` — the old half-baked Python sidecar.
  Template mining is native Rust now; nothing is shelled out, ever.