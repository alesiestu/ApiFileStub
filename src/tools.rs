use notify::Watcher;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use tokio::sync::broadcast;

// Route mapping entry stored in config/routes.txt.
#[derive(Clone)]
pub struct RouteMapping {
    pub method: String,
    pub path: String,
    pub file: String,
}

struct LogState {
    sender: broadcast::Sender<String>,
    buffer: Mutex<VecDeque<String>>,
}

static LOG_STATE: OnceLock<LogState> = OnceLock::new();

// Resolve the json/ directory path.
pub fn base_json_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("json")
}

// Resolve the config/ directory path.
pub fn base_config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config")
}

// Load refresh endpoint from config or default.
pub fn read_refresh_endpoint() -> String {
    let path = base_config_dir().join("refresh_endpoint.txt");
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        "/api/v1/authentication/refresh".to_string()
    } else {
        trimmed.to_string()
    }
}

// Load ping endpoint from config or default.
pub fn read_ping_endpoint() -> String {
    let path = base_config_dir().join("ping_endpoint.txt");
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        "/api/v1/ping".to_string()
    } else {
        trimmed.to_string()
    }
}

// Load log ignore patterns with defaults for / and /events.
pub fn read_log_ignore_patterns() -> Vec<String> {
    let mut defaults = vec!["/".to_string(), "/events".to_string()];
    let path = base_config_dir().join("log_ignore.txt");
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let mut from_file: Vec<String> = contents
        .lines()
        .filter_map(|line| normalize_log_pattern(line))
        .collect();
    defaults.append(&mut from_file);
    defaults
}

// Load the global log enabled toggle (default on).
pub fn read_log_enabled() -> bool {
    let path = base_config_dir().join("log_enabled.txt");
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let trimmed = contents.trim().to_lowercase();
    trimmed.is_empty() || trimmed == "on" || trimmed == "true" || trimmed == "1"
}

// Check whether a path matches any ignore pattern.
pub fn is_log_ignored(path: &str) -> bool {
    let patterns = read_log_ignore_patterns();
    for pattern in patterns {
        if pattern.ends_with("/*") {
            let prefix = &pattern[..pattern.len() - 1];
            let base = prefix.trim_end_matches('/');
            if path == base || path == prefix || path.starts_with(prefix) {
                return true;
            }
        } else if path == pattern {
            return true;
        }
    }
    false
}

// Normalize and validate log ignore patterns.
pub fn normalize_log_pattern(input: &str) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("/{}", raw)
    };
    if normalized.contains('\\') {
        return None;
    }
    if normalized.contains('*') && !normalized.ends_with("/*") {
        return None;
    }
    Some(normalized)
}

// Parse a urlencoded form field value by key.
pub fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').find_map(|pair| {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next()?;
        let v = parts.next().unwrap_or_default();
        if k == key {
            Some(url_decode(v))
        } else {
            None
        }
    })
}

// Decode application/x-www-form-urlencoded values.
pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                    out.push(h << 4 | l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// Convert a hex digit to a numeric value.
pub fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// Validate a single path segment to prevent traversal.
pub fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('/')
        && !segment.contains('\\')
}

