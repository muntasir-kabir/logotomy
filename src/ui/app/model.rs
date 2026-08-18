use std::path::PathBuf;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aho_corasick::AhoCorasick;
use crossbeam_channel::Receiver;
use eframe::egui;
use egui::{Color32};
use egui_dock::{DockState};
use log::{error, info};

use logotomy::core::document::{LoadProgress, LoadStage, LogDocument, ParsingConfig};
use logotomy::core::search;
use logotomy::core::settings::Settings;
use logotomy::core::saved_filter::SavedFilter;
use logotomy::core::timeline::{Timeline, DEFAULT_BUCKETS};
use crate::ui::{icons, theme::Theme};



/// Actions triggered from the right-click context menu on timeline or log view.
#[derive(Clone, Copy, Debug)]
pub enum TrimAction {
    TrimRight(usize),
    TrimLeft(usize),
}

/// A saved pin entry — a pinned range of log lines with optional user comment.
#[derive(Clone, Debug)]
pub struct PinEntry {
    pub start_line: usize,
    pub line_numbers: Vec<usize>,
    pub start_ts: i64,
    pub end_ts: i64,
    pub comment: String,
}

/// Maximum number of filters supported in the timeline (excluding
/// the "Everything Else" lane). The filter input is disabled at this cap.
pub const MAX_FILTERS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewTab {
    Timeline,
    Log,
    Pinned,
}

pub struct Filter {
    pub text: String,
    pub color: Color32,
}

pub struct LogTab {
    pub doc: Arc<LogDocument>,
    pub filters: Vec<Filter>,
    /// Per-filter sorted matching line indices.
    pub matches: Vec<Vec<usize>>,
    pub timeline: Timeline,
    /// Line shown in the bottom context panel.
    pub context_line: Option<usize>,
    /// One-shot scroll request for the central log view.
    pub pending_scroll: Option<usize>,
    pub show_templates: bool,
    /// Zoom window on the timeline: (start_x, end_x) in epoch ms or line index.
    /// None = auto (full range).
    pub timeline_zoom: Option<(i64, i64)>,
    /// The filter lane + point index of the currently selected diamond, if any.
    pub selected_diamond: Option<(usize, usize)>,
    pub filter_input: String,
    /// Automaton used for cheap per-line highlight of visible rows.
    pub highlighter: Option<Arc<AhoCorasick>>,
    /// In-flight background filter scan: result channel + cancel flag.
    pub search_rx: Option<(Receiver<Vec<Vec<usize>>>, Arc<AtomicBool>)>,
    /// Per-filter lane active toggle (true = show lane + include in filter).
    pub lane_active: Vec<bool>,
    /// Whether the "Everything Else" lane (lines matching no filter) is active.
    pub everything_else_active: bool,
    /// Filter index pending removal confirmation (None = no pending removal).
    pub pending_filter_removal: Option<usize>,
    /// Shows the "remove ALL filters?" confirmation popup (set by the timeline
    /// "Clear all filters" button, consumed by app/view.rs).
    pub pending_clear_filters: bool,
    /// Whether the timeline is popped out into its own window.
    /// When true, the fixed top panel is hidden and a detached viewport shows.
    pub timeline_detached: bool,
    /// Filtered visible line indices. None = all lines visible.
    pub visible_lines: Option<Vec<usize>>,
    /// Font size for log view and context panel (points).
    pub log_font_size: f32,
    /// Saved pin entries.
    pub pins: Vec<PinEntry>,
    /// Whether the bottom panel is expanded.
    pub bottom_panel_open: bool,
    /// Text input buffer for the pin comment modal.
    pub pin_comment: String,
    /// Range being edited in the pin modal (if any).
    pub pin_modal: Option<(usize, usize)>,
    /// Index into `pins` of the pin currently being edited in the pin modal
    /// (None = the modal is creating a brand-new pin).
    pub pin_edit_index: Option<usize>,
    /// Multi-line drag selection state.
    pub selection_range: Option<(usize, usize)>,
    pub pending_selection: Option<(usize, usize)>,
    pub drag_selecting: bool,
    pub drag_start_line: Option<usize>,
    pub drag_current_line: Option<usize>,
    pub drag_start_pos: Option<egui::Pos2>,
    /// First and last *real* line index visible in the log viewport.
    pub viewport_range: Option<(usize, usize)>,
    /// Real line index to place at the top of the log viewport after a
    /// filter change (set by rebuild_visible_lines, consumed by log_view).
    pub preserve_anchor: Option<usize>,
    pub applied_filter: Option<String>,

    pub find_input: String,
    pub find_query: String,
    pub find_matches: Vec<usize>,
    pub find_pos: Option<usize>,
    pub find_automaton: Option<Arc<AhoCorasick>>,
    pub find_rx: Option<(Receiver<Vec<usize>>, Arc<AtomicBool>)>,
    pub keyword_highlight: Option<String>,
    pub keyword_automaton: Option<Arc<AhoCorasick>>,

    pub dock_state: DockState<ViewTab>,
    pub detached_views: HashSet<ViewTab>,
    pub detached_locations: HashMap<ViewTab, egui_dock::TabPath>,
    pub just_closed_viewports: Vec<ViewTab>,
    pub pending_detach: Option<ViewTab>,
    /// Full dock layout snapshot taken before the first pop-out; restored when
    /// the last detached view returns so the original split layout is preserved.
    pub saved_dock_state: Option<DockState<ViewTab>>,

    /// Whether this tab is currently being served by the MCP server.
    pub mcp_serving: bool,
    /// Whether the file on disk has changed in-place (not appended), making
    /// the document's indexes invalid until a full reload.
    pub stale: bool,
}

impl LogTab {
    pub(crate) fn new(doc: LogDocument) -> Self {
        let doc = Arc::new(doc);
        let timeline = Timeline::build(&doc, &[], DEFAULT_BUCKETS);

        // Set up the default dock layout. Timeline is a fixed top panel
        // (always fully visible), so the dock only contains Log + Pinned.
        let mut dock_state = DockState::new(vec![ViewTab::Log]);
        let [_main_surface, _bottom_surface] = dock_state.main_surface_mut().split_below(egui_dock::NodeIndex::root(), 0.8, vec![ViewTab::Pinned]);

        LogTab {
            doc,
            filters: Vec::new(),
            matches: Vec::new(),
            timeline,
            context_line: None,
            pending_scroll: None,
            show_templates: false,
            timeline_zoom: None,
            selected_diamond: None,
            filter_input: String::new(),
            highlighter: None,
            search_rx: None,
            lane_active: Vec::new(),
            everything_else_active: true,
            pending_filter_removal: None,
            pending_clear_filters: false,
            timeline_detached: false,
            visible_lines: None,
            log_font_size: 12.0,
            pins: Vec::new(),
            bottom_panel_open: false,
            pin_comment: String::new(),
            pin_modal: None,
            pin_edit_index: None,
            selection_range: None,
            pending_selection: None,
            drag_selecting: false,
            drag_start_line: None,
            drag_current_line: None,
            drag_start_pos: None,
            viewport_range: None,
            preserve_anchor: None,
            applied_filter: None,
            find_input: String::new(),
            find_query: String::new(),
            find_matches: Vec::new(),
            find_pos: None,
            find_automaton: None,
            find_rx: None,
            keyword_highlight: None,
            keyword_automaton: None,
            dock_state,
            detached_views: HashSet::new(),
            detached_locations: HashMap::new(),
            just_closed_viewports: Vec::new(),
            pending_detach: None,
            saved_dock_state: None,
            mcp_serving: false,
            stale: false,
        }
    }

