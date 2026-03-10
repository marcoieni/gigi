use std::collections::HashMap;

use leptos::prelude::*;

use crate::db::{DashboardThread, DashboardThreadFilters};

const VSCODE_ICON_PATHS: &[&str] = &["M9 8 5 12l4 4", "m15 8 4 4-4 4", "M14 6 10 18"];
const TERMINAL_ICON_PATHS: &[&str] = &[
    "M4 5.5h16a1.5 1.5 0 0 1 1.5 1.5v10A1.5 1.5 0 0 1 20 18.5H4A1.5 1.5 0 0 1 2.5 17V7A1.5 1.5 0 0 1 4 5.5Z",
    "m6.4 9 2.6 2.3-2.6 2.3",
    "M11.7 13.8h4.9",
];
const NOTIFICATION_ICON_PATHS: &[&str] = &[
    "M15.5 17.5h4l-1.1-1.1a2 2 0 0 1-.6-1.4V11a5.8 5.8 0 1 0-11.6 0v4a2 2 0 0 1-.6 1.4l-1.1 1.1h4",
    "M10 17.5a2 2 0 0 0 4 0",
];
const MY_PR_ICON_PATHS: &[&str] = &[
    "M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M15.5 9v5.5a3 3 0 0 1-3 3H8",
    "M5.5 15V9",
];
const PR_OPEN_ICON_PATHS: &[&str] = &[
    "M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M5.5 15V9",
    "M8 17.5h4.5a3 3 0 0 0 3-3V8",
    "m15.5 8 2.8 2.8L21 8",
];
const PR_MERGED_ICON_PATHS: &[&str] = &[
    "M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M18 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M8 15V9.5a3 3 0 0 1 3-3h2",
    "M15.5 10.5V15",
];
const PR_CLOSED_ICON_PATHS: &[&str] = &[
    "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
    "M5.5 15V8.5",
    "M14.5 8.5 20 14",
    "M20 8.5 14.5 14",
];
const ISSUE_OPEN_ICON_PATHS: &[&str] = &[
    "M12 21a9 9 0 1 1 0-18a9 9 0 0 1 0 18Z",
    "M12 8.5v5",
    "M12 16.5h.01",
];
const ISSUE_CLOSED_ICON_PATHS: &[&str] = &[
    "M12 21a9 9 0 1 1 0-18a9 9 0 0 1 0 18Z",
    "M9 9l6 6",
    "M15 9l-6 6",
];

#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub filters: DashboardThreadFilters,
    pub threads: Vec<DashboardThread>,
    pub status_message: String,
}

pub fn render_page(snapshot: &DashboardSnapshot) -> String {
    view! {
        <!doctype html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <title>"gigi dashboard"</title>
                <link rel="stylesheet" href="/styles.css" />
            </head>
            <body>
                <div id="dashboard-root">{render_fragment_view(snapshot.clone())}</div>
                <script src="/app.js"></script>
            </body>
        </html>
    }
    .to_html()
}

pub fn render_fragment(snapshot: DashboardSnapshot) -> String {
    render_fragment_view(snapshot).to_html()
}

