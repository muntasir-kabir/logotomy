# Model Context Protocol (MCP) Server

The `logotomy` application includes a built-in Model Context Protocol (MCP) server that allows AI coding assistants and other external tools to interact with loaded log files programmatically. This enables powerful workflows where an AI can load, search, and analyze log data to assist with debugging and root cause analysis.

The server can run in two modes: as part of the main GUI process or as a standalone process via the command line.

## Initiation

The MCP server can be started in two ways:

1.  **GUI Integration**:
    - In the `logotomy` GUI, use the **"Start MCP"** button in the top toolbar (next to Settings) to start the server, or the Settings popup's "Start MCP Server" button.
    - The server binds a **random OS-assigned port** and appends a **random 6-digit secret token** to its URL (`http://127.0.0.1:PORT/SECRET`). The token is regenerated every session and the server rejects requests without the correct `/{SECRET}` path prefix.
    - The server runs in a background thread within the GUI application's process. Any log files currently open in the GUI are automatically shared with the MCP server and become available to clients. Use **"Copy MCP instruction"** to copy a ready-to-paste connection instruction for your coding agent.

2.  **Standalone Command-Line**:
    - The server can be run as a headless process from your terminal:
      ```sh
      logotomy --mcp
      ```
    - This starts the server in `stdio` mode by default. For HTTP mode, you can specify a port:
      ```sh
      logotomy --mcp --port 8080
      ```

## Protocols

The MCP server supports two communication protocols, providing flexibility for different client environments.

### 1. Stdio (Standard I/O)

-   **Transport**: JSON-RPC 2.0 messages are sent over `stdin` and `stdout`. Each message is a single line of JSON, delimited by a newline character (`\n`).
-   **Use Case**: This is ideal for direct integration with command-line tools or scripts that can manage a child process. The client spawns `logotomy --mcp` and communicates with it via its standard streams.
-   **Default Mode**: This is the default mode when running `logotomy --mcp` without any port specification.

### 2. HTTP

-   **Transport**: The server exposes a standard HTTP/1.1 endpoint. Clients send JSON-RPC 2.0 messages as the body of a `POST` request to the `/` or `/messages` endpoint.
-   **CORS**: The server includes permissive CORS headers (`Access-Control-Allow-Origin: *`), allowing web-based clients (like browser extensions or web UIs) to connect.
-   **Endpoints**:
    -   `POST /` or `POST /messages`: The primary endpoint for sending JSON-RPC messages.
    -   `GET /health`: A health check endpoint that returns a JSON object indicating the server is running.
    -   `OPTIONS /`: Handles pre-flight requests for CORS.
    -   `GET /sse`: An endpoint for Server-Sent Events, part of the MCP streamable transport specification.
-   **GUI secret token**: When the server is started from the GUI, every endpoint is prefixed with a per-session random 6-digit secret (`/{SECRET}`, `/{SECRET}/messages`, `/{SECRET}/sse`, `/{SECRET}/health`), and requests missing the token are rejected with `404`.
-   **Use Case**: This is suitable for clients that cannot easily manage child processes, such as VS Code extensions, web applications, or any tool that can make HTTP requests.

## API (Tools)

The server exposes a set of tools that can be called via the JSON-RPC `tools/call` method. These tools provide the core functionality for interacting with log files.

### Core Tools

*   `load_log(path: string)`
    -   **Description**: Loads and indexes a log file from the given absolute path.
    -   **Returns**: A `log_id` for use with other tools and a `stats` object containing metadata (line count, time range, top templates, etc.).

*   `list_logs()`
    -   **Description**: Lists all currently loaded log files and their statistics.
    -   **Returns**: An array of loaded logs, each with its `log_id` and `stats`.

*   `close_log(log_id: string)`
    -   **Description**: Unloads a log file, freeing its associated memory and resources.
    -   **Returns**: The `log_id` of the closed file.

### Search & Query Tools

*   `find_occurrences(log_id, keyword, max_results?, offset?, format?, context?, after?, before?, with_filtered_log?)`
    -   **Description**: Finds log lines containing an exact phrase (case-sensitive) and returns the total match count plus the keyword's first/last-seen timestamps. Optional `after`/`before` restrict to a time window. Supports pagination. `format="refs"` returns only `{line, time, template_id}` anchors (no raw text) so the AI can plan fetches; `context=N` attaches N lines around each hit, collapsed by template.
    -   **Returns**: `{total_matches, first_seen, last_seen, first_seen_epoch_ms, last_seen_epoch_ms, offset, returned, format, lines, context?}`.

*   `raw_log(log_id, start, end, max_lines?, with_filtered_log?)`
    -   **Description**: Retrieves raw log lines over a range bounded by line numbers or timestamps. `start`/`end` accept a 1-based line number (integer) or a time (string).
    -   **Returns**: `{start_line, end_line, total, returned, truncated, lines}`.

### Analysis Tools

*   `get_template(log_id, ids?, with_filtered_log?)`
    -   **Description**: Resolve template ids to `{pattern, count, example_line}`. Omit `ids` to get every template as a dictionary keyed by id.
    -   **Returns**: `{template_count, templates: {"<id>": {…}}}`.

### Analysis Tools (compression-first)

These tools are designed for low-token AI investigation: they are stateless (optional `start`/`end` line/time ranges) and self-describing (large arrays carry `total`/`truncated` fields so the agent can narrow a range instead of a separate size call). Available in both headless mode (with `log_id`) and GUI mode (without). A typical investigation: `summarize_log` → `get_timeline_histogram` → `log_sequence` → `get_template` → `raw_log`.