    /// Rebuild the filtered visible-lines list based on active lanes.
    /// If all lanes + Everything Else are active, sets visible_lines to None (fast path).
    pub fn rebuild_visible_lines(&mut self) {
        let n = self.doc.total_lines();
        if n == 0 {
            self.visible_lines = None;
            return;
        }
        while self.lane_active.len() < self.filters.len() {
            self.lane_active.push(true);
        }
        self.lane_active.truncate(self.filters.len());

        // Capture the top-visible real line so the log viewport can be
        // preserved across a filter change.
        self.preserve_anchor = self.viewport_range.map(|(first, _)| first);

        let all_active = self.everything_else_active
            && self.lane_active.iter().all(|&a| a);
        if all_active {
            self.visible_lines = None;
            return;
        }

        let mut included = vec![false; n];
        for (ki, active) in self.lane_active.iter().enumerate() {
            if !active { continue; }
            if let Some(matches) = self.matches.get(ki) {
                for &ln in matches {
                    if ln < n { included[ln] = true; }
                }
            }
        }
        if self.everything_else_active {
            let mut matched = vec![false; n];
            for matches in &self.matches {
                for &ln in matches {
                    if ln < n { matched[ln] = true; }
                }
            }
            for (i, inc) in included.iter_mut().enumerate() {
                if !matched[i] { *inc = true; }
            }
        }

        let visible: Vec<usize> = included.iter().enumerate()
            .filter(|(_, &inc)| inc)
            .map(|(i, _)| i)
            .collect();
        self.visible_lines = if visible.len() == n { None } else { Some(visible) };

        // Verify the preserved anchor is still in the new filter. If not,
        // fall back to the nearest still-visible line (or clear it if none).
        if let Some(anchor) = self.preserve_anchor {
            let found = match &self.visible_lines {
                Some(vis) => vis.binary_search(&anchor).is_ok(),
                None => anchor < n,
            };
            if !found {
                match &self.visible_lines {
                    Some(vis) if !vis.is_empty() => {
                        let insertion = vis.binary_search(&anchor).unwrap_or_else(|e| e);
                        // Pick whichever of the two bracketing lines is closest.
                        let nearest = match (insertion.checked_sub(1).map(|i| vis[i]), vis.get(insertion).copied()) {
                            (Some(lower), Some(upper)) => {
                                if anchor - lower <= upper - anchor { lower } else { upper }
                            }
                            (Some(lower), None) => lower,
                            (None, Some(upper)) => upper,
                            (None, None) => anchor,
                        };
                        self.preserve_anchor = Some(nearest);
                    }
                    Some(_) | None => self.preserve_anchor = None,
                }
            }
        }

        if !self.find_query.is_empty() {
            self.start_find(self.find_query.clone());
        }
    }

    /// Remove a filter by index and rescan. If this was the last filter,
    /// "Everything Else" is re-enabled so the view never ends up blank.
    pub fn remove_filter(&mut self, idx: usize) {
        if idx >= self.filters.len() {
            return;
        }
        if self.filters.len() == 1 {
            self.everything_else_active = true;
        }
        self.filters.remove(idx);
        self.rescan_filters();
    }

    /// Clear ALL filters at once and rescan. "Everything Else" is re-enabled
    /// so the view never ends up blank.
    pub fn clear_all_filters(&mut self) {
        if self.filters.is_empty() {
            return;
        }
        self.filters.clear();
        self.everything_else_active = true;
        self.rescan_filters();
    }

    /// Toggle every filter lane on/off as a group. Never touches the
    /// "Everything Else" lane. If the resulting state would hide everything,
    /// "Everything Else" is re-enabled first so the view stays non-blank.
    pub fn toggle_all_lanes(&mut self) {
        if self.filters.is_empty() {
            return;
        }
        while self.lane_active.len() < self.filters.len() {
            self.lane_active.push(true);
        }
        self.lane_active.truncate(self.filters.len());
        let all_active = self.lane_active.iter().all(|&a| a);
        for flag in &mut self.lane_active {
            *flag = !all_active;
        }
        if !self.lane_active.iter().any(|&a| a) && !self.everything_else_active {
            self.everything_else_active = true;
        }
        self.rebuild_visible_lines();
    }

