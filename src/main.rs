
use axum::{
    body::Body,
    extract::{Multipart, Path},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{sse::Event, IntoResponse, Redirect, Response, Sse},
    routing::{get, post},
    Router,
};
use notify::Watcher;
use std::sync::{Mutex, OnceLock};
use tokio::fs;
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tracing::Level;
use walkdir::WalkDir;

#[tokio::main]
// App entry point: init logging, filesystem watch, and HTTP router.
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    init_log_state();
    start_fs_watch();

    let app = Router::new()
        .route("/", get(index))
        .route("/json", get(index))
        .route("/json/", get(index))
        .route("/events", get(sse_logs))
        .route("/json/create", axum::routing::post(create_subdir))
        .route("/json/delete", axum::routing::post(delete_subdir))
        .route("/json/rename", axum::routing::post(rename_subdir))
        .route("/json/:subdir", get(subdir_index).post(upload_files))
        .route("/json/:subdir/*path", get(get_json))
        .route("/config/refresh-endpoint", post(set_refresh_endpoint))
        .route("/config/ping-endpoint", post(set_ping_endpoint))
        .route("/config/route-mapping", post(set_route_mapping))
        .route("/config/log-ignore", post(set_log_ignore))
        .route("/config/log-toggle", post(set_log_toggle))
        .route("/api/*path", get(api_get).post(api_post))
        .layer(middleware::from_fn(log_middleware));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind");
    println!("Listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await.expect("server error");
}

// Serve JSON files under json/<subdir>/<path> with safety checks.
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

