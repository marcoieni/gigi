use std::path::Path;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::fs;
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
        .route(
            "/api/dashboard/filters",
            get(get_dashboard_filters).post(update_dashboard_filters),
        )
        .route("/api/threads", get(get_threads))
        .route(
            "/api/prs/{owner}/{repo}/{number}/review/latest",
            get(get_latest_review),
        )
        .route("/api/prs/{owner}/{repo}/{number}/review", post(run_review))
        .route("/api/threads/done", post(mark_done))
        .route("/api/prs/{owner}/{repo}/{number}/fix", post(run_fix))
        .route("/api/open/vscode", post(open_vscode))
        .route("/api/open/terminal", post(open_terminal))
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

pub async fn prepare_dashboard_assets(dashboard_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dashboard_dir).await?;

    fs::write(
        dashboard_dir.join("index.html"),
        include_str!("../assets/dashboard/index.html"),
    )
    .await?;
    fs::write(
        dashboard_dir.join("app.js"),
        include_str!("../assets/dashboard/app.js"),
    )
    .await?;
    fs::write(
        dashboard_dir.join("styles.css"),
        include_str!("../assets/dashboard/styles.css"),
    )
    .await?;

    Ok(())
}

async fn get_threads(
    State(state): State<std::sync::Arc<AppState>>,
    Query(query): Query<ThreadQuery>,
) -> Result<Json<Vec<crate::db::DashboardThread>>, ApiErrorResponse> {
    let default_filters = state
        .db
        .dashboard_thread_filters()
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    let threads = state
        .db
        .list_dashboard_threads_with_filters(query.resolve(default_filters))
        .map_err(|err| ApiErrorResponse::internal(&err))?;

    Ok(Json(threads))
}

async fn get_dashboard_filters(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<crate::db::DashboardThreadFilters>, ApiErrorResponse> {
    let filters = state
        .db
        .dashboard_thread_filters()
        .map_err(|err| ApiErrorResponse::internal(&err))?;

    Ok(Json(filters))
}

async fn update_dashboard_filters(
    State(state): State<std::sync::Arc<AppState>>,
    Json(filters): Json<crate::db::DashboardThreadFilters>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .db
        .set_dashboard_thread_filters(filters)
        .map_err(|err| ApiErrorResponse::internal(&err))?;

    Ok(StatusCode::OK)
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
    Json(request): Json<MarkDonePayload>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .mark_done(crate::serve::MarkDoneRequest {
            github_thread_id: request.github_thread_id,
            pr_url: request.pr_url,
            mark_authored_pr: request.mark_authored_pr,
        })
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

async fn run_review(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .run_review(owner, repo, number)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;

    Ok(StatusCode::OK)
}

async fn refresh(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<PollStats>, ApiErrorResponse> {
    let stats = state
        .poll_once_from_dashboard()
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(Json(stats))
}

async fn open_vscode(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<OpenProjectRequest>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .open_in_vscode(request.repository, request.pr_url)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
}

async fn open_terminal(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<OpenProjectRequest>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .open_in_terminal(request.repository, request.pr_url)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
}

#[derive(Debug, Serialize)]
struct FixResponse {
    output: String,
}

#[derive(Debug, Deserialize)]
struct OpenProjectRequest {
    repository: String,
    pr_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MarkDonePayload {
    github_thread_id: Option<String>,
    pr_url: Option<String>,
    mark_authored_pr: bool,
}

#[derive(Debug, Deserialize)]
struct ThreadQuery {
    show_notifications: Option<bool>,
    show_prs: Option<bool>,
    show_done: Option<bool>,
    show_not_done: Option<bool>,
}

impl ThreadQuery {
    fn resolve(
        self,
        defaults: crate::db::DashboardThreadFilters,
    ) -> crate::db::DashboardThreadFilters {
        crate::db::DashboardThreadFilters {
            show_notifications: self
                .show_notifications
                .unwrap_or(defaults.show_notifications),
            show_prs: self.show_prs.unwrap_or(defaults.show_prs),
            show_done: self.show_done.unwrap_or(defaults.show_done),
            show_not_done: self.show_not_done.unwrap_or(defaults.show_not_done),
            group_by_repository: defaults.group_by_repository,
        }
    }
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