    /// Re-scan the document for the current filter set in the background.
    pub fn rescan_filters(&mut self) {
        if let Some((_, cancel)) = &self.search_rx {
            cancel.store(true, Ordering::Relaxed);
        }
        self.highlighter = search::build_automaton(
            &self.filters.iter().map(|k| k.text.clone()).collect::<Vec<_>>(),
        ).map(Arc::new);

        while self.lane_active.len() < self.filters.len() {
            self.lane_active.push(true);
        }
        self.lane_active.truncate(self.filters.len());

        if self.filters.is_empty() {
            self.matches.clear();
            self.timeline = Timeline::build(&self.doc, &[], DEFAULT_BUCKETS);
            self.search_rx = None;
            self.rebuild_visible_lines();
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let doc = Arc::clone(&self.doc);
        let filters: Vec<String> = self.filters.iter().map(|k| k.text.clone()).collect();
        let cancel_worker = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let _ = tx.send(search::scan_document(&doc, &filters, &cancel_worker));
        });
        self.search_rx = Some((rx, cancel));
    }

    /// Clamp all doc-positioned view state to the current visible window.
    /// When the MCP dirty-doc sync swaps in a mutated (typically trimmed)
    /// document, `context_line`/`viewport_range`/scroll anchors can still hold
    /// line indices from the previous, larger window — feeding those to the
    /// unchecked `ts_at()` would panic (regression: index 1060 vs len 1000).
    /// Bring everything back in range so the next frame is safe to render.
    pub fn clamp_view_state(&mut self) {
        let n = self.doc.total_lines();
        let clamp_pos = |p: &mut Option<usize>| {
            if let Some(v) = *p {
                *p = if n == 0 { None } else { Some(v.min(n - 1)) };
            }
        };
        let clamp_range = |r: &mut Option<(usize, usize)>| {
            if let Some((a, b)) = *r {
                if n == 0 {
                    *r = None;
                } else {
                    *r = Some((a.min(n - 1), b.min(n - 1)));
                }
            }
        };
        clamp_pos(&mut self.context_line);
        clamp_pos(&mut self.pending_scroll);
        clamp_pos(&mut self.preserve_anchor);
        clamp_pos(&mut self.drag_start_line);
        clamp_pos(&mut self.drag_current_line);
        clamp_range(&mut self.viewport_range);
        clamp_range(&mut self.pin_modal);
        clamp_range(&mut self.selection_range);
        clamp_range(&mut self.pending_selection);
    }

    /// Ensure the timeline zoom window includes the current context_line.
    /// If the line is outside the visible range, auto-pan to center on it.
    pub fn ensure_visible(&mut self) {
        let Some(line) = self.context_line else { return };
        let v = match self.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { .. } => {
                // context_line can hold a stale index after an MCP doc swap and
                // exceed the current window; never let the unchecked ts_at index
                // out of bounds (regression: index 1060 vs len 1000 panic).
                match self.doc.ts_at_opt(line) {
                    Some(t) if t >= 0 => t,
                    _ => return,
                }
            }
            logotomy::core::timeline::TimelineDomain::Sequence => line as i64,
        };
        if v < 0 { return; }
        let (full_start, full_end) = match self.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { start_ms, end_ms } => (start_ms, end_ms),
            logotomy::core::timeline::TimelineDomain::Sequence => (1, self.doc.total_lines() as i64),
        };
        let (view_start, view_end) = match self.timeline_zoom {
            Some((s, e)) => (s, e),
            None => (full_start, full_end),
        };
        if v >= view_start && v <= view_end {
            return; // already visible
        }
        // Pan to center on v
        let span = view_end - view_start;
        let half = span / 2;
        let new_start = (v - half).max(full_start);
        let new_end = (new_start + span).min(full_end);
        let new_start = (new_end - span).max(full_start);
        if new_start == full_start && new_end == full_end {
            self.timeline_zoom = None;
        } else {
            self.timeline_zoom = Some((new_start, new_end));
        }
    }

    /// Keep the timeline zoom window in sync with the log viewport. If the visible
    /// range ("window shadow") has scrolled *completely* outside the current
    /// timeline view, recenter the zoom window on the shadow's midpoint while
    /// preserving the current zoom span. A partially-visible shadow is left alone
    /// so an intentionally-zoomed window stays stable while scrolling within it.
    pub fn ensure_viewport_visible(&mut self) {
        let Some((first_line, last_line)) = self.viewport_range else { return };
        let v0 = match self.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { .. } => {
                // viewport_range can hold a stale index after an MCP doc swap and
                // exceed the current window — never let ts_at index out of bounds.
                match self.doc.ts_at_opt(first_line) {
                    Some(t) if t >= 0 => t,
                    _ => return,
                }
            }
            logotomy::core::timeline::TimelineDomain::Sequence => first_line as i64 + 1,
        };
        let v1 = match self.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { .. } => {
                match self.doc.ts_at_opt(last_line) {
                    Some(t) if t >= 0 => t,
                    _ => return,
                }
            }
            logotomy::core::timeline::TimelineDomain::Sequence => last_line as i64 + 1,
        };
        // Match the shadow renderer: only act when the mapped values are valid.
        if v0 < 0 || v1 < 0 { return; }
        let (full_start, full_end) = match self.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { start_ms, end_ms } => (start_ms, end_ms),
            logotomy::core::timeline::TimelineDomain::Sequence => (1, self.doc.total_lines() as i64),
        };
        let (view_start, view_end) = match self.timeline_zoom {
            Some((s, e)) => (s, e),
            None => (full_start, full_end),
        };
        // Only recenter when the shadow lies entirely outside the current view.
        if v1 < view_start || v0 > view_end {
            let span = view_end - view_start;
            let center = v0 + (v1 - v0) / 2;
            let half = span / 2;
            let new_start = (center - half).max(full_start);
            let new_end = (new_start + span).min(full_end);
            let new_start = (new_end - span).max(full_start);
            if new_start == full_start && new_end == full_end {
                self.timeline_zoom = None;
            } else {
                self.timeline_zoom = Some((new_start, new_end));
            }
        }
    }

    /// Apply a trim action: mutates the document in-place, cancels any in-flight
    /// search, and rebuilds filters + timeline for the new trimmed range.
    pub fn handle_trim(&mut self, action: TrimAction) {
        // Cancel any in-flight search.
        if let Some((_, cancel)) = &self.search_rx {
            cancel.store(true, Ordering::Relaxed);
        }
        self.search_rx = None;

        // Apply the trim to the document.
        let doc = Arc::make_mut(&mut self.doc);
        match action {
            TrimAction::TrimRight(l) => doc.trim_right(l),
            TrimAction::TrimLeft(l) => doc.trim_left(l),
        }

        // Reset state that depends on the old line count.
        self.visible_lines = None;
        self.timeline_zoom = None;
        self.context_line = None;
        self.selected_diamond = None;
        self.pending_filter_removal = None;
        self.pending_clear_filters = false;
        self.pins.clear();
        self.bottom_panel_open = false;
        self.pin_modal = None;
        self.pin_edit_index = None;
        self.pin_comment.clear();
        self.selection_range = None;
        self.pending_selection = None;
        self.drag_selecting = false;
        self.drag_start_line = None;
        self.drag_current_line = None;
        self.drag_start_pos = None;
        self.clear_find();
        self.find_input.clear();

        // Rebuild filters + timeline for the new document.
        self.rescan_filters();
    }

    /// Reset the document trim to show all lines.
    pub fn handle_trim_reset(&mut self) {
        if !self.doc.is_trimmed() {
            return;
        }
        // Cancel any in-flight search.
        if let Some((_, cancel)) = &self.search_rx {
            cancel.store(true, Ordering::Relaxed);
        }
        self.search_rx = None;

        let doc = Arc::make_mut(&mut self.doc);
        doc.reset_trim();

        // Reset state.
        self.visible_lines = None;
        self.timeline_zoom = None;
        self.context_line = None;
        self.selected_diamond = None;
        self.pending_filter_removal = None;
        self.pending_clear_filters = false;
        self.pins.clear();
        self.bottom_panel_open = false;
        self.pin_modal = None;
        self.pin_edit_index = None;
        self.pin_comment.clear();
        self.selection_range = None;
        self.pending_selection = None;
        self.drag_selecting = false;
        self.drag_start_line = None;
        self.drag_current_line = None;
        self.drag_start_pos = None;
        self.clear_find();
        self.find_input.clear();

        // Rebuild filters + timeline for the full document.
        self.rescan_filters();
    }

    /// Poll the background filter scan; rebuild the timeline when it lands.
    pub fn poll_search(&mut self) -> bool {
        let Some((rx, _)) = &self.search_rx else { return false; };
        match rx.try_recv() {
            Ok(matches) => {
                self.matches = matches;
                self.timeline = Timeline::build(&self.doc, &self.matches, DEFAULT_BUCKETS);
                self.search_rx = None;
                self.rebuild_visible_lines();
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.search_rx = None;
                false
            }
        }
    }

    /// Start a background find scan for `query`. Empty query clears find state.
    pub fn start_find(&mut self, query: String) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            self.clear_find();
            self.find_input.clear();
            return;
        }
        if let Some((_, cancel)) = &self.find_rx {
            cancel.store(true, Ordering::Relaxed);
        }
        self.find_query = trimmed.to_string();
        self.find_automaton = search::build_find_automaton(trimmed, true).map(Arc::new);
        self.find_matches.clear();
        self.find_pos = None;

        let doc = Arc::clone(&self.doc);
        let subset = self.visible_lines.clone();
        let needle = trimmed.to_string();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_worker = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let _ = tx.send(search::find_lines(&doc, subset.as_deref(), &needle, true, &cancel_worker));
        });
        self.find_rx = Some((rx, cancel));
    }

    /// Poll the background find scan. Returns `true` while in flight.
    pub fn poll_find(&mut self) -> bool {
        let Some((rx, _)) = &self.find_rx else { return false; };
        match rx.try_recv() {
            Ok(matches) => {
                self.find_matches = matches;
                self.find_rx = None;
                if self.find_matches.is_empty() {
                    self.find_pos = None;
                } else {
                    self.find_pos = Some(0);
                    self.goto_find_match(0);
                }
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.find_rx = None;
                false
            }
        }
    }

    /// Step to the next match, wrapping around.
    pub fn find_next(&mut self) {
        if self.find_matches.is_empty() { return; }
        let pos = self.find_pos.unwrap_or(0);
        let next = (pos + 1) % self.find_matches.len();
        self.goto_find_match(next);
    }

    /// Step to the previous match, wrapping around.
    pub fn find_prev(&mut self) {
        if self.find_matches.is_empty() { return; }
        let pos = self.find_pos.unwrap_or(0);
        let prev = if pos == 0 { self.find_matches.len() - 1 } else { pos - 1 };
        self.goto_find_match(prev);
    }

    /// Jump to match index `i` (clamped to match list length).
    fn goto_find_match(&mut self, i: usize) {
        let line = self.find_matches[i.min(self.find_matches.len().saturating_sub(1))];
        self.find_pos = Some(i.min(self.find_matches.len().saturating_sub(1)));
        self.context_line = Some(line);
        self.pending_scroll = Some(line);
        self.ensure_visible();
    }

    /// Clear in-flight find and reset all find state.
    pub fn clear_find(&mut self) {
        if let Some((_, cancel)) = &self.find_rx {
            cancel.store(true, Ordering::Relaxed);
        }
        self.find_rx = None;
        self.find_query.clear();
        self.find_matches.clear();
        self.find_pos = None;
        self.find_automaton = None;
    }

    /// Set the keyword highlight (double-click). Case-insensitive, matching the
    /// find box, so every case variant of the word highlights across the view.
    pub fn set_keyword_highlight(&mut self, kw: Option<String>) {
        self.keyword_highlight = kw.clone();
        self.keyword_automaton = kw
            .map(|k| search::build_find_automaton(&k, true))
            .flatten()
            .map(Arc::new);
    }
}

pub struct FileLoader {
    pub name: String,
    pub rx: Receiver<LoadProgress>,
    pub cancel: Arc<AtomicBool>,
    pub stage: LoadStage,
    pub progress: f32,
}