// Validate a relative path (no traversal or prefixes).
pub fn is_safe_rel_path(path: &str) -> bool {
    let rel = std::path::Path::new(path);
    for component in rel.components() {
        match component {
            std::path::Component::Normal(part) => {
                if part.to_string_lossy().is_empty() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

// Collect file entries and subdir names for the UI.
pub fn collect_json_index(base_dir: PathBuf) -> (Vec<(String, String)>, Vec<String>) {
    let entries = collect_json_entries(base_dir.clone());
    let subdirs = collect_subdirs(base_dir);
    (entries, subdirs)
}

// Walk json/ and list all JSON file paths.
pub fn collect_json_entries(base_dir: PathBuf) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    if !base_dir.is_dir() {
        return entries;
    }

    for entry in walkdir::WalkDir::new(&base_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel_path = match entry.path().strip_prefix(&base_dir) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");
        if !is_safe_rel_path(&rel_path_str) {
            continue;
        }
        let url = format!("/json/{}", rel_path_str);
        entries.push((rel_path_str, url));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

// List immediate subdirectories under json/.
pub fn collect_subdirs(base_dir: PathBuf) -> Vec<String> {
    let mut subdirs = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(base_dir) else {
        return subdirs;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if is_safe_segment(name) {
                    subdirs.push(name.to_string());
                }
            }
        }
    }
    subdirs.sort();
    subdirs
}

// List files inside a specific json subdirectory.
pub fn collect_subdir_entries(base_dir: PathBuf, subdir: String) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(base_dir) else {
        return entries;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if is_safe_segment(name) {
                    let rel_path = format!("{}/{}", subdir, name);
                    let url = format!("/json/{}", rel_path);
                    entries.push((rel_path, url));
                }
            }
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

// Load route mappings from config file.
pub fn read_route_mappings() -> Vec<RouteMapping> {
    let path = base_config_dir().join("routes.txt");
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let mut mappings = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let method = match parts.next() {
            Some(m) => m.to_uppercase(),
            None => continue,
        };
        if method != "GET" && method != "POST" {
            continue;
        }
        let path = match parts.next() {
            Some(p) => p.to_string(),
            None => continue,
        };
        let file = match parts.next() {
            Some(f) => f.to_string(),
            None => continue,
        };
        if !path.starts_with("/api/") || !is_safe_rel_path(path.trim_start_matches('/')) {
            continue;
        }
        if !is_safe_rel_path(&file) {
            continue;
        }
        mappings.push(RouteMapping { method, path, file });
    }
    mappings
}

// Persist route mappings to config file.
pub fn write_route_mappings(mappings: &[RouteMapping]) -> std::io::Result<()> {
    let config_dir = base_config_dir();
    std::fs::create_dir_all(&config_dir)?;
    let path = config_dir.join("routes.txt");
    let mut out = String::new();
    for m in mappings {
        out.push_str(&m.method);
        out.push(' ');
        out.push_str(&m.path);
        out.push(' ');
        out.push_str(&m.file);
        out.push('\n');
    }
    std::fs::write(path, out)
}

// Escape text for safe HTML rendering.
pub fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

// Initialize the in-memory log buffer and broadcaster.
pub fn init_log_state() {
    let (sender, _) = broadcast::channel(256);
    let state = LogState {
        sender,
        buffer: Mutex::new(VecDeque::with_capacity(256)),
    };
    let _ = LOG_STATE.set(state);
}

// Subscribe to log events for SSE.
pub fn subscribe_logs() -> broadcast::Receiver<String> {
    LOG_STATE
        .get()
        .expect("log state not initialized")
        .sender
        .subscribe()
}

// Append a log line to buffer and broadcast it.
pub fn log_line(line: String) {
    if let Some(state) = LOG_STATE.get() {
        let _ = state.sender.send(line.clone());
        let mut buf = state.buffer.lock().unwrap();
        if buf.len() >= 200 {
            buf.pop_front();
        }
        buf.push_back(line);
    }
}

// Return a snapshot of the current log buffer.
pub fn log_snapshot() -> Vec<String> {
    LOG_STATE
        .get()
        .map(|state| state.buffer.lock().unwrap().iter().cloned().collect())
        .unwrap_or_default()
}

// Start filesystem watcher for json/ with log output.
pub fn start_fs_watch() {
    let base_dir = base_json_dir();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(err) => {
                tracing::error!(error = %err, "fs watch init failed");
                return;
            }
        };

        if let Err(err) = watcher.watch(&base_dir, notify::RecursiveMode::Recursive) {
            tracing::error!(error = %err, "fs watch start failed");
            return;
        }

        tracing::info!(path = %base_dir.display(), "fs watch started");
        for event in rx {
            match event {
                Ok(event) => {
                    let paths = event
                        .paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    tracing::info!(
                        kind = ?event.kind,
                        paths = %paths,
                        "fs event"
                    );
                }
                Err(err) => {
                    tracing::error!(error = %err, "fs watch error");
                }
            }
        }
    });
}