**Filters (`with_filtered_log`, default `true`).** Every analysis tool below takes an optional trailing `with_filtered_log` flag. When `true` (the default) the tool operates on the **filtered log**: only lines matching the current filter keyword set (the union of the `filters_get` matches). The GUI's "Everything Else" lane is **never** included in MCP arithmetic. When `true` but **no filters are set** there is nothing to work on — the tool short-circuits with `{"comment": "no log", "reason": "…"}` instead of scanning. Pass `with_filtered_log: false` to run against the full log explicitly, or add a filter first via `filters_add`.

*   `summarize_log(log_id, start?, end?, with_filtered_log?)`
    -   **Description**: One-call orientation (~200 tokens) over an optional range.
    -   **Returns**: Stats, time range, top-5 templates, error-ish templates (ERROR/FATAL/Exception/panic), the 3 largest time gaps, the densest 60-second window, plus byte-size budget estimates (`template_size_bytes`, `sequence_estimate_bytes`).

*   `log_sequence(log_id, start?, end?, max_entries?, collapse?, with_filtered_log?)`
    -   **Description**: Dense `[[epoch_ms|null, line, template_id], …]` over a line/time range. Optional `collapse` folds long same-template runs into one `{start_line, end_line, count, template_id, pattern}` entry.
    -   **Returns**: `{start_line, end_line, total_entries, returned, truncated, size_bytes, sequence}`.

*   `get_timeline_histogram(log_id, start?, end?, buckets?, keyword? | template_id?, with_filtered_log?)`
    -   **Description**: Tiny distribution histogram for the whole log (or a range), a keyword, or a template. Time domain (epoch ms) when the log has timestamps, else line-index domain. Use it to find the spike/window *before* fetching any lines.
    -   **Returns**: `{domain, x: [...], counts: [...], total}` parallel arrays.

*   `get_template_anomalies(log_id, start?, end?, limit?, with_filtered_log?)`
    -   **Description**: The "what is unusual in this log" question, nearly free thanks to Drain: rare templates (≤0.1% of lines), first-seen-late templates (new pattern appearing in the last 10% of the range), and bursty templates (most occurrences packed into a tiny window).
    -   **Returns**: Anomalies with `template_id`, pattern, count, reasons, and first-seen anchor.

*   `get_template_samples(log_id, template_id, n?, strategy?, with_filtered_log?)`
    -   **Description**: A few concrete example lines for a template without dumping all matches. `strategy` is `first_last_random` (default, deterministic), `first`, or `even`.
    -   **Returns**: Pattern, total match count, and up to `n` sample lines.

*   `trim(start?, end?)` (GUI mode only)
    -   **Description**: Focus the active document's visible window to a line/time range; omit both bounds to reset to the full file.
    -   **Returns**: `{remaining_lines}`.

### Filter Tools

The log's filter set is a list of case-insensitive keyword terms (max 20). When any filter is set, every analysis tool with `with_filtered_log=true` (the default) restricts its results to the **union of the filters' matches** — the "Everything Else" lane never applies. In GUI mode the filter set is shared with the served tab's live filter lanes (either side's edits propagate to the other). In headless mode filters are server-side per-`log_id`.

*   `filters_get(log_id?)`
    -   **Description**: List the current filter set.
    -   **Returns**: `{filters: [{id, filter_text}], filter_count}` where `id` is the filter's position.

*   `filters_add(log_id?, filter_text: string)` — add a keyword filter.
    -   **Rules**: case-insensitive dedupe (duplicate → error), 20-filter cap, empty text → error.
    -   **Returns**: the full updated `{filters: [{id, filter_text}], filter_count}`.

*   `filters_remove(log_id?, id: number)` — remove a filter by position.
    -   **Rules**: out-of-range `id` → error.
    -   **Returns**: the full updated `{filters: [{id, filter_text}], filter_count}`.

## Pros and Cons

### Pros

1.  **Decoupling**: The MCP server decouples the log analysis engine from the client. AI assistants don't need to have file system access or log parsing logic; they can simply connect to `logotomy`.
2.  **Performance**: The Rust backend is highly optimized for file I/O and searching. It uses memory-mapped files and efficient search algorithms (`aho-corasick`) to handle very large files with low overhead.
3.  **Shared State**: When run from the GUI, the MCP server shares the same in-memory `LogDocument` objects. This means you can visually explore a file in the GUI and have an AI assistant programmatically query the exact same data with no re-indexing cost.
4.  **Flexibility**: Supporting both `stdio` and `HTTP` makes it easy to integrate with a wide variety of clients, from simple shell scripts to complex web-based IDEs.
5.  **Extensibility**: The tool-based API is easy to extend. New analysis functions can be added as new tools in `mcp.rs` without changing the protocol itself.

### Cons

1.  **Stateful Nature**: The server is stateful. The client is responsible for loading a log file and receiving a `log_id`, which it must then use in subsequent calls. If the server is restarted, all state is lost.
2.  **Security**: The HTTP server is bound to `127.0.0.1`, which limits access to the local machine. GUI-started servers additionally require a per-session random 6-digit secret token in the URL path (`http://127.0.0.1:PORT/SECRET`), so only clients that know the URL can connect. Headless `--mcp --port` mode has no token, so any local process can connect to that port — acceptable for a local developer tool but worth noting in multi-user environments.
3.  **Resource Management**: The server holds loaded log files in memory. While efficient, loading many multi-gigabyte files can consume significant RAM. Clients (or the user) must be mindful of closing logs (`close_log`) when they are no longer needed.

---

This documentation should provide a comprehensive overview for anyone looking to integrate with `logotomy`'s MCP server.