pub struct LogotomyApp {
    pub tabs: Vec<LogTab>,
    pub active: Option<usize>,
    /// Index into `loaders` of the loading file currently shown (as its own tab).
    /// When `Some`, `active` is `None` (a loading tab has no doc yet).
    pub active_loader: Option<usize>,
    pub loaders: Vec<FileLoader>,
    pub status: String,

    pub theme: Theme,
    pub dark_mode: bool,

    // Persistent settings
    pub settings: Settings,

    // MCP server state
    pub mcp_enabled: bool,
    pub mcp_port: u16,
    pub mcp_state: Option<Arc<Mutex<logotomy::mcp::ServerState>>>,
    pub mcp_thread: Option<std::thread::JoinHandle<()>>,
    pub mcp_shutdown: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Random per-session secret token appended to the MCP server URL path.
    pub mcp_secret: Option<String>,
    pub mcp_started_at: Option<Instant>,
    /// Error message to display in a popup when MCP server fails to start.
    pub mcp_error_popup: Option<String>,

    // Toast notification state
    pub toast_message: Option<String>,
    pub toast_at: Option<Instant>,

    // Integration guide popup (opened from settings)
    pub show_integrate_popup: bool,

    // Recent files popup
    pub recent_show_dropdown: bool,
    pub recent_button_rect: Option<egui::Rect>,

    // SavedFilter management
    pub available_filters: Vec<String>,
    pub show_filter_dropdown: bool,
    pub filter_button_rect: Option<egui::Rect>,
    pub show_new_filter_popup: bool,
    pub new_filter_name: String,
    pub show_rename_filter_popup: bool,
    pub rename_filter_target: String,
    pub rename_filter_new_name: String,

    // Window management
    pub viewport_map: HashMap<egui::ViewportId, (usize, ViewTab)>,

    pub show_settings_popup: bool,
    pub settings_button_rect: Option<egui::Rect>,

    // File update polling
    pub last_file_check: Option<Instant>,
}

/// Render the detected format + date format summary for a document.
fn doc_format_summary(doc: &LogDocument) -> String {
    let date = doc
        .time_format_name()
        .map(|s| s.to_string())
        .or_else(|| doc.time_range.is_some().then(|| "field-based".to_string()))
        .unwrap_or_else(|| "none".to_string());
    format!("format: {} · date: {}", doc.format_name(), date)
}

