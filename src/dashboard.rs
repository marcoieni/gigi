use std::collections::HashMap;

use leptos::prelude::*;

use crate::{
    db::{DashboardThread, DashboardThreadFilters},
    icons::{
        CHECKMARK_ICON, ISSUE_CLOSED_ICON, ISSUE_OPEN_ICON, MAIL_ICON, MY_PR_ICON,
        NOTIFICATION_ICON, PR_CLOSED_ICON, PR_DRAFT_ICON, PR_MERGED_ICON, PR_OPEN_ICON,
        PR_QUEUED_ICON, REFRESH_ICON, TERMINAL_ICON, VSCODE_ICON,
    },
};

#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub filters: DashboardThreadFilters,
    pub threads: Vec<DashboardThread>,
    pub available_repositories: Vec<String>,
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
                <link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>🎤</text></svg>" />
                <link rel="stylesheet" href="/styles.css" />
            </head>
            <body>
                <DashboardRoot snapshot=snapshot.clone() />
            </body>
        </html>
    }
    .to_html()
}

#[component]
fn DashboardRoot(snapshot: DashboardSnapshot) -> impl IntoView {
    let grouped = snapshot.filters.group_by_repository;
    let groups = grouped_threads(&snapshot.threads);
    let hidden_repos = snapshot.filters.hidden_repositories.clone();
    let available_repos = snapshot.available_repositories.clone();

    view! {
        <main class="layout">
            <header class="header">
                <h1>"gigi dashboard"</h1>
                <div class="actions">
                    <span class="status">{snapshot.status_message}</span>
                    <form action="/dashboard/actions/refresh" method="post">
                        <button class="btn icon-btn" type="submit" aria-label="Refresh" title="Refresh">
                            {svg_icon(REFRESH_ICON)}
                        </button>
                    </form>
                    <a
                        class="btn icon-btn header-link"
                        href="https://github.com/notifications"
                        target="_blank"
                        rel="noreferrer"
                        aria-label="Open GitHub notifications"
                        title="Open GitHub notifications"
                    >
                        {svg_icon(MAIL_ICON)}
                    </a>
                </div>
            </header>

            <section class="filters">
                <form action="/dashboard/actions/filters" method="post" onchange="this.requestSubmit()">
                    <div class="filter-row">
                        <fieldset class="filter-group">
                            <legend>"Show"</legend>
                            <FilterCheckbox
                                name="show_notifications"
                                label="Notifications"
                                checked=snapshot.filters.show_notifications
                            />
                            <FilterCheckbox
                                name="show_prs"
                                label="PRs"
                                checked=snapshot.filters.show_prs
                            />
                        </fieldset>
                        <fieldset class="filter-group">
                            <legend>"Status"</legend>
                            <FilterCheckbox
                                name="show_not_done"
                                label="Not done"
                                checked=snapshot.filters.show_not_done
                            />
                            <FilterCheckbox
                                name="show_done"
                                label="Done"
                                checked=snapshot.filters.show_done
                            />
                        </fieldset>
                        <fieldset class="filter-group">
                            <legend>"Display"</legend>
                            <FilterCheckbox
                                name="group_by_repository"
                                label="Group by repository"
                                checked=snapshot.filters.group_by_repository
                            />
                        </fieldset>
                    </div>
                </form>

                {if available_repos.is_empty() {
                    ().into_any()
                } else {
                    let repos = available_repos.clone();
                    let hidden = hidden_repos.clone();
                    let active_count = repos.len().saturating_sub(hidden.len());
                    let total_count = repos.len();
                    let badge_label = if active_count == total_count {
                        "All".to_string()
                    } else {
                        format!("{active_count}/{total_count}")
                    };

                    view! {
                        <details class="repo-dropdown">
                            <summary class="btn repo-dropdown-toggle">
                                "Repositories "
                                <span class="repo-badge">{badge_label}</span>
                            </summary>
                            <div class="repo-dropdown-panel">
                                <form
                                    action="/dashboard/actions/repo-filter"
                                    method="post"
                                    onchange="this.requestSubmit()"
                                >
                                    <div class="repo-filter-form">
                                        {repos
                                            .into_iter()
                                            .enumerate()
                                            .map(|(index, repo)| {
                                                let checked = !hidden.contains(&repo);
                                                let value = repo.clone();
                                                let name = format!("repo:{index}:{repo}");
                                                view! {
                                                    <label class="repo-dropdown-option">
                                                        <input type="checkbox" name=name value=value checked=checked />
                                                        <span>{repo}</span>
                                                    </label>
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </div>
                                </form>
                            </div>
                        </details>
                    }
                        .into_any()
                }}
            </section>

            <section>
                {if snapshot.threads.is_empty() {
                    view! {
                        <div class="threads">
                            <article class="thread">
                                <h3>"Nothing to review"</h3>
                                <p class="meta">"No cards match the current filters."</p>
                            </article>
                        </div>
                    }
                        .into_any()
                } else if grouped {
                    let available_repositories = snapshot.available_repositories.clone();
                    view! {
                        <div class="threads grouped">
                            {groups
                                .into_iter()
                                .map(|(repository, threads)| {
                                    let can_hide = available_repositories.len() > 1;
                                    view! { <RepositorySection repository threads can_hide /> }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    }
                        .into_any()
                } else {
                    view! {
                        <div class="threads">
                            {snapshot
                                .threads
                                .into_iter()
                                .map(|thread| view! { <ThreadCard thread /> })
                                .collect::<Vec<_>>()}
                        </div>
                    }
                        .into_any()
                }}
            </section>
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
fn RepositorySection(
    repository: String,
    threads: Vec<DashboardThread>,
    can_hide: bool,
) -> impl IntoView {
    let repo_link = format!("https://github.com/{repository}");
    let hide_repository = repository.clone();

    view! {
        <section class="repo-group">
            <header class="repo-group-header">
                <h2 class="repo-group-title">
                    <a class="thread-link repo-link" href=repo_link target="_blank" rel="noreferrer">
                        {repository}
                    </a>
                </h2>
                {if can_hide {
                    view! {
                        <form action="/dashboard/actions/repositories/hide" method="post">
                            <input type="hidden" name="repository" value=hide_repository />
                            <button class="btn btn-subtle" type="submit">"Hide"</button>
                        </form>
                    }
                        .into_any()
                } else {
                    ().into_any()
                }}
            </header>
            <div class="threads">
                {threads
                    .into_iter()
                    .map(|thread| view! { <ThreadCard thread /> })
                    .collect::<Vec<_>>()}
            </div>
        </section>
    }
}

#[component]
fn ThreadCard(thread: DashboardThread) -> impl IntoView {
    let repository = thread.repository.clone();
    let pr_url = thread.pr_url.clone();
    let github_thread_id = thread.github_thread_id.clone();
    let vscode_repository = repository.clone();
    let terminal_repository = repository.clone();
    let repo_link = format!("https://github.com/{repository}");
    let vscode_pr_url = pr_url.clone();
    let terminal_pr_url = pr_url.clone();
    let done_pr_url = pr_url.clone();
    let destination = thread
        .subject_url
        .clone()
        .or_else(|| pr_url.clone())
        .unwrap_or_else(|| format!("https://github.com/{repository}"));
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
    let review_target = review_target(&thread);
    let review_action = review_action_path(&thread);
    let fix_action = fix_action_path(&thread);
    let can_fix = review_target.is_some() && thread.latest_requires_code_changes == Some(true);
    let mark_authored_pr = thread.sources.iter().any(|source| source == "my_pr");
    let (state_icon_class, state_icon_paths, state_icon_label) = thread_state_data(
        thread.subject_type.as_deref(),
        thread.pr_state.as_deref().or(thread.issue_state.as_deref()),
        thread.pr_merge_queue_state.as_deref(),
        thread.is_draft,
    );

    view! {
        <article class="thread">
            <h3>
                <span class=state_icon_class aria-label=state_icon_label title=state_icon_label>
                    {svg_icon(state_icon_paths)}
                </span>
                <a class="thread-link" href=destination target="_blank" rel="noreferrer">
                    {thread.subject_title.clone()}
                </a>
            </h3>

            <div class="meta">
                <a
                    class="thread-link repo-link"
                    href=repo_link
                    target="_blank"
                    rel="noreferrer"
                >
                    {repository.clone()}
                </a>
                <span class="meta-separator">"•"</span>
                {thread
                    .sources
                    .iter()
                    .map(|source| view! { <SourceBadge source=source.clone() /> })
                    .collect::<Vec<_>>()}
                <span class="meta-separator">"•"</span>
                {
                    let (relative, absolute) = format_timestamp(&thread.updated_at);
                    view! { <span title=absolute>{relative}</span> }
                }
                {if let Some(reason) = thread.reason.clone() {
                    view! { <><span class="meta-separator">"•"</span><span>{reason}</span></> }.into_any()
                } else {
                    ().into_any()
                }}
                {
                    let non_bot_participants: Vec<_> = thread
                        .participants
                        .iter()
                        .filter(|participant| !participant.login.ends_with("[bot]"))
                        .collect();
                    if non_bot_participants.is_empty() {
                        ().into_any()
                    } else {
                        let avatars = non_bot_participants
                            .into_iter()
                            .take(5)
                            .map(|participant| {
                                let alt = participant.login.clone();
                                let src = if participant.avatar_url.contains('?') {
                                    format!("{}&s=40", participant.avatar_url)
                                } else {
                                    format!("{}?s=40", participant.avatar_url)
                                };
                                let profile = format!("https://github.com/{}", participant.login);
                                view! {
                                    <a class="avatar-link" href=profile target="_blank" rel="noreferrer" title=alt.clone()>
                                        <img class="avatar" src=src alt=alt.clone() loading="lazy" />
                                    </a>
                                }
                            })
                            .collect::<Vec<_>>();
                        view! {
                            <span class="meta-separator">"•"</span>
                            <span class="avatar-stack">{avatars}</span>
                        }
                            .into_any()
                    }
                }
            </div>

            {render_review_section(
                review_content,
                review_tone,
                review_label,
                review_target.is_some(),
                can_fix,
            )}

            <div class="row">
                {if let Some((owner, repo, number)) = review_target.clone() {
                    view! {
                        <form action=review_action method="post">
                            <input type="hidden" name="owner" value=owner />
                            <input type="hidden" name="repo" value=repo />
                            <input type="hidden" name="number" value=number.to_string() />
                            <button class="btn" type="submit">"Review"</button>
                        </form>
                    }
                        .into_any()
                } else {
                    ().into_any()
                }}
                <div class="icon-actions">
                    <form action="/dashboard/actions/open/vscode" method="post">
                        <input type="hidden" name="repository" value=vscode_repository />
                        {vscode_pr_url
                            .clone()
                            .map(|pr_url| view! { <input type="hidden" name="pr_url" value=pr_url /> })}
                        <button class="btn icon-btn" type="submit" aria-label="Open in VS Code" title="Open in VS Code">
                            {svg_icon(VSCODE_ICON)}
                        </button>
                    </form>
                    <form action="/dashboard/actions/open/terminal" method="post">
                        <input type="hidden" name="repository" value=terminal_repository />
                        {terminal_pr_url
                            .clone()
                            .map(|pr_url| view! { <input type="hidden" name="pr_url" value=pr_url /> })}
                        <button class="btn icon-btn" type="submit" aria-label="Open in Terminal" title="Open in Terminal">
                            {svg_icon(TERMINAL_ICON)}
                        </button>
                    </form>
                </div>
                {if github_thread_id.is_some() || mark_authored_pr {
                    view! {
                        <form action="/dashboard/actions/done" method="post">
                            {github_thread_id
                                .clone()
                                .map(|thread_id| view! { <input type="hidden" name="github_thread_id" value=thread_id /> })}
                            {done_pr_url
                                .clone()
                                .map(|pr_url| view! { <input type="hidden" name="pr_url" value=pr_url /> })}
                            {mark_authored_pr
                                .then(|| view! { <input type="hidden" name="mark_authored_pr" value="true" /> })}
                            <button class="btn icon-btn" type="submit" aria-label="Mark done" title="Mark done">
                                {svg_icon(CHECKMARK_ICON)}
                            </button>
                        </form>
                    }
                        .into_any()
                } else {
                    ().into_any()
                }}
            </div>

            {if can_fix {
                if let Some((owner, repo, number)) = review_target {
                    view! {
                        <div class="review-actions">
                            <form action=fix_action method="post">
                                <input type="hidden" name="owner" value=owner />
                                <input type="hidden" name="repo" value=repo />
                                <input type="hidden" name="number" value=number.to_string() />
                                <button class="btn" type="submit">"Fix"</button>
                            </form>
                        </div>
                    }
                        .into_any()
                } else {
                    ().into_any()
                }
            } else {
                ().into_any()
            }}
        </article>
    }
}

fn render_review_section(
    review_content: Option<String>,
    review_tone: &'static str,
    review_label: &'static str,
    can_review: bool,
    can_fix: bool,
) -> impl IntoView {
    if !can_review {
        return ().into_any();
    }

    match review_content {
        Some(review) => {
            let cleaned_review = clean_review_text(&review);
            view! {
                <details class="review-details">
                    <summary class="review-summary">
                        <span class=format!("pill {review_tone}")>{review_label}</span>
                    </summary>
                    <div class="review-body">
                        <pre>{cleaned_review}</pre>
                        {if can_fix {
                            view! {
                                <p class="review-note">
                                    "Run "
                                    <code>"Fix"</code>
                                    " below to ask gigi to address the current review."
                                </p>
                            }
                                .into_any()
                        } else {
                            ().into_any()
                        }}
                    </div>
                </details>
            }
            .into_any()
        }
        None => view! {
            <div class="review-strip">
                <span class=format!("pill {review_tone}")>{review_label}</span>
            </div>
        }
        .into_any(),
    }
}

#[component]
fn SourceBadge(source: String) -> impl IntoView {
    let label = source_label(&source);
    let icon = source_icon(&source);

    view! {
        <span class="source-badge" title=label aria-label=label>{svg_icon(icon)}</span>
    }
}

fn review_target(thread: &DashboardThread) -> Option<(String, String, i64)> {
    match (
        thread.pr_owner.clone(),
        thread.pr_repo.clone(),
        thread.pr_number,
    ) {
        (Some(owner), Some(repo), Some(number)) => Some((owner, repo, number)),
        _ => None,
    }
}

fn clean_review_text(raw: &str) -> String {
    let mut cleaned = raw
        .lines()
        .filter(|line| {
            !matches!(
                line.trim(),
                "REQUIRES_CODE_CHANGES: YES" | "REQUIRES_CODE_CHANGES: NO"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    while cleaned.contains("\n\n\n") {
        cleaned = cleaned.replace("\n\n\n", "\n\n");
    }

    cleaned.trim().to_string()
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

fn format_timestamp(raw: &str) -> (String, String) {
    use chrono::{NaiveDateTime, Utc};

    let Ok(dt) = raw.parse::<chrono::DateTime<Utc>>().or_else(|_| {
        NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S").map(|naive| naive.and_utc())
    }) else {
        return (raw.to_string(), raw.to_string());
    };
    let absolute = dt.format("%b %d, %Y at %H:%M").to_string();
    let now = Utc::now();
    let duration = now.signed_duration_since(dt);

    let relative = if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_hours() < 1 {
        let minutes = duration.num_minutes();
        format!("{minutes}m ago")
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        format!("{hours}h ago")
    } else if duration.num_days() < 7 {
        let days = duration.num_days();
        format!("{days}d ago")
    } else {
        return (absolute.clone(), absolute);
    };

    (relative, absolute)
}

fn source_label(source: &str) -> &'static str {
    match source {
        "notification" => "Notification",
        "my_pr" => "My PR",
        _ => "Other",
    }
}

fn source_icon(source: &str) -> &'static str {
    match source {
        "my_pr" => MY_PR_ICON,
        _ => NOTIFICATION_ICON,
    }
}

fn thread_state_data(
    subject_type: Option<&str>,
    state: Option<&str>,
    merge_queue_state: Option<&str>,
    is_draft: bool,
) -> (&'static str, &'static str, &'static str) {
    match state {
        Some("MERGED") => ("title-state-icon merged", PR_MERGED_ICON, "Merged"),
        Some("CLOSED") if subject_type == Some("Issue") => {
            ("title-state-icon closed", ISSUE_CLOSED_ICON, "Closed issue")
        }
        Some("CLOSED") => ("title-state-icon closed", PR_CLOSED_ICON, "Closed"),
        Some("OPEN") if subject_type == Some("Issue") => {
            ("title-state-icon open", ISSUE_OPEN_ICON, "Open issue")
        }
        Some("OPEN") if merge_queue_state.is_some() => (
            "title-state-icon queued",
            PR_QUEUED_ICON,
            merge_queue_state_label(merge_queue_state.unwrap_or_default()),
        ),
        _ if is_draft => ("title-state-icon draft", PR_DRAFT_ICON, "Draft"),
        _ => ("title-state-icon open", PR_OPEN_ICON, "Open"),
    }
}

fn merge_queue_state_label(state: &str) -> &'static str {
    match state {
        "AWAITING_CHECKS" => "Awaiting checks",
        "MERGEABLE" => "Mergeable in queue",
        "UNMERGEABLE" => "Blocked in queue",
        "LOCKED" => "Queue locked",
        _ => "Queued",
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

fn svg_icon(inner: &'static str) -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" inner_html=inner>
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
            pr_merge_queue_state: None,
            latest_review_content_md: None,
            latest_review_created_at: None,
            latest_review_provider: None,
            is_draft: false,
            participants: Vec::new(),
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

    #[test]
    fn thread_state_data_marks_open_prs_in_merge_queue_as_queued() {
        let (_, _, label) =
            thread_state_data(Some("PullRequest"), Some("OPEN"), Some("QUEUED"), false);

        assert_eq!(label, "Queued");
    }

    #[test]
    fn clean_review_text_strips_requires_code_changes_marker() {
        let raw = "Summary\n\nREQUIRES_CODE_CHANGES: YES\n\nMore detail";

        assert_eq!(clean_review_text(raw), "Summary\n\nMore detail");
    }

    #[test]
    fn render_page_smoke_test() {
        let html = render_page(&DashboardSnapshot {
            filters: DashboardThreadFilters::default(),
            threads: Vec::new(),
            available_repositories: Vec::new(),
            status_message: "Ready".to_string(),
        });

        assert!(html.contains("gigi dashboard"));
        assert!(html.contains("/dashboard/actions/refresh"));
    }
}
