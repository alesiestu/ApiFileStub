use axum::{
    body::Body,
    extract::{Multipart, Path},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use std::env;
use tokio::fs;
use tower_http::trace::TraceLayer;
use tracing::Level;
use walkdir::WalkDir;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    start_fs_watch();

    let app = Router::new()
        .route("/", get(index))
        .route("/json", get(index))
        .route("/json/", get(index))
        .route("/json/create", axum::routing::post(create_subdir))
        .route("/json/:subdir", get(subdir_index).post(upload_files))
        .route("/json/:subdir/*path", get(get_json))
        .layer(TraceLayer::new_for_http().on_request(log_request));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind");
    println!("Listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await.expect("server error");
}

async fn get_json(Path((subdir, path)): Path<(String, String)>) -> Response {
    if !is_safe_segment(&subdir) || path.is_empty() || !is_safe_rel_path(&path) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = base_json_dir().join(subdir).join(path);

    match fs::read(path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
            response
        }
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND.into_response(),
            _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
    }
}

async fn index() -> Response {
    let base_dir = base_json_dir();

    let (entries, subdirs) =
        tokio::task::spawn_blocking(move || collect_json_index(base_dir))
            .await
            .unwrap_or_default();

    let mut body = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>JSON endpoints</title><style>
        :root{--bg:#0b0f1a;--card:#12192a;--accent:#ffb703;--accent2:#219ebc;--text:#e5ecf4;--muted:#93a3b8;}
        *{box-sizing:border-box}body{margin:0;font-family:\"Space Grotesk\",system-ui,-apple-system,sans-serif;color:var(--text);
        background:radial-gradient(1200px 600px at 10% -10%, #1d2b4a 0%, transparent 60%),linear-gradient(180deg,#0b0f1a 0%,#0d1222 100%);}
        a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
        header{padding:40px 24px 16px;max-width:1000px;margin:0 auto}
        h1{margin:0;font-size:32px;letter-spacing:0.4px}
        p{color:var(--muted);max-width:760px}
        .grid{display:grid;gap:16px;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));max-width:1000px;margin:0 auto;padding:0 24px 48px}
        .card{background:var(--card);border:1px solid #1e2842;border-radius:14px;padding:16px}
        .tag{display:inline-block;padding:2px 8px;border-radius:999px;background:rgba(255,183,3,0.15);color:var(--accent);font-size:12px;margin-bottom:8px}
        ul{list-style:none;padding:0;margin:8px 0 0}
        li{padding:6px 0;border-bottom:1px dashed #1f2a44}
        li:last-child{border-bottom:none}
        .muted{color:var(--muted)}
        .pill{display:inline-block;margin-right:8px;padding:4px 10px;border-radius:999px;background:rgba(33,158,188,0.15);color:var(--accent2);font-size:12px}
        </style></head><body><header><span class=\"pill\">API stub</span><h1>JSON endpoints</h1>
        <p>Questa app espone automaticamente i file presenti in <code>json/</code> come endpoint HTTP. Ogni file diventa raggiungibile con <code>/json/&lt;sottocartella&gt;/&lt;file&gt;</code>. Le risposte vengono lette dal disco a ogni richiesta, quindi gli aggiornamenti sono immediati.</p>
        </header><section class=\"grid\">",
    );

    body.push_str("<div class=\"card\"><div class=\"tag\">Sottocartelle</div><ul>");
    for subdir in subdirs {
        body.push_str("<li><a href=\"/json/");
        body.push_str(&subdir);
        body.push_str("\">");
        body.push_str(&subdir);
        body.push_str("</a> <span class=\"muted\">/json/");
        body.push_str(&subdir);
        body.push_str("</span></li>");
    }
    body.push_str("</ul></div>");

    body.push_str("<div class=\"card\"><div class=\"tag\">File disponibili</div><ul>");
    for (path, url) in entries {
        body.push_str("<li><a href=\"");
        body.push_str(&url);
        body.push_str("\">");
        body.push_str(&path);
        body.push_str("</a></li>");
    }
    body.push_str("</ul></div>");

    body.push_str("<div class=\"card\"><div class=\"tag\">Crea sottocartella</div>");
    body.push_str("<form method=\"post\" action=\"/json/create\">");
    body.push_str("<label class=\"muted\">Nome sottocartella</label>");
    body.push_str("<input type=\"text\" name=\"name\" required>");
    body.push_str("<button type=\"submit\">Crea</button></form></div>");

    body.push_str("</section></body></html>");

    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

async fn subdir_index(Path(subdir): Path<String>) -> Response {
    if !is_safe_segment(&subdir) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let base_dir = base_json_dir().join(&subdir);
    let subdir_clone = subdir.clone();
    let entries = tokio::task::spawn_blocking(move || collect_subdir_entries(base_dir, subdir_clone))
        .await
        .unwrap_or_default();

    let mut body = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>JSON folder</title><style>
        :root{--bg:#0b0f1a;--card:#12192a;--accent:#ffb703;--text:#e5ecf4;--muted:#93a3b8;}
        *{box-sizing:border-box}body{margin:0;font-family:\"Space Grotesk\",system-ui,-apple-system,sans-serif;color:var(--text);
        background:radial-gradient(1200px 600px at 10% -10%, #1d2b4a 0%, transparent 60%),linear-gradient(180deg,#0b0f1a 0%,#0d1222 100%);}
        a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
        header{padding:32px 24px 12px;max-width:900px;margin:0 auto}
        h1{margin:0;font-size:28px}
        .wrap{max-width:900px;margin:0 auto;padding:0 24px 40px}
        .card{background:var(--card);border:1px solid #1e2842;border-radius:14px;padding:16px;margin-bottom:16px}
        ul{list-style:none;padding:0;margin:8px 0 0}
        li{padding:6px 0;border-bottom:1px dashed #1f2a44}
        li:last-child{border-bottom:none}
        label{display:block;margin-bottom:8px;color:var(--muted)}
        input[type=file],input[type=text]{width:100%;padding:10px;border-radius:10px;border:1px solid #1f2a44;background:#0d1425;color:var(--text)}
        button{margin-top:10px;background:var(--accent);border:none;color:#111;padding:10px 16px;border-radius:10px;font-weight:600;cursor:pointer}
        </style></head><body><header><a href=\"/json\">← torna all'indice</a><h1>Cartella</h1></header><div class=\"wrap\">",
    );

    body.push_str("<div class=\"card\"><h2>File disponibili</h2><ul>");
    for (path, url) in entries {
        body.push_str("<li><a href=\"");
        body.push_str(&url);
        body.push_str("\">");
        body.push_str(&path);
        body.push_str("</a></li>");
    }
    body.push_str("</ul></div>");

    body.push_str("<div class=\"card\"><h2>Upload</h2><form method=\"post\" enctype=\"multipart/form-data\">");
    body.push_str("<label>Carica uno o piu file. Verranno salvati con il nome originale.</label>");
    body.push_str("<input type=\"file\" name=\"files\" multiple>");
    body.push_str("<button type=\"submit\">Carica</button></form></div></div></body></html>");

    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

async fn upload_files(Path(subdir): Path<String>, mut multipart: Multipart) -> Response {
    if !is_safe_segment(&subdir) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let dir = base_json_dir().join(&subdir);
    if let Err(_) = fs::create_dir_all(&dir).await {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let mut saved_any = false;
    while let Ok(Some(field)) = multipart.next_field().await {
        let Some(file_name) = field.file_name().map(|s| s.to_string()) else {
            continue;
        };
        if !is_safe_segment(&file_name) {
            continue;
        }
        let Ok(bytes) = field.bytes().await else {
            continue;
        };
        let path = dir.join(file_name);
        if fs::write(path, bytes).await.is_ok() {
            saved_any = true;
        }
    }

    if !saved_any {
        return StatusCode::BAD_REQUEST.into_response();
    }

    Redirect::to(&format!("/json/{}", subdir)).into_response()
}

async fn create_subdir(body: String) -> Response {
    let name = body
        .split('&')
        .find_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?;
            let value = parts.next()?;
            if key == "name" {
                Some(value.replace('+', " "))
            } else {
                None
            }
        })
        .unwrap_or_default();

    if !is_safe_segment(&name) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let dir = base_json_dir().join(&name);
    if let Err(_) = fs::create_dir_all(&dir).await {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

fn collect_json_index(base_dir: std::path::PathBuf) -> (Vec<(String, String)>, Vec<String>) {
    let entries = collect_json_entries(base_dir.clone());
    let subdirs = collect_subdirs(base_dir);
    (entries, subdirs)
}

fn collect_json_entries(base_dir: std::path::PathBuf) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    if !base_dir.is_dir() {
        return entries;
    }

    for entry in WalkDir::new(&base_dir)
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

fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('/')
        && !segment.contains('\\')
}

fn log_request<B>(request: &axum::http::Request<B>, _span: &tracing::Span) {
    tracing::info!(
        method = %request.method(),
        uri = %request.uri(),
        "request"
    );
}

fn is_safe_rel_path(path: &str) -> bool {
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

fn base_json_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("json")
}

fn start_fs_watch() {
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

fn collect_subdirs(base_dir: std::path::PathBuf) -> Vec<String> {
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

fn collect_subdir_entries(base_dir: std::path::PathBuf, subdir: String) -> Vec<(String, String)> {
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
