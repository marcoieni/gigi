use std::path::Path;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;
use tower_http::services::ServeDir;

use crate::{
    config::AppConfig,
    serve::{AppState, PollStats},
};

pub async fn run_server(
    state: std::sync::Arc<AppState>,
    config: &AppConfig,
    dashboard_dir: &Path,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/threads", get(get_threads))
        .route(
            "/api/prs/{owner}/{repo}/{number}/review/latest",
            get(get_latest_review),
        )
        .route("/api/threads/{thread_id}/done", post(mark_done))
        .route("/api/prs/{owner}/{repo}/{number}/fix", post(run_fix))
        .route("/api/refresh", post(refresh))
        .fallback_service(ServeDir::new(dashboard_dir).append_index_html_on_directories(true))
        .with_state(state);

    let listener =
        tokio::net::TcpListener::bind((config.dashboard.host.as_str(), config.dashboard.port))
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "Failed to bind HTTP server on {}:{}: {err}",
                    config.dashboard.host,
                    config.dashboard.port
                )
            })?;

    axum::serve(listener, app)
        .await
        .map_err(anyhow::Error::from)
}

pub fn prepare_dashboard_assets(dashboard_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dashboard_dir)?;

    std::fs::write(
        dashboard_dir.join("index.html"),
        include_str!("../assets/dashboard/index.html"),
    )?;
    std::fs::write(
        dashboard_dir.join("app.js"),
        include_str!("../assets/dashboard/app.js"),
    )?;
    std::fs::write(
        dashboard_dir.join("styles.css"),
        include_str!("../assets/dashboard/styles.css"),
    )?;

    Ok(())
}

async fn get_threads(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<Vec<crate::db::DashboardThread>>, ApiErrorResponse> {
    let threads = state
        .db
        .list_dashboard_threads()
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(Json(threads))
}

async fn get_latest_review(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Result<Json<Option<crate::db::StoredReview>>, ApiErrorResponse> {
    let review = state
        .db
        .latest_review_for_pr(&owner, &repo, number)
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(Json(review))
}

async fn mark_done(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath(thread_id): AxumPath<String>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .mark_done(thread_id)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
}

async fn run_fix(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Result<Json<FixResponse>, ApiErrorResponse> {
    let output = state
        .run_fix(owner, repo, number)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;

    Ok(Json(FixResponse { output }))
}

async fn refresh(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<PollStats>, ApiErrorResponse> {
    let stats = state
        .poll_once()
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(Json(stats))
}

#[derive(Debug, Serialize)]
struct FixResponse {
    output: String,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

struct ApiErrorResponse(StatusCode, String);

impl ApiErrorResponse {
    fn internal(err: &anyhow::Error) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> axum::response::Response {
        let body = Json(ApiError { error: self.1 });
        (self.0, body).into_response()
    }
}
