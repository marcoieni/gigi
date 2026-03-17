use std::{collections::HashMap, convert::Infallible, sync::Arc};

use axum::{
    Form, Router,
    body::Body,
    extract::{FromRef, Path as AxumPath, Request, State},
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Redirect, Sse, sse::Event, sse::KeepAlive},
    routing::{get, post},
};
use leptos::{config::LeptosOptions, prelude::provide_context};
use serde::Deserialize;
use tokio_stream::{StreamExt as _, wrappers::WatchStream};

use crate::{app, config::AppConfig, db::DashboardThreadFilters, serve::AppState};

#[derive(Clone)]
struct WebState {
    app_state: Arc<AppState>,
    leptos_options: LeptosOptions,
}

impl FromRef<WebState> for Arc<AppState> {
    fn from_ref(state: &WebState) -> Self {
        Self::clone(&state.app_state)
    }
}

impl FromRef<WebState> for LeptosOptions {
    fn from_ref(state: &WebState) -> Self {
        state.leptos_options.clone()
    }
}

pub async fn run_server(state: Arc<AppState>, config: &AppConfig) -> anyhow::Result<()> {
    let leptos_options = build_leptos_options(config);
    let web_state = WebState {
        app_state: Arc::clone(&state),
        leptos_options: leptos_options.clone(),
    };

    let app_context = {
        let state = Arc::clone(&state);
        move || provide_context(Arc::clone(&state))
    };

    let shell = {
        let leptos_options = leptos_options.clone();
        move || app::shell(leptos_options.clone())
    };

    let app = Router::new()
        .route(
            "/api/{*fn_name}",
            post(server_fn_handler).get(server_fn_handler),
        )
        .route(
            "/",
            get(leptos_axum::render_app_to_stream_with_context(
                app_context.clone(),
                shell,
            )),
        )
        .route("/dashboard/events", get(dashboard_events))
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
        .fallback(file_and_error_handler)
        .with_state(web_state);

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

async fn server_fn_handler(
    State(state): State<WebState>,
    request: Request<Body>,
) -> impl IntoResponse {
    let app_state = Arc::clone(&state.app_state);
    leptos_axum::handle_server_fns_with_context(
        move || provide_context(Arc::clone(&app_state)),
        request,
    )
    .await
}

async fn dashboard_events(
    State(state): State<Arc<AppState>>,
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
    State(state): State<Arc<AppState>>,
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
    State(state): State<Arc<AppState>>,
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
    State(state): State<Arc<AppState>>,
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

async fn mark_done(State(state): State<Arc<AppState>>, Form(form): Form<MarkDoneForm>) -> Redirect {
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
    State(state): State<Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Redirect {
    if let Err(err) = state.run_fix(owner, repo, number).await {
        state.notify_dashboard(format!("Failed to start fix: {err}"));
    }

    redirect_home()
}

async fn run_review(
    State(state): State<Arc<AppState>>,
    AxumPath((owner, repo, number)): AxumPath<(String, String, i64)>,
) -> Redirect {
    if let Err(err) = state.run_review(owner, repo, number).await {
        state.notify_dashboard(format!("Failed to start review: {err}"));
    }

    redirect_home()
}

async fn refresh(State(state): State<Arc<AppState>>) -> Redirect {
    if let Err(err) = state.poll_once_from_dashboard().await {
        state.notify_dashboard(format!("Failed to refresh dashboard: {err}"));
    }

    redirect_home()
}

async fn open_vscode(
    State(state): State<Arc<AppState>>,
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
    State(state): State<Arc<AppState>>,
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

async fn file_and_error_handler(
    uri: axum::http::Uri,
    State(state): State<WebState>,
    request: Request<Body>,
) -> impl IntoResponse {
    let app_state = Arc::clone(&state.app_state);
    leptos_axum::file_and_error_handler_with_context(
        move || provide_context(Arc::clone(&app_state)),
        app::shell,
    )(uri, State(state), request)
    .await
}

fn build_leptos_options(config: &AppConfig) -> LeptosOptions {
    use std::net::SocketAddr;

    let site_addr = format!("{}:{}", config.dashboard.host, config.dashboard.port)
        .parse::<SocketAddr>()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], config.dashboard.port)));
    LeptosOptions::builder()
        .output_name(env!("CARGO_CRATE_NAME"))
        .site_root("target/site")
        .site_pkg_dir("pkg")
        .site_addr(site_addr)
        .build()
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