// Render the main HTML dashboard.
async fn index() -> Response {
    let base_dir = base_json_dir();

    let refresh_endpoint = read_refresh_endpoint();
    let ping_endpoint = read_ping_endpoint();
    let route_mappings = read_route_mappings();
    let log_patterns = read_log_ignore_patterns();
    let log_enabled = read_log_enabled();
    let log_snapshot = log_snapshot();
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
        h2{margin:0 0 8px;font-size:20px}
        p{color:var(--muted);max-width:760px}
        .grid{display:grid;gap:16px;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));max-width:1000px;margin:0 auto;padding:0 24px 48px}
        .section{max-width:1000px;margin:0 auto;padding:0 24px 16px}
        .card{background:var(--card);border:1px solid #1e2842;border-radius:14px;padding:16px}
        .card + .card{margin-top:16px}
        .tag{display:inline-block;padding:2px 8px;border-radius:999px;background:rgba(255,183,3,0.15);color:var(--accent);font-size:12px;margin-bottom:8px}
        ul{list-style:none;padding:0;margin:8px 0 0}
        li{padding:6px 0;border-bottom:1px dashed #1f2a44}
        li:last-child{border-bottom:none}
        .muted{color:var(--muted)}
        .pill{display:inline-block;margin-right:8px;padding:4px 10px;border-radius:999px;background:rgba(33,158,188,0.15);color:var(--accent2);font-size:12px}
        input[type=file],input[type=text],select,textarea{width:100%;padding:10px;border-radius:10px;border:1px solid #1f2a44;background:#0d1425;color:var(--text)}
        .log{background:#0d1425;border:1px solid #1f2a44;border-radius:12px;padding:10px;max-height:220px;overflow:auto;font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,\"Liberation Mono\",monospace;font-size:12px}
        .log-line{padding:4px 0;border-bottom:1px dashed #1f2a44}
        .log-line:last-child{border-bottom:none}
        .tabs{max-width:1000px;margin:0 auto;padding:0 24px 8px;display:flex;gap:8px;flex-wrap:wrap}
        .tab-btn{border:1px solid #1f2a44;background:#0d1425;color:var(--text);padding:8px 14px;border-radius:999px;cursor:pointer}
        .tab-btn.active{background:var(--accent);color:#111;border-color:transparent}
        .tab-panel{display:none}
        .tab-panel.active{display:block}
        </style></head><body><header><span class=\"pill\">API stub</span><h1>JSON endpoints</h1>
        <p>Questa app espone automaticamente i file presenti in <code>json/</code> come endpoint HTTP. Ogni file diventa raggiungibile con <code>/json/&lt;sottocartella&gt;/&lt;file&gt;</code>. Le risposte vengono lette dal disco a ogni richiesta, quindi gli aggiornamenti sono immediati.</p>
        <p class=\"muted\">Autore: Alessandro Iannacone - <a href=\"https://iannaconealessandro.it\">iannaconealessandro.it</a></p>
        </header>",
    );

    body.push_str("<div class=\"tabs\">");
    body.push_str("<button class=\"tab-btn active\" data-tab=\"overview\">Panoramica</button>");
    body.push_str("<button class=\"tab-btn\" data-tab=\"routing\">Routing API</button>");
    body.push_str("<button class=\"tab-btn\" data-tab=\"settings\">Impostazioni</button>");
    body.push_str("</div>");

    body.push_str("<div id=\"overview\" class=\"tab-panel active\">");
    body.push_str("<section class=\"section\"><div class=\"card\"><h2>Log richieste (live)</h2>");
    body.push_str("<div id=\"log\" class=\"log\">");
    for line in log_snapshot {
        body.push_str("<div class=\"log-line\">");
        body.push_str(&html_escape(&line));
        body.push_str("</div>");
    }
    body.push_str("</div></div></section>");

    body.push_str("<section class=\"section\"><div class=\"card\"><h2>Endpoint attivi</h2>");
    body.push_str("<p class=\"muted\">Refresh: <code>");
    body.push_str(&refresh_endpoint);
    body.push_str("</code></p>");
    body.push_str("<p class=\"muted\">Ping: <code>");
    body.push_str(&ping_endpoint);
    body.push_str("</code></p>");
    body.push_str("<div class=\"tag\">Mappature API</div><ul>");
    for mapping in &route_mappings {
        body.push_str("<li><span class=\"pill\">");
        body.push_str(&mapping.method);
        body.push_str("</span> <code>");
        body.push_str(&mapping.path);
        body.push_str("</code> → <a href=\"/json/");
        body.push_str(&mapping.file);
        body.push_str("\">");
        body.push_str(&mapping.file);
        body.push_str("</a></li>");
    }
    if route_mappings.is_empty() {
        body.push_str("<li class=\"muted\">Nessuna mappatura configurata</li>");
    }
    body.push_str("</ul></div></section>");

    body.push_str("<section class=\"grid\">");
    body.push_str("<div class=\"card\"><div class=\"tag\">Sottocartelle</div><ul>");
    for subdir in &subdirs {
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
    for (path, url) in &entries {
        body.push_str("<li><a href=\"");
        body.push_str(&url);
        body.push_str("\">");
        body.push_str(&path);
        body.push_str("</a></li>");
    }
    body.push_str("</ul></div>");

    body.push_str("</section></div>");

    body.push_str("<div id=\"routing\" class=\"tab-panel\">");
    body.push_str("<section class=\"section\"><div class=\"card\"><h2>Routing API</h2>");
    body.push_str("<p class=\"muted\">Associa un endpoint <code>/api/...</code> a un file JSON in <code>json/</code>.</p>");
    body.push_str("<form method=\"post\" action=\"/config/route-mapping\">");
    body.push_str("<label class=\"muted\">Metodo</label>");
    body.push_str("<select name=\"method\"><option>GET</option><option>POST</option></select>");
    body.push_str("<label class=\"muted\">Path</label>");
    body.push_str("<input type=\"text\" name=\"path\" placeholder=\"/api/v1/ipv4/get/all\" required>");
    body.push_str("<label class=\"muted\">File (relativo a json/)</label>");
    body.push_str("<input type=\"text\" name=\"file\" list=\"file-options\" placeholder=\"ipv4/file.json\" required>");
    body.push_str("<button type=\"submit\">Associa</button></form>");
    body.push_str("<datalist id=\"file-options\">");
    for (path, _url) in &entries {
        body.push_str("<option value=\"");
        body.push_str(path);
        body.push_str("\"></option>");
    }
    body.push_str("</datalist>");

    body.push_str("<div class=\"tag\">Associazioni attive</div><ul>");
    for mapping in &route_mappings {
        body.push_str("<li><span class=\"pill\">");
        body.push_str(&mapping.method);
        body.push_str("</span> <code>");
        body.push_str(&mapping.path);
        body.push_str("</code> → <a href=\"/json/");
        body.push_str(&mapping.file);
        body.push_str("\">");
        body.push_str(&mapping.file);
        body.push_str("</a></li>");
    }
    if route_mappings.is_empty() {
        body.push_str("<li class=\"muted\">Nessuna associazione configurata</li>");
    }
    body.push_str("</ul></div></section></div>");

    body.push_str("<div id=\"settings\" class=\"tab-panel\">");
    body.push_str("<section class=\"section\">");
    body.push_str("<div class=\"card\"><h2>Autenticazione</h2>");
    body.push_str("<p class=\"muted\">Configura l'endpoint di refresh token e usa la risposta JSON salvata su disco.</p>");
    body.push_str("<p class=\"muted\">Endpoint attuale: <code>");
    body.push_str(&refresh_endpoint);
    body.push_str("</code></p>");
    body.push_str("<form method=\"post\" action=\"/config/refresh-endpoint\">");
    body.push_str("<label class=\"muted\">Imposta un endpoint sotto <code>/api/</code></label>");
    body.push_str("<input type=\"text\" name=\"path\" value=\"");
    body.push_str(&refresh_endpoint);
    body.push_str("\" required>");
    body.push_str("<button type=\"submit\">Aggiorna</button></form></div>");

    body.push_str("<div class=\"card\"><h2>Ping API</h2>");
    body.push_str("<p class=\"muted\">Endpoint di check connessione che ritorna uno stato JSON.</p>");
    body.push_str("<p class=\"muted\">Endpoint attuale: <code>");
    body.push_str(&ping_endpoint);
    body.push_str("</code></p>");
    body.push_str("<form method=\"post\" action=\"/config/ping-endpoint\">");
    body.push_str("<label class=\"muted\">Imposta un endpoint sotto <code>/api/</code></label>");
    body.push_str("<input type=\"text\" name=\"path\" value=\"");
    body.push_str(&ping_endpoint);
    body.push_str("\" required>");
    body.push_str("<button type=\"submit\">Aggiorna</button></form></div>");

    body.push_str("<div class=\"card\"><h2>Gestione cartelle</h2>");
    body.push_str("<p class=\"muted\">Crea, rinomina o elimina sottocartelle sotto <code>json/</code>.</p>");
    body.push_str("<form method=\"post\" action=\"/json/create\">");
    body.push_str("<label class=\"muted\">Nome sottocartella</label>");
    body.push_str("<input type=\"text\" name=\"name\" required>");
    body.push_str("<button type=\"submit\">Crea</button></form>");

    body.push_str("<form method=\"post\" action=\"/json/rename\">");
    body.push_str("<label class=\"muted\">Rinomina cartella</label>");
    body.push_str("<select name=\"from\">");
    for subdir in &subdirs {
        body.push_str("<option value=\"");
        body.push_str(subdir);
        body.push_str("\">");
        body.push_str(subdir);
        body.push_str("</option>");
    }
    body.push_str("</select>");
    body.push_str("<input type=\"text\" name=\"to\" placeholder=\"nuovo_nome\" required>");
    body.push_str("<button type=\"submit\">Rinomina</button></form>");

    body.push_str("<form method=\"post\" action=\"/json/delete\">");
    body.push_str("<label class=\"muted\">Elimina cartella</label>");
    body.push_str("<select name=\"name\">");
    for subdir in &subdirs {
        body.push_str("<option value=\"");
        body.push_str(subdir);
        body.push_str("\">");
        body.push_str(subdir);
        body.push_str("</option>");
    }
    body.push_str("</select>");
    body.push_str("<button type=\"submit\">Elimina</button></form></div>");

    body.push_str("<div class=\"card\"><h2>Filtri log</h2>");
    body.push_str("<p class=\"muted\">Inserisci uno per riga. Supporta match esatto o prefisso con <code>/*</code> (es. <code>/json/*</code>).</p>");
    body.push_str("<form method=\"post\" action=\"/config/log-ignore\">");
    body.push_str("<label class=\"muted\">Path da ignorare</label>");
    body.push_str("<textarea name=\"patterns\" rows=\"4\" required>");
    if !log_patterns.is_empty() {
        body.push_str(&html_escape(&log_patterns.join("\n")));
    }
    body.push_str("</textarea>");
    body.push_str("<button type=\"submit\">Aggiorna</button></form></div>");

    body.push_str("<div class=\"card\"><h2>Log globale</h2>");
    body.push_str("<p class=\"muted\">Abilita o disabilita completamente i log di richieste e risposte.</p>");
    body.push_str("<form method=\"post\" action=\"/config/log-toggle\">");
    body.push_str("<label class=\"muted\">Stato log</label>");
    body.push_str("<select name=\"enabled\">");
    body.push_str("<option value=\"on\"");
    if log_enabled {
        body.push_str(" selected");
    }
    body.push_str(">ON</option>");
    body.push_str("<option value=\"off\"");
    if !log_enabled {
        body.push_str(" selected");
    }
    body.push_str(">OFF</option>");
    body.push_str("</select>");
    body.push_str("<button type=\"submit\">Salva</button></form></div>");
    body.push_str("</section></div>");

    body.push_str("<script>
    (function(){
        const logEl = document.getElementById('log');
        const es = new EventSource('/events');
        es.onmessage = (e) => {
            const line = document.createElement('div');
            line.className = 'log-line';
            line.textContent = e.data;
            logEl.appendChild(line);
            while (logEl.children.length > 200) {
                logEl.removeChild(logEl.firstChild);
            }
            logEl.scrollTop = logEl.scrollHeight;
        };

        const buttons = document.querySelectorAll('.tab-btn');
        const panels = document.querySelectorAll('.tab-panel');
        const activate = (id) => {
            buttons.forEach(btn => btn.classList.toggle('active', btn.dataset.tab === id));
            panels.forEach(panel => panel.classList.toggle('active', panel.id === id));
        };
        buttons.forEach(btn => {
            btn.addEventListener('click', () => activate(btn.dataset.tab));
        });
    })();
    </script></body></html>");

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

// Render per-subdirectory page with file list and upload form.
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

// Handle multipart uploads into json/<subdir>.
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

// Return refresh-token JSON response from file or fallback.
async fn refresh_token() -> Response {
    let path = base_json_dir().join("authentication").join("refresh.json");
    let bytes = match fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let fallback = r#"{"status":"success","data":{"access_token":"dev_access_token"}} "#;
            fallback.as_bytes().to_vec()
        }
    };

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

// Return ping JSON response from file or fallback.
async fn ping_response() -> Response {
    let path = base_json_dir().join("ping").join("response.json");
    let bytes = match fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let fallback = r#"{"status":"success"}"#;
            fallback.as_bytes().to_vec()
        }
    };

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

// Route GET /api/* to ping or mapped JSON files.
async fn api_get(Path(path): Path<String>) -> Response {
    let requested = format!("/api/{}", path);
    if read_ping_endpoint() == requested {
        return ping_response().await;
    }

    if let Some(file) = find_route_mapping("GET", &requested) {
        return serve_mapped_json(&file).await;
    }

    StatusCode::NOT_FOUND.into_response()
}

// Route POST /api/* to refresh or mapped JSON files.
async fn api_post(Path(path): Path<String>) -> Response {
    let requested = format!("/api/{}", path);
    if read_refresh_endpoint() == requested {
        return refresh_token().await;
    }

    if let Some(file) = find_route_mapping("POST", &requested) {
        return serve_mapped_json(&file).await;
    }

    StatusCode::NOT_FOUND.into_response()
}

// Persist configurable refresh endpoint.
async fn set_refresh_endpoint(body: String) -> Response {
    let Some(path) = form_value(&body, "path") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !path.starts_with("/api/") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if !is_safe_rel_path(path.trim_start_matches('/')) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let config_dir = base_config_dir();
    if fs::create_dir_all(&config_dir).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let file_path = config_dir.join("refresh_endpoint.txt");
    if fs::write(file_path, path).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Persist configurable ping endpoint.
async fn set_ping_endpoint(body: String) -> Response {
    let Some(path) = form_value(&body, "path") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !path.starts_with("/api/") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if !is_safe_rel_path(path.trim_start_matches('/')) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let config_dir = base_config_dir();
    if fs::create_dir_all(&config_dir).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let file_path = config_dir.join("ping_endpoint.txt");
    if fs::write(file_path, path).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Persist list of log-ignored paths.
async fn set_log_ignore(body: String) -> Response {
    let Some(patterns) = form_value(&body, "patterns") else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let mut lines = Vec::new();
    for line in patterns.lines() {
        if let Some(normalized) = normalize_log_pattern(line) {
            lines.push(normalized);
        }
    }

    let config_dir = base_config_dir();
    if fs::create_dir_all(&config_dir).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let file_path = config_dir.join("log_ignore.txt");
    let data = lines.join("\n");
    if fs::write(file_path, data).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Enable or disable logging globally.
async fn set_log_toggle(body: String) -> Response {
    let Some(value) = form_value(&body, "enabled") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let enabled = value.trim().eq_ignore_ascii_case("on");
    let config_dir = base_config_dir();
    if fs::create_dir_all(&config_dir).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let file_path = config_dir.join("log_enabled.txt");
    let data = if enabled { "on" } else { "off" };
    if fs::write(file_path, data).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Persist mapping from API path+method to JSON file.
async fn set_route_mapping(body: String) -> Response {
    let Some(method) = form_value(&body, "method") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(path) = form_value(&body, "path") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(file) = form_value(&body, "file") else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let method = method.trim().to_uppercase();
    if method != "GET" && method != "POST" {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = path.trim().to_string();
    if !path.starts_with("/api/") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if !is_safe_rel_path(path.trim_start_matches('/')) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let file = match normalize_json_file(&file) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !is_safe_rel_path(&file) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let mut mappings = read_route_mappings();
    mappings.retain(|m| !(m.method == method && m.path == path));
    mappings.push(RouteMapping { method, path, file });
    if write_route_mappings(&mappings).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Create a new subdirectory under json/.
async fn create_subdir(body: String) -> Response {
    let name = form_value(&body, "name").unwrap_or_default();

    if !is_safe_segment(&name) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let dir = base_json_dir().join(&name);
    if let Err(_) = fs::create_dir_all(&dir).await {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Delete a subdirectory under json/.
async fn delete_subdir(body: String) -> Response {
    let name = form_value(&body, "name").unwrap_or_default();
    if !is_safe_segment(&name) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let dir = base_json_dir().join(&name);
    if fs::remove_dir_all(&dir).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Rename a subdirectory under json/.
async fn rename_subdir(body: String) -> Response {
    let from = form_value(&body, "from").unwrap_or_default();
    let to = form_value(&body, "to").unwrap_or_default();
    if !is_safe_segment(&from) || !is_safe_segment(&to) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let from_dir = base_json_dir().join(&from);
    let to_dir = base_json_dir().join(&to);
    if fs::rename(from_dir, to_dir).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Redirect::to("/json").into_response()
}

// Collect file entries and subdir names for the UI.
fn collect_json_index(base_dir: std::path::PathBuf) -> (Vec<(String, String)>, Vec<String>) {
    let entries = collect_json_entries(base_dir.clone());
    let subdirs = collect_subdirs(base_dir);
    (entries, subdirs)
}

// Walk json/ and list all JSON file paths.
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

// Validate a single path segment to prevent traversal.
fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('/')
        && !segment.contains('\\')
}

// Log requests and responses unless filtered.
async fn log_middleware(request: axum::http::Request<Body>, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let enabled = read_log_enabled();
    let ignored = is_log_ignored(&path);
    if enabled && !ignored {
        tracing::info!(
            method = %request.method(),
            uri = %request.uri(),
            "request"
        );
        log_line(format!("REQ {} {}", request.method(), request.uri()));
    }

    let response = next.run(request).await;
    if enabled && !ignored {
        tracing::info!(
            status = %response.status(),
            "response"
        );
        log_line(format!("RES {}", response.status()));
    }
    response
}

// Stream log lines to the browser via SSE.
async fn sse_logs() -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let receiver = log_state()
        .sender
        .subscribe();
    let stream = BroadcastStream::new(receiver).filter_map(|msg| match msg {
        Ok(line) => Some(Ok(Event::default().data(line))),
        Err(_) => None,
    });
    Sse::new(stream)
}

// Validate a relative path (no traversal or prefixes).
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

// Resolve the json/ directory path.
fn base_json_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("json")
}

// Resolve the config/ directory path.
fn base_config_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config")
}

// Load refresh endpoint from config or default.
fn read_refresh_endpoint() -> String {
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
fn read_ping_endpoint() -> String {
    let path = base_config_dir().join("ping_endpoint.txt");
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        "/api/v1/ping".to_string()
    } else {
        trimmed.to_string()
    }
}

// Lookup a mapping for the given method and path.
fn find_route_mapping(method: &str, path: &str) -> Option<String> {
    read_route_mappings()
        .into_iter()
        .find(|m| m.method == method && m.path == path)
        .map(|m| m.file)
}

// Load route mappings from config file.
fn read_route_mappings() -> Vec<RouteMapping> {
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
fn write_route_mappings(mappings: &[RouteMapping]) -> std::io::Result<()> {
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

// Read and return the mapped JSON response.
async fn serve_mapped_json(file: &str) -> Response {
    let path = base_json_dir().join(file);
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

// Normalize a JSON file path relative to json/.
fn normalize_json_file(input: &str) -> Result<String, Response> {
    let mut trimmed = input.trim().to_string();
    if trimmed.starts_with("/json/") {
        trimmed = trimmed.trim_start_matches("/json/").to_string();
    }
    if trimmed.starts_with("json/") {
        trimmed = trimmed.trim_start_matches("json/").to_string();
    }
    if trimmed.starts_with('/') {
        return Err(StatusCode::BAD_REQUEST.into_response());
    }
    Ok(trimmed)
}

// Load log ignore patterns from config.
fn read_log_ignore_patterns() -> Vec<String> {
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

// Load the global log enabled toggle.
fn read_log_enabled() -> bool {
    let path = base_config_dir().join("log_enabled.txt");
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    let trimmed = contents.trim().to_lowercase();
    trimmed.is_empty() || trimmed == "on" || trimmed == "true" || trimmed == "1"
}

// Check whether a path matches ignore patterns.
fn is_log_ignored(path: &str) -> bool {
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

// Parse a urlencoded form field value.
fn form_value(body: &str, key: &str) -> Option<String> {
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
fn url_decode(input: &str) -> String {
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
fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// Normalize and validate a log ignore pattern.
fn normalize_log_pattern(input: &str) -> Option<String> {
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

struct LogState {
    sender: tokio::sync::broadcast::Sender<String>,
    buffer: Mutex<std::collections::VecDeque<String>>,
}

#[derive(Clone)]
struct RouteMapping {
    method: String,
    path: String,
    file: String,
}

static LOG_STATE: OnceLock<LogState> = OnceLock::new();

// Initialize the in-memory log buffer and broadcaster.
fn init_log_state() {
    let (sender, _) = broadcast::channel(256);
    let state = LogState {
        sender,
        buffer: Mutex::new(std::collections::VecDeque::with_capacity(256)),
    };
    let _ = LOG_STATE.set(state);
}

// Get the global log state instance.
fn log_state() -> &'static LogState {
    LOG_STATE.get().expect("log state not initialized")
}

// Append a log line to buffer and broadcast it.
fn log_line(line: String) {
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
fn log_snapshot() -> Vec<String> {
    LOG_STATE
        .get()
        .map(|state| state.buffer.lock().unwrap().iter().cloned().collect())
        .unwrap_or_default()
}

// Escape text for safe HTML rendering.
fn html_escape(input: &str) -> String {
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

// Start filesystem watcher for json/ with log output.
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

// List immediate subdirectories under json/.
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

// List files inside a specific json subdirectory.
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