fn render_fragment_view(snapshot: DashboardSnapshot) -> impl IntoView {
    let grouped = snapshot.filters.group_by_repository;
    let groups = grouped_threads(&snapshot.threads);

    view! {
        <main class="layout">
            <header class="header">
                <h1>"gigi dashboard"</h1>
                <div class="actions">
                    <form action="/dashboard/actions/refresh" method="post" data-async-form>
                        <button class="btn" type="submit" data-loading-label="Refreshing...">"Refresh"</button>
                    </form>
                    <span id="status-text" class="status">{snapshot.status_message}</span>
                    <a
                        class="btn icon-btn header-link"
                        href="https://github.com/notifications"
                        target="_blank"
                        rel="noreferrer"
                        aria-label="Open GitHub notifications"
                        title="Open GitHub notifications"
                    >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M4.5 7.5A2.5 2.5 0 0 1 7 5h10a2.5 2.5 0 0 1 2.5 2.5v9A2.5 2.5 0 0 1 17 19H7a2.5 2.5 0 0 1-2.5-2.5v-9Z" />
                            <path d="m6.5 8.5 5.5 4 5.5-4" />
                            <path d="M8 15.5h8" />
                        </svg>
                    </a>
                </div>
            </header>

            <form class="filters" aria-label="Dashboard filters" action="/dashboard/actions/filters" method="post" data-async-form data-auto-submit-form>
                <fieldset class="filter-group">
                    <legend>"Show"</legend>
                    <FilterCheckbox name="show_notifications" label="Notifications" checked=snapshot.filters.show_notifications />
                    <FilterCheckbox name="show_prs" label="PRs" checked=snapshot.filters.show_prs />
                </fieldset>
                <fieldset class="filter-group">
                    <legend>"Status"</legend>
                    <FilterCheckbox name="show_not_done" label="Not done" checked=snapshot.filters.show_not_done />
                    <FilterCheckbox name="show_done" label="Done" checked=snapshot.filters.show_done />
                </fieldset>
                <fieldset class="filter-group">
                    <legend>"Display"</legend>
                    <FilterCheckbox name="group_by_repository" label="Group by repository" checked=snapshot.filters.group_by_repository />
                </fieldset>
            </form>

            <section>
                {if snapshot.threads.is_empty() {
                    view! { <div class="threads"><article class="thread"><h3>"Nothing to review"</h3><p class="meta">"No cards match the current filters."</p></article></div> }.into_any()
                } else if grouped {
                    view! {
                        <div class="threads grouped">
                            {groups.into_iter().map(|(repository, threads)| view! { <RepositorySection repository threads /> }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="threads">
                            {snapshot.threads.into_iter().map(|thread| view! { <ThreadCard thread /> }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                }}
            </section>

            <dialog id="review-modal">
                <article>
                    <header class="modal-head">
                        <h2>"Review"</h2>
                        <button id="close-modal" class="btn" type="button">"Close"</button>
                    </header>
                    <pre id="review-content"></pre>
                </article>
            </dialog>
        </main>
    }
}

#[component]
fn FilterCheckbox(name: &'static str, label: &'static str, checked: bool) -> impl IntoView {
    view! {
        <label class="filter-option">
            <input type="checkbox" name=name checked=checked />
            <span>{label}</span>
        </label>
    }
}

#[component]
fn RepositorySection(repository: String, threads: Vec<DashboardThread>) -> impl IntoView {
    let count = threads.len();
    let repo_link = format!("https://github.com/{repository}");

    view! {
        <section class="repo-group">
            <header class="repo-group-header">
                <h2 class="repo-group-title">
                    <a class="thread-link repo-link" href=repo_link target="_blank" rel="noreferrer">{repository}</a>
                </h2>
                <span class="repo-group-count">{format!("{count} {}", if count == 1 { "item" } else { "items" })}</span>
            </header>
            <div class="threads">
                {threads.into_iter().map(|thread| view! { <ThreadCard thread /> }).collect::<Vec<_>>()}
            </div>
        </section>
    }
}

#[component]
fn ThreadCard(thread: DashboardThread) -> impl IntoView {
    let destination = thread
        .subject_url
        .clone()
        .or_else(|| thread.pr_url.clone())
        .unwrap_or_else(|| format!("https://github.com/{}", thread.repository));
    let review_content = thread.latest_review_content_md.clone();
    let review_tone = match thread.latest_requires_code_changes {
        Some(true) => "unsafe",
        Some(false) => "safe",
        None => "pending",
    };
    let review_label = match thread.latest_requires_code_changes {
        Some(true) => "Fixes needed",
        Some(false) => "Safe",
        None => "No review",
    };
    let can_review =
        thread.pr_owner.is_some() && thread.pr_repo.is_some() && thread.pr_number.is_some();
    let can_fix = can_review && thread.latest_requires_code_changes == Some(true);
    let mark_authored_pr = thread.sources.iter().any(|source| source == "my_pr");
    let review_action = review_action_path(&thread);
    let fix_action = fix_action_path(&thread);
    let (state_icon_class, state_icon_paths, state_icon_label) = thread_state_data(
        thread.subject_type.as_deref(),
        thread.pr_state.as_deref().or(thread.issue_state.as_deref()),
    );

    view! {
        <article class="thread">
            <h3>
                <span class=state_icon_class aria-label=state_icon_label title=state_icon_label>{svg_icon(state_icon_paths)}</span>
                <a class="thread-link" href=destination target="_blank" rel="noreferrer">{thread.subject_title.clone()}</a>
            </h3>

            <div class="meta">
                <a class="thread-link repo-link" href=format!("https://github.com/{}", thread.repository) target="_blank" rel="noreferrer">{thread.repository.clone()}</a>
                <span class="meta-separator">"•"</span>
                {thread.sources.iter().map(|source| view! { <SourceBadge source=source.clone() /> }).collect::<Vec<_>>()}
                <span class="meta-separator">"•"</span>
                <span>{thread.updated_at.clone()}</span>
                {if let Some(reason) = thread.reason.clone() {
                    view! { <><span class="meta-separator">"•"</span><span>{reason}</span></> }.into_any()
                } else {
                    ().into_any()
                }}
            </div>

            <div class="row">
                <div class="icon-actions">
                    <form action="/dashboard/actions/open/vscode" method="post" data-async-form>
                        <input type="hidden" name="repository" value=thread.repository.clone() />
                        {thread.pr_url.clone().map(|pr_url| view! { <input type="hidden" name="pr_url" value=pr_url /> })}
                        <button class="btn icon-btn" type="submit" data-loading-label="Opening..." aria-label="Open in VS Code" title="Open in VS Code">{svg_icon(VSCODE_ICON_PATHS)}</button>
                    </form>
                    <form action="/dashboard/actions/open/terminal" method="post" data-async-form>
                        <input type="hidden" name="repository" value=thread.repository.clone() />
                        {thread.pr_url.clone().map(|pr_url| view! { <input type="hidden" name="pr_url" value=pr_url /> })}
                        <button class="btn icon-btn" type="submit" data-loading-label="Opening..." aria-label="Open in Terminal" title="Open in Terminal">{svg_icon(TERMINAL_ICON_PATHS)}</button>
                    </form>
                </div>

            </div>

            <div class="row">
                {if let Some(review) = review_content {
                    view! {
                        <button
                            class=format!("pill {review_tone} review-open")
                            type="button"
                            data-review-content=review
                        >
                            {review_label}
                        </button>
                    }.into_any()
                } else {
                    view! { <button class=format!("pill {review_tone}") type="button" disabled>{review_label}</button> }.into_any()
                }}
                {if can_review {
                    view! {
                        <form action=review_action method="post" data-async-form>
                            <button class="btn" type="submit" data-loading-label="Reviewing...">"Run review"</button>
                        </form>
                    }.into_any()
                } else {
                    ().into_any()
                }}
                {if can_fix {
                    view! {
                        <form action=fix_action method="post" data-async-form>
                            <button class="btn" type="submit" data-loading-label="Fixing...">"Do fixes"</button>
                        </form>
                    }.into_any()
                } else {
                    ().into_any()
                }}
                {if thread.github_thread_id.is_some() || mark_authored_pr {
                    view! {
                        <form action="/dashboard/actions/done" method="post" data-async-form>
                            {thread.github_thread_id.clone().map(|thread_id| view! { <input type="hidden" name="github_thread_id" value=thread_id /> })}
                            {thread.pr_url.clone().map(|pr_url| view! { <input type="hidden" name="pr_url" value=pr_url /> })}
                            <input type="hidden" name="mark_authored_pr" value=mark_authored_pr.to_string() />
                            <button class="btn" type="submit" data-loading-label="Saving...">"Mark done"</button>
                        </form>
                    }.into_any()
                } else {
                    ().into_any()
                }}
            </div>
        </article>
    }
}

#[component]
fn SourceBadge(source: String) -> impl IntoView {
    let label = source_label(&source);
    let icon = source_icon_paths(&source);

    view! {
        <span class="source-badge" title=label aria-label=label>{svg_icon(icon)}</span>
    }
}

fn grouped_threads(threads: &[DashboardThread]) -> Vec<(String, Vec<DashboardThread>)> {
    let mut groups = HashMap::<String, Vec<DashboardThread>>::new();
    for thread in threads {
        groups
            .entry(thread.repository.clone())
            .or_default()
            .push(thread.clone());
    }

    let mut grouped: Vec<_> = groups.into_iter().collect();
    grouped.sort_by(|(repository_a, threads_a), (repository_b, threads_b)| {
        let latest_a = threads_a
            .iter()
            .map(|thread| thread.updated_at.as_str())
            .max();
        let latest_b = threads_b
            .iter()
            .map(|thread| thread.updated_at.as_str())
            .max();
        latest_b
            .cmp(&latest_a)
            .then_with(|| repository_a.cmp(repository_b))
    });
    grouped
}

fn source_label(source: &str) -> &'static str {
    match source {
        "notification" => "Notification",
        "my_pr" => "My PR",
        _ => "Other",
    }
}

fn source_icon_paths(source: &str) -> &'static [&'static str] {
    match source {
        "my_pr" => MY_PR_ICON_PATHS,
        _ => NOTIFICATION_ICON_PATHS,
    }
}

fn thread_state_data(
    subject_type: Option<&str>,
    state: Option<&str>,
) -> (&'static str, &'static [&'static str], &'static str) {
    match state {
        Some("MERGED") => ("title-state-icon merged", PR_MERGED_ICON_PATHS, "Merged"),
        Some("CLOSED") if subject_type == Some("Issue") => (
            "title-state-icon closed",
            ISSUE_CLOSED_ICON_PATHS,
            "Closed issue",
        ),
        Some("CLOSED") => ("title-state-icon closed", PR_CLOSED_ICON_PATHS, "Closed"),
        Some("OPEN") if subject_type == Some("Issue") => {
            ("title-state-icon open", ISSUE_OPEN_ICON_PATHS, "Open issue")
        }
        _ => ("title-state-icon open", PR_OPEN_ICON_PATHS, "Open"),
    }
}

fn review_action_path(thread: &DashboardThread) -> String {
    format!(
        "/dashboard/actions/prs/{}/{}/{}/review",
        thread.pr_owner.clone().unwrap_or_default(),
        thread.pr_repo.clone().unwrap_or_default(),
        thread.pr_number.unwrap_or_default()
    )
}

fn fix_action_path(thread: &DashboardThread) -> String {
    format!(
        "/dashboard/actions/prs/{}/{}/{}/fix",
        thread.pr_owner.clone().unwrap_or_default(),
        thread.pr_repo.clone().unwrap_or_default(),
        thread.pr_number.unwrap_or_default()
    )
}

fn svg_icon(paths: &'static [&'static str]) -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true">
            {paths.iter().map(|path| view! { <path d=*path /> }).collect::<Vec<_>>()}
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_thread(repository: &str, updated_at: &str) -> DashboardThread {
        DashboardThread {
            thread_key: format!("{repository}:{updated_at}"),
            github_thread_id: None,
            sources: vec!["notification".to_string()],
            repository: repository.to_string(),
            pr_owner: None,
            pr_repo: None,
            pr_number: None,
            subject_type: None,
            subject_title: "subject".to_string(),
            subject_url: None,
            issue_state: None,
            reason: None,
            pr_url: None,
            unread: false,
            done: false,
            updated_at: updated_at.to_string(),
            latest_requires_code_changes: None,
            pr_state: None,
            latest_review_content_md: None,
            latest_review_created_at: None,
            latest_review_provider: None,
        }
    }

    #[test]
    fn grouped_threads_orders_repositories_by_latest_updated_at_desc() {
        let threads = vec![
            test_thread("beta/repo", "2026-01-03T00:00:00Z"),
            test_thread("alpha/repo", "2026-01-02T00:00:00Z"),
            test_thread("alpha/repo", "2026-01-01T00:00:00Z"),
        ];

        let groups = grouped_threads(&threads);
        let repositories = groups
            .into_iter()
            .map(|(repository, _)| repository)
            .collect::<Vec<_>>();

        assert_eq!(repositories, vec!["beta/repo", "alpha/repo"]);
    }
}
