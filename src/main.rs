mod api;
mod tools;

use axum::{middleware, routing::{get, post}, Router};
use tracing::Level;

// App entry point: init logging, filesystem watch, and HTTP router.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    tools::init_log_state();
    tools::start_fs_watch();

    let app = Router::new()
        .route("/", get(api::index))
        .route("/json", get(api::index))
        .route("/json/", get(api::index))
        .route("/events", get(api::sse_logs))
        .route("/json/create", axum::routing::post(api::create_subdir))
        .route("/json/delete", axum::routing::post(api::delete_subdir))
        .route("/json/rename", axum::routing::post(api::rename_subdir))
        .route("/json/:subdir", get(api::subdir_index).post(api::upload_files))
        .route("/json/:subdir/*path", get(api::get_json))
        .route("/config/refresh-endpoint", post(api::set_refresh_endpoint))
        .route("/config/ping-endpoint", post(api::set_ping_endpoint))
        .route("/config/route-mapping", post(api::set_route_mapping))
        .route("/config/log-ignore", post(api::set_log_ignore))
        .route("/config/log-toggle", post(api::set_log_toggle))
        .route("/api/*path", get(api::api_get).post(api::api_post))
        .layer(middleware::from_fn(api::log_middleware));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind");
    println!("Listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await.expect("server error");
}