impl LogotomyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        info!("logotomy GUI starting");
        // Install the embedded Space Mono font for log text immediately, so
        // every viewport renders log lines with it from the first frame.
        crate::ui::fonts::install(&cc.egui_ctx);
        let settings = Settings::load();
        let dark_mode = settings.dark_mode;

        // Ensure filter directory exists
        let filters_dir = Settings::filters_dir();
        if let Err(e) = std::fs::create_dir_all(&filters_dir) {
            error!("failed to create filters directory: {e}");
        }

        let available_filters = Self::load_available_filters();

        info!("settings loaded: dark_mode={dark_mode}, recent_files={}, filters={}",
            settings.recent_files.len(), available_filters.len());

        Self {
            tabs: Vec::new(),
            active: None,
            active_loader: None,
            loaders: Vec::new(),
            status: "Drop a log file anywhere. Go on.".to_string(),
            theme: if dark_mode { Theme::dark() } else { Theme::light() },
            dark_mode,
            settings,
            mcp_enabled: false,
            mcp_port: 0,
            mcp_state: None,
            mcp_thread: None,
            mcp_shutdown: None,
            mcp_secret: None,
            mcp_started_at: None,
            mcp_error_popup: None,
            toast_message: None,
            toast_at: None,
            show_integrate_popup: false,
            recent_show_dropdown: false,
            recent_button_rect: None,
            available_filters,
            show_filter_dropdown: false,
            filter_button_rect: None,
            show_new_filter_popup: false,
            new_filter_name: String::new(),
            show_rename_filter_popup: false,
            rename_filter_target: String::new(),
            rename_filter_new_name: String::new(),
            viewport_map: HashMap::new(),

            show_settings_popup: false,
            settings_button_rect: None,
            last_file_check: Some(Instant::now()),
        }
    }

    fn load_available_filters() -> Vec<String> {
        let filters_dir = Settings::filters_dir();
        let mut filters = Vec::new();
        if let Ok(entries) = std::fs::read_dir(filters_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".json") {
                        filters.push(name.trim_end_matches(".json").to_string());
                    }
                }
            }
        }
        filters.sort();
        filters
    }

    pub fn toggle_theme(&mut self) {
        self.dark_mode = !self.dark_mode;
        self.theme = if self.dark_mode { Theme::dark() } else { Theme::light() };
        self.settings.dark_mode = self.dark_mode;
        self.settings.save();
        // Re-color filter lanes for the new theme so already-open filters
        // immediately pick up the mode-appropriate palette.
        let colors = &self.theme.filter_colors;
        for tab in &mut self.tabs {
            for (i, f) in tab.filters.iter_mut().enumerate() {
                f.color = colors[i % colors.len()];
            }
        }
        // Re-render icons with the new theme color.
        icons::clear_cache();
    }

    /// Detected log format + date format for the selected log (None when no log open).
    pub fn selected_log_format_status(&self) -> Option<String> {
        let idx = self.active?;
        Some(doc_format_summary(&self.tabs[idx].doc))
    }

    pub fn open_file(&mut self, path: PathBuf) {
        if let Some(i) = self.tabs.iter().position(|t| t.doc.path == path) {
            self.active = Some(i);
            self.status = "That file is already open. Nice try though.".to_string();
            info!("file already open: {}", path.display());
            return;
        }
        let name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        info!("opening file: {} ({})", path.display(), name);
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_worker = Arc::clone(&cancel);
        let path_for_load = path.clone();
        let parsing = ParsingConfig {
            sim_threshold: self.settings.sim_threshold,
            header_sample_lines: self.settings.header_sample_lines,
            drain_depth: self.settings.drain_depth,
        };
        std::thread::spawn(move || LogDocument::load_with_config(&path_for_load, parsing, tx, cancel_worker));
        self.loaders.push(FileLoader { name, rx, cancel, stage: LoadStage::Indexing, progress: 0.0 });
        // The loading file is shown in its own (new) log tab, so focus it.
        self.active_loader = Some(self.loaders.len() - 1);
        self.active = None;
        // Track in recent files
        self.settings.add_recent_file(path);
        self.settings.save();
    }

    pub fn poll_file_updates(&mut self) {
        if self.last_file_check.map_or(true, |t| t.elapsed() > Duration::from_secs(2)) {
            self.check_for_file_updates();
            self.last_file_check = Some(Instant::now());
        }
    }

    pub fn check_for_file_updates(&mut self) {
        if let Some(active_idx) = self.active {
            if let Some(tab) = self.tabs.get_mut(active_idx) {
                let old_line_count = tab.doc.total_lines();
                let doc = Arc::make_mut(&mut tab.doc);
                match doc.append_new_data() {
                    Ok(true) => {
                        let new_lines = doc.total_lines() - old_line_count;
                        let file_name = doc.file_name.clone();
                        self.status = format!("Loaded {new_lines} new lines from '{file_name}'.");
                        let _ = doc;
                        tab.stale = false;
                        tab.rescan_filters();
                        log::info!("File {} was appended. Loaded {} new lines.", file_name, new_lines);
                    }
                    Ok(false) => { /* No change, do nothing. */ }
                    Err(e) => {
                        // This now handles both shrinking and in-place modification.
                        log::warn!("File {} changed on disk and requires a full reload: {}", doc.file_name, e);
                        self.status = format!("'{}' changed on disk — only tailing is supported. Close and reopen the file.", doc.file_name);
                        let _ = doc;
                        tab.stale = true;
                    }
                }
            }
        }
    }

    pub fn poll_loaders(&mut self) {
        let mut i = 0;
        while i < self.loaders.len() {
            let mut remove = false;
            match self.loaders[i].rx.try_recv() {
                Ok(LoadProgress::Progress { stage, done, total }) => {
                    self.loaders[i].stage = stage;
                    self.loaders[i].progress = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
                }
                Ok(LoadProgress::Done(doc)) => {
                    let n = doc.total_lines();
                    let mb = doc.file_size as f64 / 1e6;
                    info!("loaded {} — {} lines, {:.1} MB, {} templates", self.loaders[i].name, n, mb, doc.templates.len());
                    self.status = format!(
                        "Loaded `{}` — `{}` lines, {:.1} MB, {} templates.",
                        self.loaders[i].name, n, mb, doc.templates.len()
                    );
                    let mut new_tab = LogTab::new(*doc);
                    if let Some(filter_name) = self.settings.default_filter.clone() {
                        apply_filter_to_tab(&mut new_tab, &filter_name, &self.theme);
                    }
                    self.tabs.push(new_tab);

                    // If MCP is running, share the new file with the MCP server
                    if let Some(ref mcp_state) = self.mcp_state {
                        if let Ok(mut guard) = mcp_state.lock() {
                            if let Some(idx) = self.tabs.last() {
                                // If no active doc is set yet, set this one
                                if guard.active_doc.is_none() {
                                    guard.set_active_doc(Arc::clone(&idx.doc));
                                    if let Some(active_idx) = self.tabs.len().checked_sub(1) {
                                        self.tabs[active_idx].mcp_serving = true;
                                    }
                                    info!("MCP: set active doc to newly loaded file");
                                }
                            }
                        }
                    }
                    self.active = Some(self.tabs.len() - 1);
                    self.active_loader = None;
                    remove = true;
                }
                Ok(LoadProgress::Error(e)) => {
                    error!("failed to load {}: {e}", self.loaders[i].name);
                    self.status = format!("{}: {e}", self.loaders[i].name);
                    self.active_loader = None;
                    remove = true;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.active_loader = None;
                    remove = true;
                }
            }
            if remove {
                self.adjust_active_loader_on_remove(i);
                self.loaders.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Keep `active_loader` pointing at the correct loader as earlier ones are
    /// removed from `self.loaders` (removal shifts later indices down).
    fn adjust_active_loader_on_remove(&mut self, removed_idx: usize) {
        if let Some(al) = self.active_loader {
            if al == removed_idx {
                self.active_loader = None;
            } else if al > removed_idx {
                self.active_loader = Some(al - 1);
            }
        }
    }

    /// Poll the MCP server's dirty flag. If the active doc was modified by an
    /// MCP tool call, re-read it and refresh the UI.
    pub fn poll_mcp_dirty(&mut self) {
        let Some(ref mcp_state) = self.mcp_state else { return };
        let Some(active_idx) = self.active else { return };

        let dirty = {
            let guard = mcp_state.lock().unwrap();
            guard.active_doc_dirty.load(Ordering::Relaxed)
        };
        if !dirty {
            return;
        }

        // Reset the dirty flag and refresh the active tab
        {
            let guard = mcp_state.lock().unwrap();
            guard.active_doc_dirty.store(false, Ordering::Relaxed);
        }

        // Re-read the active doc from MCP state and update the tab
        if let Some(tab) = self.tabs.get_mut(active_idx) {
            let guard = mcp_state.lock().unwrap();
            if let Some(ref mcp_doc) = guard.active_doc {
                // Only refresh if the doc pointer changed (was mutated)
                if !Arc::ptr_eq(&tab.doc, mcp_doc) {
                    tab.doc = Arc::clone(mcp_doc);
                    tab.rescan_filters();
                    // The swapped-in doc may be trimmed/smaller than the previous
                    // one; clamp stale view indices so the next frame's ts_at()
                    // lookups can't run out of bounds (see clamp_view_state).
                    tab.clamp_view_state();
                    info!("MCP: refreshed UI from dirty doc");
                }
            }
        }
    }

    /// Push any GUI-originated document mutations (trim / trim-reset /
    /// file-append) back into the MCP server state. GUI mutations use
    /// `Arc::make_mut`, which deep-copies the document whenever the MCP
    /// server holds another reference — leaving the server with a stale
    /// pre-mutation Arc. This swap keeps `ServerState::active_doc` pointing
    /// at the same Arc the GUI is displaying, and `set_active_doc` drops the
    /// stale `_active` match-cache entries as a side effect.
    pub fn sync_mcp_active_doc(&mut self) {
        let Some(ref mcp_state) = self.mcp_state else { return };
        for tab in &self.tabs {
            if !tab.mcp_serving { continue; }
            let mut guard = mcp_state.lock().unwrap();
            let stale = guard.active_doc.as_ref().map_or(true, |d| !Arc::ptr_eq(d, &tab.doc));
            if stale {
                guard.set_active_doc(Arc::clone(&tab.doc));
                // GUI-originated change — the GUI already holds the newest
                // doc, so don't trigger the MCP→GUI refresh cycle.
                guard.active_doc_dirty.store(false, Ordering::Relaxed);
                info!("MCP: synced GUI-mutated doc into server state");
            }
        }
    }

    /// Push any GUI-originated filter-set changes (toolbar add/remove, saved
    /// filter apply, lane edits) into the MCP server's `_active` filter list.
    /// Only the filter texts are synced — the "Everything Else" lane and lane
    /// toggles are GUI-only and never flow into MCP arithmetic.
    pub fn sync_mcp_filters(&mut self) {
        let Some(ref mcp_state) = self.mcp_state else { return };
        for tab in &self.tabs {
            if !tab.mcp_serving { continue; }
            let texts: Vec<String> = tab.filters.iter().map(|f| f.text.clone()).collect();
            let mut guard = mcp_state.lock().unwrap();
            if guard.get_filters("_active") != texts {
                guard.set_filters("_active", texts);
                // GUI-originated change — the GUI already shows the newest
                // filter set, so don't trigger the MCP→GUI re-apply.
                guard.filters_dirty.store(false, Ordering::Relaxed);
                info!("MCP: synced GUI filters into server state");
            }
        }
    }

    /// Poll the MCP server's `filters_dirty` flag. When the filter set was
    /// modified by an MCP tool call (filters_add/filters_remove), re-apply it
    /// to the served tab so the GUI lanes match what the agent set.
    pub fn poll_mcp_filters(&mut self) {
        let Some(ref mcp_state) = self.mcp_state else { return };
        let dirty = mcp_state.lock().unwrap().filters_dirty.load(Ordering::Relaxed);
        if !dirty {
            return;
        }
        let filters: Vec<String> = {
            let guard = mcp_state.lock().unwrap();
            guard.filters_dirty.store(false, Ordering::Relaxed);
            guard.get_filters("_active")
        };
        if let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
            tab.filters.clear();
            let colors = &self.theme.filter_colors;
            for (i, text) in filters.into_iter().take(MAX_FILTERS).enumerate() {
                tab.filters.push(Filter {
                    text,
                    color: colors[i % colors.len()],
                });
            }
            tab.rescan_filters();
            info!("MCP: re-applied filters from server state");
        }
    }

    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() { return; }
        info!("closing tab {} ({})", idx, self.tabs[idx].doc.file_name);
        if let Some((_, cancel)) = &self.tabs[idx].search_rx {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some((_, cancel)) = &self.tabs[idx].find_rx {
            cancel.store(true, Ordering::Relaxed);
        }

        // If the closed tab was being served by MCP, update MCP state
        let was_serving = self.tabs[idx].mcp_serving;
        self.tabs.remove(idx);

        if was_serving {
            if let Some(ref mcp_state) = self.mcp_state {
                let mut guard = mcp_state.lock().unwrap();
                // Try to serve another tab
                let new_active = if self.tabs.is_empty() {
                    None
                } else {
                    let new_idx = idx.min(self.tabs.len() - 1);
                    self.tabs[new_idx].mcp_serving = true;
                    Some(Arc::clone(&self.tabs[new_idx].doc))
                };
                match new_active {
                    Some(doc) => {
                        guard.set_active_doc(doc);
                        // Seed `_active` filters from the newly served tab
                        // (GUI-originated → clear the MCP→GUI dirty flag).
                        let new_idx = idx.min(self.tabs.len() - 1);
                        let texts: Vec<String> = self.tabs[new_idx]
                            .filters
                            .iter()
                            .map(|f| f.text.clone())
                            .collect();
                        guard.set_filters("_active", texts);
                        guard.filters_dirty.store(false, Ordering::Relaxed);
                        info!("MCP: switched active doc to another tab");
                    }
                    None => {
                        guard.clear_active_doc();
                        info!("MCP: no more tabs, cleared active doc");
                    }
                }
            }
        }

        self.active = if self.tabs.is_empty() { None } else { Some(idx.min(self.tabs.len() - 1)) };
    }

    /// The full MCP server URL for the current session, including the dynamic
    /// port and the per-session secret token (when running).
    pub fn mcp_connection_url(&self) -> Option<String> {
        if !self.mcp_enabled {
            return None;
        }
        let mut url = format!("http://127.0.0.1:{}", self.mcp_port);
        if let Some(secret) = &self.mcp_secret {
            if !secret.is_empty() {
                url.push('/');
                url.push_str(secret);
            }
        }
        Some(url)
    }

    /// A ready-to-paste instruction for a coding agent, pointing it at the
    /// running MCP server and the active log file.
    pub fn mcp_instruction(&self) -> Option<String> {
        let url = self.mcp_connection_url()?;
        let log_path = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.doc.path.display().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Some(format!("Connect to MCP server {url} to analyze {log_path} with user."))
    }

    /// Show a self-dismissing toast notification for a few seconds.
    pub fn show_toast(&mut self, message: String) {
        self.toast_message = Some(message);
        self.toast_at = Some(Instant::now());
    }

    pub fn start_mcp(&mut self) {
        if self.mcp_enabled { return; }
        if self.tabs.is_empty() {
            self.status = "Open a log file first before starting MCP server.".to_string();
            info!("MCP: cannot start — no tabs open");
            return;
        }
        // Dynamic port (OS-assigned) + a fresh random secret token per session.
        let secret: String = {
            use rand::Rng;
            format!("{:06}", rand::thread_rng().gen_range(100_000..=999_999))
        };
        info!("starting MCP server on a dynamic port with secret token {secret}");

        let state = Arc::new(Mutex::new(logotomy::mcp::ServerState::default()));
        {
            let mut guard = state.lock().unwrap();
            // Use the active tab's document directly (no disk reload)
            if let Some(active_idx) = self.active {
                let doc = Arc::clone(&self.tabs[active_idx].doc);
                guard.set_active_doc(doc);
                // Seed the server's filter set from the served tab's live
                // filters so `with_filtered_log=true` (the default) starts out
                // matching what the user is viewing. GUI-originated, so clear
                // the MCP→GUI dirty flag.
                let texts: Vec<String> = self.tabs[active_idx].filters.iter().map(|f| f.text.clone()).collect();
                guard.set_filters("_active", texts);
                guard.filters_dirty.store(false, Ordering::Relaxed);
                self.tabs[active_idx].mcp_serving = true;
                info!("MCP: set active doc from tab {}", active_idx);
            }
        }

        let port = 0u16; // 0 = OS-assigned dynamic port
        let state_clone = Arc::clone(&state);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        // Channel to receive the bind result from the server thread
        let (bind_tx, bind_rx) = std::sync::mpsc::channel();

        let secret_for_thread = secret.clone();
        let thread = std::thread::Builder::new()
            .name("mcp-server".to_string())
            .spawn(move || {
                let _ = logotomy::mcp::run_http(port, state_clone, shutdown_clone, Some(bind_tx), Some(secret_for_thread));
            });

        match thread {
            Ok(handle) => {
                // Wait briefly for the bind result (100ms timeout).
                // run_http reports the bind result immediately after binding,
                // so a timeout means the server thread failed to start at all.
                let bind_result = bind_rx.recv_timeout(Duration::from_millis(100));
                match bind_result {
                    Ok(Ok(actual_port)) => {
                        self.mcp_thread = Some(handle);
                        self.mcp_state = Some(state);
                        self.mcp_shutdown = Some(shutdown);
                        self.mcp_port = actual_port;
                        self.mcp_secret = Some(secret);
                        self.mcp_enabled = true;
                        self.mcp_started_at = Some(Instant::now());
                        self.status = format!("MCP server ready on port {} — serving '{}'", actual_port, self.tabs[self.active.unwrap()].doc.file_name);
                        info!("MCP server started on port {} (secret {})", actual_port, self.mcp_secret.as_deref().unwrap_or(""));
                    }
                    Ok(Err(e)) => {
                        // Bind failed — clean up and show popup
                        error!("MCP: failed to bind: {e}");
                        self.status = format!("MCP: {e}");
                        self.mcp_error_popup = Some(format!("MCP server failed to start:\n\n{e}"));
                        // Clear serving flag
                        if let Some(active_idx) = self.active {
                            if active_idx < self.tabs.len() {
                                self.tabs[active_idx].mcp_serving = false;
                            }
                        }
                    }
                    Err(_) => {
                        // Timeout — the server thread never reported a bind result.
                        // Report an error instead of assuming success.
                        error!("MCP: timed out waiting for server thread to bind");
                        self.status = "MCP: timed out waiting for server to bind".to_string();
                        self.mcp_error_popup = Some("MCP server failed to start:\n\nTimed out waiting for the server thread to bind.".to_string());
                        // Clear serving flag
                        if let Some(active_idx) = self.active {
                            if active_idx < self.tabs.len() {
                                self.tabs[active_idx].mcp_serving = false;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("MCP: failed to start thread: {e}");
                self.status = format!("MCP: failed to start: {e}");
            }
        }
    }

    pub fn stop_mcp(&mut self) {
        info!("stopping MCP server");
        // Signal the server thread to shut down
        if let Some(shutdown) = self.mcp_shutdown.take() {
            shutdown.store(true, Ordering::Relaxed);
        }
        // Wait for the server thread to finish (with a timeout)
        if let Some(handle) = self.mcp_thread.take() {
            if handle.thread().id() != std::thread::current().id() {
                let _ = handle.join();
            }
        }
        // Clear mcp_serving flags
        for tab in &mut self.tabs {
            tab.mcp_serving = false;
        }
        self.mcp_enabled = false;
        self.mcp_port = 0;
        self.mcp_secret = None;
        self.mcp_state = None;
        self.mcp_shutdown = None;
        self.mcp_started_at = None;
        self.status = "MCP server stopped".to_string();
        info!("MCP server shut down");
    }

    /// Called when the active tab changes. If MCP is running, update the
    /// active doc in the server state.
    pub fn on_tab_switched(&mut self, old_idx: Option<usize>, new_idx: usize) {
        if !self.mcp_enabled {
            return;
        }
        if let Some(ref mcp_state) = self.mcp_state {
            let mut guard = mcp_state.lock().unwrap();
            // Clear old serving flag
            if let Some(old) = old_idx {
                if old < self.tabs.len() {
                    self.tabs[old].mcp_serving = false;
                }
            }
            // Set new serving flag
            if new_idx < self.tabs.len() {
                self.tabs[new_idx].mcp_serving = true;
                guard.set_active_doc(Arc::clone(&self.tabs[new_idx].doc));
                // Seed `_active` filters from the newly served tab
                // (GUI-originated → clear the MCP→GUI dirty flag).
                let texts: Vec<String> = self.tabs[new_idx].filters.iter().map(|f| f.text.clone()).collect();
                guard.set_filters("_active", texts);
                guard.filters_dirty.store(false, Ordering::Relaxed);
                info!("MCP: switched active doc to tab {} ({})", new_idx, self.tabs[new_idx].doc.file_name);
            }
        }
    }

    pub fn apply_filter(&mut self, filter_name: &str) {
        if let Some(active_tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
            apply_filter_to_tab(active_tab, filter_name, &self.theme);
        }
    }

    pub fn save_filter(&mut self, filter_name: &str) {
        if filter_name.is_empty() { return; }
        let Some(active_tab) = self.active.and_then(|i| self.tabs.get(i)) else { return };
        let filters: Vec<String> = active_tab.filters.iter().map(|k| k.text.clone()).collect();
        let filter = SavedFilter { filters };
        let filter_path = Settings::filters_dir().join(format!("{filter_name}.json"));

        match serde_json::to_string_pretty(&filter) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&filter_path, &text) {
                    error!("failed to write filter '{}': {e}", filter_path.display());
                } else {
                    info!("saved filter '{}'", filter_path.display());
                    self.available_filters = Self::load_available_filters();
                }
            }
            Err(e) => error!("failed to serialize filter '{filter_name}': {e}"),
        }
    }

    pub fn rename_filter(&mut self, old_name: &str, new_name: &str) {
        if old_name == new_name || new_name.is_empty() { return; }
        let old_path = Settings::filters_dir().join(format!("{old_name}.json"));
        let new_path = Settings::filters_dir().join(format!("{new_name}.json"));
        if new_path.exists() {
            error!("filter already exists: {}", new_path.display());
            return;
        }
        if let Err(e) = std::fs::rename(&old_path, &new_path) {
            error!("failed to rename filter '{old_name}' to '{new_name}': {e}");
        } else {
            info!("renamed filter '{old_name}' to '{new_name}'");
            self.available_filters = Self::load_available_filters();
            for tab in &mut self.tabs {
                if tab.applied_filter.as_deref() == Some(old_name) {
                    tab.applied_filter = Some(new_name.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(content: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("logotomy_app_model_test_{}_{}.log", std::process::id(), n));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn doc_format_summary_reports_format_and_date() {
        // JSON: field-based timestamp (no positional date format).
        let json_path = write_temp("{\"time\": \"2026-08-15T19:40:01Z\", \"lvl\": 30, \"msg\": \"Page load\"}\n");
        let json_doc = LogDocument::open(&json_path).unwrap();
        assert_eq!(
            doc_format_summary(&json_doc),
            "format: json · date: field-based"
        );

        // Plain ISO text.
        let plain_path = write_temp("2026-08-15 19:40:30.123456+0300 INFO hello\n");
        let plain_doc = LogDocument::open(&plain_path).unwrap();
        assert_eq!(
            doc_format_summary(&plain_doc),
            "format: plain · date: ISO-8601"
        );

        // CEF: timeless.
        let cef_path = write_temp("CEF:0|Vendor|Product|1.0|100|Name|3|spt=443\n");
        let cef_doc = LogDocument::open(&cef_path).unwrap();
        assert_eq!(
            doc_format_summary(&cef_doc),
            "format: cef · date: none"
        );

        std::fs::remove_file(json_path).ok();
        std::fs::remove_file(plain_path).ok();
        std::fs::remove_file(cef_path).ok();
    }

    #[test]
    fn remove_filter_removes_and_rescans() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z INFO alpha\n\
             2026-07-19T10:00:01.000Z WARN beta\n\
             2026-07-19T10:00:02.000Z INFO alpha again\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        // Add two filters.
        tab.filters.push(Filter { text: "alpha".into(), color: Theme::light().filter_colors[0] });
        tab.filters.push(Filter { text: "beta".into(), color: Theme::light().filter_colors[1] });
        tab.rescan_filters();
        // Poll until the background scan lands.
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(tab.filters.len(), 2);
        assert_eq!(tab.matches.len(), 2);
        assert_eq!(tab.matches[0].len(), 2); // alpha matches 2 lines
        assert_eq!(tab.matches[1].len(), 1); // beta matches 1 line

        // Remove the first filter.
        tab.remove_filter(0);
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(tab.filters.len(), 1);
        assert_eq!(tab.filters[0].text, "beta");
        assert_eq!(tab.matches.len(), 1);
        assert_eq!(tab.matches[0].len(), 1);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn remove_last_filter_reenables_everything_else() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z INFO alpha\n\
             2026-07-19T10:00:01.000Z WARN beta\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        tab.filters.push(Filter { text: "alpha".into(), color: Theme::light().filter_colors[0] });
        tab.rescan_filters();
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Turn off everything else so the view would be blank after removal.
        tab.everything_else_active = false;
        tab.rebuild_visible_lines();

        tab.remove_filter(0);
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(tab.filters.is_empty());
        // Everything Else must be re-enabled so the view isn't blank.
        assert!(tab.everything_else_active);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn toggle_all_lanes_toggles_every_lane_but_keeps_everything_else() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z INFO alpha\n\
             2026-07-19T10:00:01.000Z WARN beta\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        tab.filters.push(Filter { text: "alpha".into(), color: Theme::light().filter_colors[0] });
        tab.filters.push(Filter { text: "beta".into(), color: Theme::light().filter_colors[1] });
        tab.rescan_filters();
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(tab.lane_active.iter().all(|&a| a), "new filters start visible");
        let ee_before = tab.everything_else_active;

        tab.toggle_all_lanes();
        assert!(tab.lane_active.iter().all(|&a| !a), "toggle-all hides every lane");
        assert_eq!(tab.everything_else_active, ee_before, "Everything Else is never toggled");

        tab.toggle_all_lanes();
        assert!(tab.lane_active.iter().all(|&a| a), "toggle-all restores every lane");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn toggle_all_lanes_reenables_everything_else_when_view_would_blank() {
        let path = write_temp("2026-07-19T10:00:00.000Z INFO alpha\n");
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        tab.filters.push(Filter { text: "alpha".into(), color: Theme::light().filter_colors[0] });
        tab.rescan_filters();
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        tab.everything_else_active = false;
        tab.rebuild_visible_lines();

        // Hiding the only filter lane would blank the view, so Everything Else
        // must be re-enabled.
        tab.toggle_all_lanes();
        assert!(tab.lane_active.iter().all(|&a| !a));
        assert!(tab.everything_else_active);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn clear_all_filters_removes_everything_and_reenables_everything_else() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z INFO alpha\n\
             2026-07-19T10:00:01.000Z WARN beta\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        tab.filters.push(Filter { text: "alpha".into(), color: Theme::light().filter_colors[0] });
        tab.filters.push(Filter { text: "beta".into(), color: Theme::light().filter_colors[1] });
        tab.rescan_filters();
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        tab.everything_else_active = false;
        tab.rebuild_visible_lines();

        tab.clear_all_filters();
        for _ in 0..100 {
            if !tab.poll_search() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(tab.filters.is_empty());
        assert!(tab.matches.is_empty());
        assert!(tab.everything_else_active, "Everything Else must be re-enabled after clearing all filters");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn ensure_viewport_visible_recenters_when_shadow_fully_out_of_view() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z A\n\
             2026-07-19T10:00:10.000Z B\n\
             2026-07-19T10:00:20.000Z C\n\
             2026-07-19T10:00:30.000Z D\n\
             2026-07-19T10:00:40.000Z E\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        // Confirm we're on a time domain and grab its absolute bounds.
        let (full_start, _full_end) = match tab.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { start_ms, end_ms } => (start_ms, end_ms),
            _ => panic!("expected time domain"),
        };

        // Narrowly zoomed to the very start; viewport scrolled to the last two lines,
        // which are entirely outside the zoom window.
        tab.timeline_zoom = Some((full_start, full_start + 10_000));
        tab.viewport_range = Some((3, 4)); // ts(3)=+30000ms, ts(4)=+40000ms

        tab.ensure_viewport_visible();

        let (s, e) = tab.timeline_zoom.expect("zoom should have been recentered");
        // Span is preserved.
        assert_eq!(e - s, 10_000);
        // The shadow midpoint (+35000ms) is centered in the new window.
        assert_eq!(s, full_start + 30_000);
        assert_eq!(e, full_start + 40_000);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn ensure_viewport_visible_keeps_zoom_when_shadow_inside_or_partial() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z A\n\
             2026-07-19T10:00:10.000Z B\n\
             2026-07-19T10:00:20.000Z C\n\
             2026-07-19T10:00:30.000Z D\n\
             2026-07-19T10:00:40.000Z E\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        let (full_start, _full_end) = match tab.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { start_ms, end_ms } => (start_ms, end_ms),
            _ => panic!("expected time domain"),
        };

        // Case 1: shadow fully inside the zoom window -> unchanged.
        tab.timeline_zoom = Some((full_start, full_start + 40_000));
        tab.viewport_range = Some((1, 2)); // +10000..+20000ms
        tab.ensure_viewport_visible();
        assert_eq!(tab.timeline_zoom, Some((full_start, full_start + 40_000)));

        // Case 2: shadow partially overlaps the zoom window (not fully out) -> unchanged.
        tab.timeline_zoom = Some((full_start + 5_000, full_start + 25_000));
        tab.viewport_range = Some((2, 3)); // +20000..+30000ms, pokes past +25000
        tab.ensure_viewport_visible();
        assert_eq!(tab.timeline_zoom, Some((full_start + 5_000, full_start + 25_000)));

        // Case 3: no viewport range -> unchanged (early return).
        tab.viewport_range = None;
        tab.ensure_viewport_visible();
        assert_eq!(tab.timeline_zoom, Some((full_start + 5_000, full_start + 25_000)));

        std::fs::remove_file(path).ok();
    }

    /// Regression: after an MCP dirty-doc swap, `viewport_range` can still hold
    /// line indices that exceed the (smaller) current window. `ensure_viewport_visible`
    /// must not let the unchecked `ts_at()` index out of bounds — it skips instead.
    #[test]
    fn ensure_viewport_visible_ignores_stale_out_of_range_viewport() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z A\n\
             2026-07-19T10:00:10.000Z B\n\
             2026-07-19T10:00:20.000Z C\n\
             2026-07-19T10:00:30.000Z D\n\
             2026-07-19T10:00:40.000Z E\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);
        let (full_start, _full_end) = match tab.timeline.domain {
            logotomy::core::timeline::TimelineDomain::Time { start_ms, end_ms } => (start_ms, end_ms),
            _ => panic!("expected time domain"),
        };
        tab.timeline_zoom = Some((full_start, full_start + 40_000));
        // Stale viewport far beyond the 5-line doc (like after a doc swap).
        tab.viewport_range = Some((100, 100));
        tab.ensure_viewport_visible();
        // No panic; zoom left untouched because the shadow can't be mapped.
        assert_eq!(tab.timeline_zoom, Some((full_start, full_start + 40_000)));
        std::fs::remove_file(path).ok();
    }

    /// Regression: a stale `context_line` beyond the current window must make
    /// `ensure_visible` bail out instead of panicking in `ts_at`.
    #[test]
    fn ensure_visible_ignores_stale_out_of_range_context_line() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z A\n\
             2026-07-19T10:00:10.000Z B\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);
        tab.context_line = Some(500);
        tab.ensure_visible();
        std::fs::remove_file(path).ok();
    }

    /// `clamp_view_state` (run on MCP doc swap) brings every doc-positioned view
    /// field back into the current window, so the next frame can't OOB index.
    #[test]
    fn clamp_view_state_brings_stale_indices_back_in_range() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z A\n\
             2026-07-19T10:00:10.000Z B\n\
             2026-07-19T10:00:20.000Z C\n\
             2026-07-19T10:00:30.000Z D\n\
             2026-07-19T10:00:40.000Z E\n\
             x\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);
        let before = tab.doc.total_lines();

        tab.context_line = Some(1060);
        tab.pending_scroll = Some(2000);
        tab.preserve_anchor = Some(3000);
        tab.drag_start_line = Some(4000);
        tab.drag_current_line = Some(5000);
        tab.viewport_range = Some((999, 1999));
        tab.pin_modal = Some((5000, 6000));
        tab.selection_range = Some((100, 200));
        tab.pending_selection = Some((300, 400));

        tab.clamp_view_state();

        for v in [tab.context_line, tab.pending_scroll, tab.preserve_anchor,
                  tab.drag_start_line, tab.drag_current_line].iter().flatten() {
            assert!(*v < before, "position index {v} not clamped to < {before}");
        }
        for (a, b) in [tab.viewport_range, tab.pin_modal,
                       tab.selection_range, tab.pending_selection].iter().flatten() {
            assert!(*a < before && *b < before, "range ({a},{b}) not clamped to < {before}");
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn find_next_and_prev_wrap_around_and_noop_on_empty() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z err a\n\
             2026-07-19T10:00:01.000Z err b\n\
             2026-07-19T10:00:02.000Z err c\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        // No matches: no-op.
        tab.find_matches = vec![];
        tab.find_pos = None;
        tab.find_next();
        tab.find_prev();
        assert!(tab.find_pos.is_none());

        // Two matches: wrap around.
        tab.find_matches = vec![0, 2];
        tab.find_pos = Some(0);
        tab.find_next();
        assert_eq!(tab.find_pos, Some(1));
        tab.find_next();
        assert_eq!(tab.find_pos, Some(0));
        tab.find_prev();
        assert_eq!(tab.find_pos, Some(1));
        tab.find_prev();
        assert_eq!(tab.find_pos, Some(0));

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn set_keyword_highlight_builds_and_clears_automaton() {
        let path = write_temp("2026-07-19T10:00:00.000Z INFO hello\n");
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        tab.set_keyword_highlight(Some("hello".into()));
        assert_eq!(tab.keyword_highlight, Some("hello".into()));
        assert!(tab.keyword_automaton.is_some());

        tab.set_keyword_highlight(None);
        assert!(tab.keyword_highlight.is_none());
        assert!(tab.keyword_automaton.is_none());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn start_find_polls_to_completion_and_respects_subset() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z err a\n\
             2026-07-19T10:00:01.000Z err b\n\
             2026-07-19T10:00:02.000Z err c\n\
             2026-07-19T10:00:03.000Z ok  d\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);
        tab.visible_lines = Some(vec![0, 2, 3]);

        tab.start_find("err".to_string());
        for _ in 0..200 {
            if !tab.poll_find() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(tab.find_matches, vec![0, 2]);
        assert_eq!(tab.find_pos, Some(0));
        assert_eq!(tab.context_line, Some(0));

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn keyword_highlight_is_case_insensitive() {
        let path = write_temp(
            "2026-07-19T10:00:00.000Z INFO Error starting\\n\
             2026-07-19T10:00:01.000Z WARN error retry\\n\
             2026-07-19T10:00:02.000Z INFO ERROR final\\n",
        );
        let doc = LogDocument::open(&path).unwrap();
        let mut tab = LogTab::new(doc);

        // Double-clicking a word must paint every case variant of it across the
        // view, matching the case-insensitive find box (regression: keyword
        // automaton used to be case-sensitive, highlighting only the exact-case
        // occurrence).
        tab.set_keyword_highlight(Some("Error".to_string()));
        let ac = tab.keyword_automaton.expect("keyword automaton must be built");
        assert!(ac.find_iter("Error starting").next().is_some());
        assert!(ac.find_iter("error retry").next().is_some());
        assert!(ac.find_iter("ERROR final").next().is_some());
        assert_eq!(ac.find_iter("nothing here").next(), None);

        std::fs::remove_file(path).ok();
    }
}

/// Apply a saved filter to a given tab.
pub fn apply_filter_to_tab(tab: &mut LogTab, filter_name: &str, theme: &Theme) {
    let filter_path = Settings::filters_dir().join(format!("{filter_name}.json"));
    if !filter_path.exists() {
        error!("filter not found: {}", filter_path.display());
        return;
    }
    match std::fs::read_to_string(&filter_path) {
        Ok(text) => match serde_json::from_str::<SavedFilter>(&text) {
            Ok(filter) => {
                tab.filters.clear();
                for (i, filter_text) in filter.filters.iter().take(MAX_FILTERS).enumerate() {
                    tab.filters.push(Filter {
                        text: filter_text.clone(),
                        color: theme.filter_colors[i % theme.filter_colors.len()],
                    });
                }
                tab.applied_filter = Some(filter_name.to_string());
                tab.rescan_filters();
                info!("applied filter '{filter_name}' to tab '{}'", tab.doc.file_name);
            }
            Err(e) => error!("failed to parse filter '{filter_name}': {e}"),
        },
        Err(e) => error!("failed to read filter '{filter_name}': {e}"),
    }
}