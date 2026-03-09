use std::convert::Infallible;

use axum::{
    Form, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response, Sse, sse::Event, sse::KeepAlive},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio_stream::{StreamExt as _, wrappers::WatchStream};

use crate::{
    config::AppConfig,
    dashboard::{self, DashboardSnapshot},
    db::DashboardThreadFilters,
    serve::AppState,
};

pub async fn run_server(state: std::sync::Arc<AppState>, config: &AppConfig) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(dashboard_page))
        .route("/dashboard/fragment", get(dashboard_fragment))
        .route("/dashboard/events", get(dashboard_events))
        .route("/dashboard/actions/filters", post(update_dashboard_filters))
        .route("/dashboard/actions/done", post(mark_done))
        .route("/dashboard/actions/open/vscode", post(open_vscode))
        .route("/dashboard/actions/open/terminal", post(open_terminal))
        .route("/dashboard/actions/refresh", post(refresh))
        .route(
            "/dashboard/actions/prs/{owner}/{repo}/{number}/review",
            post(run_review),
        )
        .route(
            "/dashboard/actions/prs/{owner}/{repo}/{number}/fix",
            post(run_fix),
        )
        .route("/styles.css", get(stylesheet))
        .route("/app.js", get(script))
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

async fn dashboard_page(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Html<String>, ApiErrorResponse> {
    let snapshot = load_snapshot(&state).map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(Html(dashboard::render_page(&snapshot)))
}

async fn dashboard_fragment(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Html<String>, ApiErrorResponse> {
    let snapshot = load_snapshot(&state).map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(Html(dashboard::render_fragment(snapshot)))
}

async fn dashboard_events(
    State(state): State<std::sync::Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = WatchStream::new(state.subscribe_dashboard_updates())
        .skip(1)
        .map(|update| {
            Ok(Event::default()
                .event("update")
                .data(update.version.to_string()))
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn update_dashboard_filters(
    State(state): State<std::sync::Arc<AppState>>,
    Form(form): Form<DashboardFiltersForm>,
) -> Result<StatusCode, ApiErrorResponse> {
    let filters = form.into_filters();
    state
        .db
        .set_dashboard_thread_filters(filters)
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    state.notify_dashboard("Filters updated");
    Ok(StatusCode::OK)
}

async fn mark_done(
    State(state): State<std::sync::Arc<AppState>>,
    Form(form): Form<MarkDoneForm>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .mark_done(crate::serve::MarkDoneRequest {
            github_thread_id: form.github_thread_id,
            pr_url: form.pr_url,
            mark_authored_pr: form.mark_authored_pr,
        })
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
}

async fn run_fix(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .run_fix(owner, repo, number)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
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
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .poll_once_from_dashboard()
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
}

async fn open_vscode(
    State(state): State<std::sync::Arc<AppState>>,
    Form(request): Form<OpenProjectRequest>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .open_in_vscode(request.repository, request.pr_url)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
}

async fn open_terminal(
    State(state): State<std::sync::Arc<AppState>>,
    Form(request): Form<OpenProjectRequest>,
) -> Result<StatusCode, ApiErrorResponse> {
    state
        .open_in_terminal(request.repository, request.pr_url)
        .await
        .map_err(|err| ApiErrorResponse::internal(&err))?;
    Ok(StatusCode::OK)
}

async fn stylesheet() -> impl IntoResponse {
    let headers = static_asset_headers("text/css; charset=utf-8");
    (headers, include_str!("../assets/dashboard/styles.css"))
}

async fn script() -> impl IntoResponse {
    let headers = static_asset_headers("application/javascript; charset=utf-8");
    (headers, include_str!("../assets/dashboard/app.js"))
}

fn static_asset_headers(content_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300, must-revalidate"),
    );
    headers
}

fn load_snapshot(state: &AppState) -> anyhow::Result<DashboardSnapshot> {
    let filters = state.db.dashboard_thread_filters()?;
    let threads = state.db.list_dashboard_threads_with_filters(filters)?;
    Ok(DashboardSnapshot {
        filters,
        threads,
        status_message: state.dashboard_status_message(),
    })
}

#[derive(Debug, Deserialize)]
struct OpenProjectRequest {
    repository: String,
    pr_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MarkDoneForm {
    github_thread_id: Option<String>,
    pr_url: Option<String>,
    #[serde(default)]
    mark_authored_pr: bool,
}

#[derive(Debug, Deserialize)]
struct DashboardFiltersForm {
    show_notifications: Option<String>,
    show_prs: Option<String>,
    show_done: Option<String>,
    show_not_done: Option<String>,
    group_by_repository: Option<String>,
}

impl DashboardFiltersForm {
    fn into_filters(self) -> DashboardThreadFilters {
        DashboardThreadFilters {
            show_notifications: self.show_notifications.is_some(),
            show_prs: self.show_prs.is_some(),
            show_done: self.show_done.is_some(),
            show_not_done: self.show_not_done.is_some(),
            group_by_repository: self.group_by_repository.is_some(),
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
    fn into_response(self) -> Response {
        let body = axum::Json(ApiError { error: self.1 });
        (self.0, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_asset_headers_include_cache_control() {
        let headers = static_asset_headers("text/css; charset=utf-8");

        assert_eq!(
            headers.get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/css; charset=utf-8"))
        );
        assert_eq!(
            headers.get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static(
                "public, max-age=300, must-revalidate"
            ))
        );
    }
}
