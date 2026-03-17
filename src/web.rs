use std::collections::HashMap;

use axum::{
    Form, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    config::AppConfig,
    dashboard::{self, DashboardSnapshot},
    db::DashboardThreadFilters,
    serve::AppState,
};

pub async fn run_server(state: std::sync::Arc<AppState>, config: &AppConfig) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(dashboard_page))
        .route("/dashboard/actions/filters", post(update_dashboard_filters))
        .route("/dashboard/actions/repo-filter", post(update_repo_filter))
        .route(
            "/dashboard/actions/repositories/hide",
            post(hide_repository),
        )
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

async fn update_dashboard_filters(
    State(state): State<std::sync::Arc<AppState>>,
    Form(form): Form<DashboardFiltersForm>,
) -> Redirect {
    match state.db.dashboard_thread_filters() {
        Ok(existing) => {
            let filters = form.into_filters(existing.hidden_repositories);
            if let Err(err) = state.db.set_dashboard_thread_filters(&filters) {
                state.notify_dashboard(format!("Failed to update filters: {err}"));
            } else {
                state.notify_dashboard("Filters updated");
            }
        }
        Err(err) => state.notify_dashboard(format!("Failed to load filters: {err}")),
    }

    redirect_home()
}

async fn update_repo_filter(
    State(state): State<std::sync::Arc<AppState>>,
    Form(form): Form<RepoFilterForm>,
) -> Redirect {
    match state.db.list_all_repositories() {
        Ok(all_repos) => {
            let hidden_repositories = form.hidden_repositories(&all_repos);
            if let Err(err) = state.db.set_repository_filter(&hidden_repositories) {
                state.notify_dashboard(format!("Failed to update repository filter: {err}"));
            } else {
                state.notify_dashboard("Repository filter updated");
            }
        }
        Err(err) => state.notify_dashboard(format!("Failed to load repositories: {err}")),
    }

    redirect_home()
}

async fn hide_repository(
    State(state): State<std::sync::Arc<AppState>>,
    Form(form): Form<HideRepositoryForm>,
) -> Redirect {
    match state.db.list_all_repositories() {
        Ok(all_repos) => {
            if !all_repos.iter().any(|repo| repo == &form.repository) {
                state.notify_dashboard(format!("Unknown repository: {}", form.repository));
                return redirect_home();
            }

            match state.db.dashboard_thread_filters() {
                Ok(filters) => {
                    let mut hidden_repositories = filters.hidden_repositories;
                    if !hidden_repositories
                        .iter()
                        .any(|repo| repo == &form.repository)
                    {
                        hidden_repositories.push(form.repository);
                        hidden_repositories.sort();
                    }

                    if let Err(err) = state.db.set_repository_filter(&hidden_repositories) {
                        state.notify_dashboard(format!("Failed to hide repository: {err}"));
                    } else {
                        state.notify_dashboard("Repository hidden");
                    }
                }
                Err(err) => {
                    state.notify_dashboard(format!("Failed to load repository filter: {err}"));
                }
            }
        }
        Err(err) => state.notify_dashboard(format!("Failed to load repositories: {err}")),
    }

    redirect_home()
}

async fn mark_done(
    State(state): State<std::sync::Arc<AppState>>,
    Form(form): Form<MarkDoneForm>,
) -> Redirect {
    if let Err(err) = state
        .mark_done(crate::serve::MarkDoneRequest {
            github_thread_id: form.github_thread_id,
            pr_url: form.pr_url,
            mark_authored_pr: form.mark_authored_pr,
        })
        .await
    {
        state.notify_dashboard(format!("Failed to mark item done: {err}"));
    }

    redirect_home()
}

async fn run_fix(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Redirect {
    if let Err(err) = state.run_fix(owner, repo, number).await {
        state.notify_dashboard(format!("Failed to start fix: {err}"));
    }

    redirect_home()
}

async fn run_review(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Redirect {
    if let Err(err) = state.run_review(owner, repo, number).await {
        state.notify_dashboard(format!("Failed to start review: {err}"));
    }

    redirect_home()
}

async fn refresh(State(state): State<std::sync::Arc<AppState>>) -> Redirect {
    if let Err(err) = state.poll_once_from_dashboard().await {
        state.notify_dashboard(format!("Failed to refresh dashboard: {err}"));
    }

    redirect_home()
}

async fn open_vscode(
    State(state): State<std::sync::Arc<AppState>>,
    Form(request): Form<OpenProjectRequest>,
) -> Redirect {
    if let Err(err) = state
        .open_in_vscode(request.repository, request.pr_url)
        .await
    {
        state.notify_dashboard(format!("Failed to open VS Code: {err}"));
    }

    redirect_home()
}

async fn open_terminal(
    State(state): State<std::sync::Arc<AppState>>,
    Form(request): Form<OpenProjectRequest>,
) -> Redirect {
    if let Err(err) = state
        .open_in_terminal(request.repository, request.pr_url)
        .await
    {
        state.notify_dashboard(format!("Failed to open terminal: {err}"));
    }

    redirect_home()
}

async fn stylesheet() -> impl IntoResponse {
    let headers = static_asset_headers("text/css; charset=utf-8");
    (headers, include_str!("../assets/dashboard/styles.css"))
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
    let available_repositories = state.db.list_all_repositories()?;
    let mut threads = state.db.list_dashboard_threads_with_filters(&filters)?;
    for thread in &mut threads {
        let participant_key = thread.pr_url.as_deref().or(thread.subject_url.as_deref());
        if let Some(key) = participant_key {
            thread.participants = state.db.get_pr_participants(key).unwrap_or_default();
        }
    }
    Ok(DashboardSnapshot {
        filters,
        threads,
        available_repositories,
        status_message: state.dashboard_status_message(),
    })
}

fn redirect_home() -> Redirect {
    Redirect::to("/")
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
    fn into_filters(self, hidden_repositories: Vec<String>) -> DashboardThreadFilters {
        DashboardThreadFilters {
            show_notifications: self.show_notifications.is_some(),
            show_prs: self.show_prs.is_some(),
            show_done: self.show_done.is_some(),
            show_not_done: self.show_not_done.is_some(),
            group_by_repository: self.group_by_repository.is_some(),
            hidden_repositories,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RepoFilterForm {
    #[serde(flatten)]
    fields: HashMap<String, String>,
}

impl RepoFilterForm {
    fn hidden_repositories(&self, all_repos: &[String]) -> Vec<String> {
        all_repos
            .iter()
            .filter(|repo| !self.fields.values().any(|value| value == *repo))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct HideRepositoryForm {
    repository: String,
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

    #[test]
    fn repo_filter_form_returns_hidden_repositories() {
        let form = RepoFilterForm {
            fields: HashMap::from([
                ("repo:0:a/b".to_string(), "a/b".to_string()),
                ("repo:1:c/d".to_string(), "c/d".to_string()),
            ]),
        };
        let all_repos = vec!["a/b".to_string(), "c/d".to_string(), "e/f".to_string()];

        assert_eq!(
            form.hidden_repositories(&all_repos),
            vec!["e/f".to_string()]
        );
    }
}
