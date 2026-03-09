use leptos::prelude::*;
use leptos_meta::*;

use crate::dashboard_types::*;

/// Server functions — these run on the server and are callable from the client.
#[cfg(feature = "ssr")]
mod server_impl {
    pub fn app_state() -> crate::serve::AppState {
        leptos::prelude::expect_context::<crate::serve::AppState>()
    }
}

#[server(GetThreads, "/api")]
pub async fn get_threads(filters: DashboardFilters) -> Result<Vec<DashboardThread>, ServerFnError> {
    let state = server_impl::app_state();
    let db_filters = crate::db::DashboardThreadFilters {
        show_notifications: filters.show_notifications,
        show_prs: filters.show_prs,
        show_done: filters.show_done,
        show_not_done: filters.show_not_done,
        group_by_repository: filters.group_by_repository,
    };
    let threads = state.db.list_dashboard_threads_with_filters(db_filters)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    // Convert from db types to shared types
    Ok(threads.into_iter().map(|t| DashboardThread {
        thread_key: t.thread_key,
        github_thread_id: t.github_thread_id,
        sources: t.sources,
        repository: t.repository,
        subject_type: t.subject_type,
        subject_title: t.subject_title,
        subject_url: t.subject_url,
        issue_state: t.issue_state,
        reason: t.reason,
        pr_url: t.pr_url,
        unread: t.unread,
        done: t.done,
        updated_at: t.updated_at,
        latest_requires_code_changes: t.latest_requires_code_changes,
        pr_state: t.pr_state,
    }).collect())
}

#[server(GetFilters, "/api")]
pub async fn get_filters() -> Result<DashboardFilters, ServerFnError> {
    let state = server_impl::app_state();
    let f = state.db.dashboard_thread_filters()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(DashboardFilters {
        show_notifications: f.show_notifications,
        show_prs: f.show_prs,
        show_done: f.show_done,
        show_not_done: f.show_not_done,
        group_by_repository: f.group_by_repository,
    })
}

#[server(SaveFilters, "/api")]
pub async fn save_filters(filters: DashboardFilters) -> Result<(), ServerFnError> {
    let state = server_impl::app_state();
    state.db.set_dashboard_thread_filters(crate::db::DashboardThreadFilters {
        show_notifications: filters.show_notifications,
        show_prs: filters.show_prs,
        show_done: filters.show_done,
        show_not_done: filters.show_not_done,
        group_by_repository: filters.group_by_repository,
    }).map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(RefreshFromGitHub, "/api")]
pub async fn refresh_from_github() -> Result<PollStats, ServerFnError> {
    let state = server_impl::app_state();
    let stats = state.poll_once_from_dashboard().await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(PollStats {
        notifications_fetched: stats.notifications_fetched,
        authored_prs_fetched: stats.authored_prs_fetched,
        prs_seen: stats.prs_seen,
        reviews_run: stats.reviews_run,
    })
}

#[server(GetLatestReview, "/api")]
pub async fn get_latest_review(owner: String, repo: String, number: i64) -> Result<Option<StoredReview>, ServerFnError> {
    let state = server_impl::app_state();
    let review = state.db.latest_review_for_pr(&owner, &repo, number)
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(review.map(|r| StoredReview {
        id: r.id,
        pr_url: r.pr_url,
        provider: r.provider,
        model: r.model,
        requires_code_changes: r.requires_code_changes,
        content_md: r.content_md,
        created_at: r.created_at,
    }))
}

#[server(RunReview, "/api")]
pub async fn run_review(owner: String, repo: String, number: i64) -> Result<(), ServerFnError> {
    let state = server_impl::app_state();
    state.run_review(owner, repo, number).await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(RunFix, "/api")]
pub async fn run_fix(owner: String, repo: String, number: i64) -> Result<FixResponse, ServerFnError> {
    let state = server_impl::app_state();
    let output = state.run_fix(owner, repo, number).await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(FixResponse { output })
}

#[server(MarkDone, "/api")]
pub async fn mark_done(payload: MarkDonePayload) -> Result<(), ServerFnError> {
    let state = server_impl::app_state();
    state.mark_done(crate::serve::MarkDoneRequest {
        github_thread_id: payload.github_thread_id,
        pr_url: payload.pr_url,
        mark_authored_pr: payload.mark_authored_pr,
    }).await.map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(OpenVscode, "/api")]
pub async fn open_vscode(request: OpenProjectRequest) -> Result<(), ServerFnError> {
    let state = server_impl::app_state();
    state.open_in_vscode(request.repository, request.pr_url).await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(OpenTerminal, "/api")]
pub async fn open_terminal(request: OpenProjectRequest) -> Result<(), ServerFnError> {
    let state = server_impl::app_state();
    state.open_in_terminal(request.repository, request.pr_url).await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

// ─── Leptos Components ───

fn parse_pr_url(pr_url: &str) -> Option<(String, String, i64)> {
    let re = pr_url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = re.split('/').collect();
    if parts.len() < 4 || parts[2] != "pull" {
        return None;
    }
    let number: i64 = parts[3].parse().ok()?;
    Some((parts[0].to_string(), parts[1].to_string(), number))
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/gigi.css"/>
        <Title text="gigi dashboard"/>
        <Dashboard/>
    }
}

#[component]
fn Dashboard() -> impl IntoView {
    let (filters, set_filters) = signal(DashboardFilters::default());
    let (status, set_status) = signal("Loading...".to_string());
    let (review_modal_content, set_review_modal_content) = signal(None::<String>);
    let (refresh_version, set_refresh_version) = signal(0_u32);

    // Load initial filters from server
    let filters_resource = Resource::new(|| (), async move |_| {
        get_filters().await.unwrap_or_default()
    });

    // Apply loaded filters
    Effect::new(move || {
        if let Some(f) = filters_resource.get() {
            set_filters.set(f);
        }
    });

    // Load threads reactively — re-fetches when filters or refresh_version change
    let threads = Resource::new(
        move || (filters.get(), refresh_version.get()),
        async move |(f, _)| {
            get_threads(f).await.unwrap_or_default()
        },
    );

    // SSE event listener for auto-refresh
    #[cfg(feature = "hydrate")]
    {
        Effect::new(move || {
            use wasm_bindgen::prelude::*;
            use web_sys::EventSource;

            let es = EventSource::new("/api/events").ok();
            if let Some(es) = es {
                let set_status_clone = set_status;
                let set_refresh = set_refresh_version;

                let on_poll = Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                    if let Some(data) = e.data().as_string() {
                        if let Ok(stats) = serde_json::from_str::<PollStats>(&data) {
                            set_status_clone.set(format!(
                                "Auto-refreshed: notifications={}, my_prs={}, reviews={}",
                                stats.notifications_fetched, stats.authored_prs_fetched, stats.reviews_run
                            ));
                        }
                    }
                    set_refresh.update(|v| *v += 1);
                });
                let _ = es.add_event_listener_with_callback("poll_complete", on_poll.as_ref().unchecked_ref());
                on_poll.forget();

                let set_refresh2 = set_refresh_version;
                let on_review = Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |_: web_sys::MessageEvent| {
                    set_refresh2.update(|v| *v += 1);
                });
                let _ = es.add_event_listener_with_callback("review_complete", on_review.as_ref().unchecked_ref());
                on_review.forget();
            }
        });
    }

    let on_refresh = move |_| {
        set_status.set("Refreshing from GitHub...".to_string());
        leptos::task::spawn_local(async move {
            match refresh_from_github().await {
                Ok(stats) => {
                    set_status.set(format!(
                        "Refreshed: notifications={}, my_prs={}, reviews={}",
                        stats.notifications_fetched, stats.authored_prs_fetched, stats.reviews_run
                    ));
                    set_refresh_version.update(|v| *v += 1);
                }
                Err(e) => set_status.set(format!("Refresh failed: {e}")),
            }
        });
    };

    let on_filter_change = move |updater: Box<dyn Fn(&mut DashboardFilters)>| {
        set_filters.update(move |f| updater(f));
        let f = filters.get_untracked();
        leptos::task::spawn_local(async move {
            drop(save_filters(f).await);
        });
    };

    view! {
        <main class="layout">
            <header class="header">
                <h1>"gigi dashboard"</h1>
                <div class="actions">
                    <button class="btn" on:click=on_refresh>"Refresh"</button>
                    <span class="status">{move || status.get()}</span>
                    <a class="btn icon-btn header-link"
                       href="https://github.com/notifications"
                       target="_blank" rel="noreferrer"
                       aria-label="Open GitHub notifications"
                       title="Open GitHub notifications">
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M4.5 7.5A2.5 2.5 0 0 1 7 5h10a2.5 2.5 0 0 1 2.5 2.5v9A2.5 2.5 0 0 1 17 19H7a2.5 2.5 0 0 1-2.5-2.5v-9Z"/>
                            <path d="m6.5 8.5 5.5 4 5.5-4"/>
                            <path d="M8 15.5h8"/>
                        </svg>
                    </a>
                </div>
            </header>

            <FilterBar filters=filters on_change=on_filter_change/>

            <section>
                <Suspense fallback=move || view! { <p>"Loading threads..."</p> }>
                    {move || {
                        threads.get().map(|thread_list| {
                            let group = filters.get().group_by_repository;
                            if thread_list.is_empty() {
                                view! { <p class="empty">"No threads to display."</p> }.into_any()
                            } else if group {
                                view! { <GroupedThreads threads=thread_list set_status=set_status set_review_modal_content=set_review_modal_content set_refresh_version=set_refresh_version/> }.into_any()
                            } else {
                                view! { <FlatThreads threads=thread_list set_status=set_status set_review_modal_content=set_review_modal_content set_refresh_version=set_refresh_version/> }.into_any()
                            }
                        })
                    }}
                </Suspense>
            </section>

            <ReviewModal content=review_modal_content set_content=set_review_modal_content/>
        </main>
    }
}

#[component]
fn FilterBar(
    filters: ReadSignal<DashboardFilters>,
    on_change: impl Fn(Box<dyn Fn(&mut DashboardFilters)>) + 'static + Copy,
) -> impl IntoView {
    view! {
        <section class="filters" aria-label="Dashboard filters">
            <fieldset class="filter-group">
                <legend>"Show"</legend>
                <label class="filter-option">
                    <input type="checkbox"
                        prop:checked=move || filters.get().show_notifications
                        on:change=move |_| on_change(Box::new(|f| f.show_notifications = !f.show_notifications))
                    />
                    <span>"Notifications"</span>
                </label>
                <label class="filter-option">
                    <input type="checkbox"
                        prop:checked=move || filters.get().show_prs
                        on:change=move |_| on_change(Box::new(|f| f.show_prs = !f.show_prs))
                    />
                    <span>"PRs"</span>
                </label>
            </fieldset>
            <fieldset class="filter-group">
                <legend>"Status"</legend>
                <label class="filter-option">
                    <input type="checkbox"
                        prop:checked=move || filters.get().show_not_done
                        on:change=move |_| on_change(Box::new(|f| f.show_not_done = !f.show_not_done))
                    />
                    <span>"Not done"</span>
                </label>
                <label class="filter-option">
                    <input type="checkbox"
                        prop:checked=move || filters.get().show_done
                        on:change=move |_| on_change(Box::new(|f| f.show_done = !f.show_done))
                    />
                    <span>"Done"</span>
                </label>
            </fieldset>
            <fieldset class="filter-group">
                <legend>"Display"</legend>
                <label class="filter-option">
                    <input type="checkbox"
                        prop:checked=move || filters.get().group_by_repository
                        on:change=move |_| on_change(Box::new(|f| f.group_by_repository = !f.group_by_repository))
                    />
                    <span>"Group by repository"</span>
                </label>
            </fieldset>
        </section>
    }
}

#[component]
fn GroupedThreads(
    threads: Vec<DashboardThread>,
    set_status: WriteSignal<String>,
    set_review_modal_content: WriteSignal<Option<String>>,
    set_refresh_version: WriteSignal<u32>,
) -> impl IntoView {
    let mut groups: Vec<(String, Vec<DashboardThread>)> = Vec::new();
    let mut group_map = std::collections::HashMap::new();
    for thread in threads {
        let repo = thread.repository.clone();
        let idx = *group_map.entry(repo.clone()).or_insert_with(|| {
            groups.push((repo, Vec::new()));
            groups.len() - 1
        });
        groups[idx].1.push(thread);
    }

    view! {
        <div class="threads grouped">
            {groups.into_iter().map(|(repo, repo_threads)| {
                let count = repo_threads.len();
                let count_label = if count == 1 { "1 item".to_string() } else { format!("{count} items") };
                let repo_url = format!("https://github.com/{repo}");
                view! {
                    <section class="repo-group">
                        <header class="repo-group-header">
                            <h2 class="repo-group-title">
                                <a class="thread-link repo-link"
                                   href=repo_url
                                   target="_blank" rel="noreferrer">
                                    {repo}
                                </a>
                            </h2>
                            <span class="repo-group-count">{count_label}</span>
                        </header>
                        <div class="threads repo-group-threads">
                            {repo_threads.into_iter().map(|t| view! {
                                <ThreadCard thread=t set_status=set_status set_review_modal_content=set_review_modal_content set_refresh_version=set_refresh_version/>
                            }).collect_view()}
                        </div>
                    </section>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn FlatThreads(
    threads: Vec<DashboardThread>,
    set_status: WriteSignal<String>,
    set_review_modal_content: WriteSignal<Option<String>>,
    set_refresh_version: WriteSignal<u32>,
) -> impl IntoView {
    view! {
        <div class="threads">
            {threads.into_iter().map(|t| view! {
                <ThreadCard thread=t set_status=set_status set_review_modal_content=set_review_modal_content set_refresh_version=set_refresh_version/>
            }).collect_view()}
        </div>
    }
}

#[component]
fn ThreadCard(
    thread: DashboardThread,
    set_status: WriteSignal<String>,
    set_review_modal_content: WriteSignal<Option<String>>,
    set_refresh_version: WriteSignal<u32>,
) -> impl IntoView {
    let (reviewing, set_reviewing) = signal(false);
    let (fixing, set_fixing) = signal(false);
    let (marking_done, set_marking_done) = signal(false);

    let title_href = thread.subject_url.clone().or_else(|| thread.pr_url.clone());
    let has_pr = thread.pr_url.is_some();
    let has_review = thread.latest_requires_code_changes.is_some();
    let needs_changes = thread.latest_requires_code_changes == Some(true);
    let can_mark_done = thread.github_thread_id.is_some() || thread.sources.contains(&"my_pr".to_string());
    let is_done = thread.done;

    let state_icon = thread_state_icon(
        thread.subject_type.clone(),
        thread.pr_state.clone().or_else(|| thread.sources.contains(&"my_pr".to_string()).then(|| "OPEN".to_string())),
        thread.issue_state.clone(),
    );
    let pr_url_for_review = thread.pr_url.clone();
    let pr_url_for_run = thread.pr_url.clone();
    let pr_url_for_fix = thread.pr_url.clone();
    let thread_for_done = thread.clone();
    let thread_for_vscode = thread.clone();
    let thread_for_terminal = thread.clone();

    let on_open_review = move |_| {
        if let Some(ref pr_url) = pr_url_for_review {
            let pr_url = pr_url.clone();
            let set_modal = set_review_modal_content;
            leptos::task::spawn_local(async move {
                if let Some((owner, repo, number)) = parse_pr_url(&pr_url) {
                    match get_latest_review(owner, repo, number).await {
                        Ok(Some(review)) => set_modal.set(Some(review.content_md)),
                        Ok(None) => set_modal.set(Some("No review stored yet.".to_string())),
                        Err(e) => set_modal.set(Some(format!("Error: {e}"))),
                    }
                }
            });
        }
    };

    let on_run_review = move |_| {
        if let Some(ref pr_url) = pr_url_for_run {
            let pr_url = pr_url.clone();
            set_reviewing.set(true);
            set_status.set("Running review...".to_string());
            leptos::task::spawn_local(async move {
                if let Some((owner, repo, number)) = parse_pr_url(&pr_url) {
                    match run_review(owner, repo, number).await {
                        Ok(()) => {
                            set_status.set("Review completed".to_string());
                            set_refresh_version.update(|v| *v += 1);
                        }
                        Err(e) => set_status.set(format!("Review failed: {e}")),
                    }
                }
                set_reviewing.set(false);
            });
        }
    };

    let on_fix = move |_| {
        if let Some(ref pr_url) = pr_url_for_fix {
            let pr_url = pr_url.clone();
            set_fixing.set(true);
            set_status.set("Running fixes...".to_string());
            leptos::task::spawn_local(async move {
                if let Some((owner, repo, number)) = parse_pr_url(&pr_url) {
                    match run_fix(owner, repo, number).await {
                        Ok(_) => {
                            set_status.set("Fix run completed".to_string());
                            set_refresh_version.update(|v| *v += 1);
                        }
                        Err(e) => set_status.set(format!("Fix run failed: {e}")),
                    }
                }
                set_fixing.set(false);
            });
        }
    };

    let on_mark_done = move |_| {
        let t = thread_for_done.clone();
        set_marking_done.set(true);
        set_status.set("Marking done...".to_string());
        leptos::task::spawn_local(async move {
            let mark_authored = t.sources.contains(&"my_pr".to_string());
            match mark_done(MarkDonePayload {
                github_thread_id: t.github_thread_id.clone(),
                pr_url: t.pr_url.clone(),
                mark_authored_pr: mark_authored,
            }).await {
                Ok(()) => {
                    set_status.set("Marked done".to_string());
                    set_refresh_version.update(|v| *v += 1);
                }
                Err(e) => set_status.set(format!("Mark done failed: {e}")),
            }
            set_marking_done.set(false);
        });
    };

    let on_open_vscode = move |_| {
        let t = thread_for_vscode.clone();
        set_status.set("Opening VS Code...".to_string());
        leptos::task::spawn_local(async move {
            match open_vscode(OpenProjectRequest {
                repository: t.repository.clone(),
                pr_url: t.pr_url.clone(),
            }).await {
                Ok(()) => set_status.set("VS Code opened".to_string()),
                Err(e) => set_status.set(format!("Open failed: {e}")),
            }
        });
    };

    let on_open_terminal = move |_| {
        let t = thread_for_terminal.clone();
        set_status.set("Opening Terminal...".to_string());
        leptos::task::spawn_local(async move {
            match open_terminal(OpenProjectRequest {
                repository: t.repository.clone(),
                pr_url: t.pr_url.clone(),
            }).await {
                Ok(()) => set_status.set("Terminal opened".to_string()),
                Err(e) => set_status.set(format!("Open failed: {e}")),
            }
        });
    };

    let has_title_href = title_href.is_some();
    let title_text = thread.subject_title.clone();
    let title_text2 = thread.subject_title.clone();

    let review_pill_class = if !has_review {
        "pill pending"
    } else if needs_changes {
        "pill unsafe"
    } else {
        "pill safe"
    };
    let review_pill_text = if !has_review {
        "No review"
    } else if needs_changes {
        "Fixes needed"
    } else {
        "Safe"
    };

    view! {
        <article class="thread">
            <h3>
                {state_icon}
                {title_href.map(|href| view! {
                    <a class="thread-link" href=href target="_blank" rel="noreferrer">
                        {title_text.clone()}
                    </a>
                })}
                {(!has_title_href).then(|| title_text2.clone())}
            </h3>
            <p class="meta">
                <a class="thread-link repo-link"
                   href=format!("https://github.com/{}", thread.repository)
                   target="_blank" rel="noreferrer">
                    {thread.repository.clone()}
                </a>
                <span class="meta-separator" aria-hidden="true">"•"</span>
                {thread.sources.iter().map(|s| source_label(s)).collect::<Vec<_>>().join(", ")}
                <span class="meta-separator" aria-hidden="true">"•"</span>
                {thread.updated_at.clone()}
            </p>
            <div class="row">
                <div class="icon-actions">
                    <button class="btn icon-btn" title="Open in VS Code" on:click=on_open_vscode>
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M9 8 5 12l4 4"/>
                            <path d="m15 8 4 4-4 4"/>
                            <path d="M14 6 10 18"/>
                        </svg>
                    </button>
                    <button class="btn icon-btn" title="Open in Terminal" on:click=on_open_terminal>
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M4 5.5h16a1.5 1.5 0 0 1 1.5 1.5v10A1.5 1.5 0 0 1 20 18.5H4A1.5 1.5 0 0 1 2.5 17V7A1.5 1.5 0 0 1 4 5.5Z"/>
                            <path d="m6.4 9 2.6 2.3-2.6 2.3"/>
                            <path d="M11.7 13.8h4.9"/>
                        </svg>
                    </button>
                </div>

                {has_pr.then(|| view! {
                    <button class=review_pill_class on:click=on_open_review>
                        {review_pill_text}
                    </button>
                    <button class="btn"
                        disabled=move || reviewing.get() || fixing.get()
                        on:click=on_run_review>
                        {move || if reviewing.get() { "Reviewing..." } else if has_review { "Re-review" } else { "Review now" }}
                    </button>
                })}

                {(has_pr && needs_changes).then(|| view! {
                    <button class="btn"
                        disabled=move || fixing.get() || reviewing.get()
                        on:click=on_fix>
                        {move || if fixing.get() { "Fixing..." } else { "Do fixes" }}
                    </button>
                })}

                {can_mark_done.then(|| view! {
                    <button class="btn icon-btn"
                        title=if is_done { "Done" } else { "Mark done" }
                        disabled=move || is_done || marking_done.get()
                        on:click=on_mark_done>
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="m5 12.5 4 4 10-10"/>
                        </svg>
                    </button>
                })}

                {thread.unread.then(|| view! {
                    <span class="unread-dot" aria-label="Unread" title="Unread"/>
                })}
            </div>
        </article>
    }
}

fn thread_state_icon(
    subject_type: Option<String>,
    pr_state: Option<String>,
    issue_state: Option<String>,
) -> Option<impl IntoView + 'static> {
    match (subject_type.as_deref(), pr_state.as_deref()) {
        (Some("PullRequest"), Some("MERGED")) => Some(view! {
            <span class="title-state-icon pull-request merged" title="Merged pull request">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z"/>
                    <path d="M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z"/>
                    <path d="M18 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z"/>
                    <path d="M8 15V9.5a3 3 0 0 1 3-3h2"/>
                    <path d="M15.5 10.5V15"/>
                </svg>
            </span>
        }.into_any()),
        (Some("PullRequest"), Some("CLOSED")) => Some(view! {
            <span class="title-state-icon pull-request closed" title="Closed pull request">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z"/>
                    <path d="M5.5 15V8.5"/>
                    <path d="M14.5 8.5 20 14"/>
                    <path d="M20 8.5 14.5 14"/>
                </svg>
            </span>
        }.into_any()),
        (Some("PullRequest"), Some(_)) => Some(view! {
            <span class="title-state-icon pull-request open" title="Open pull request">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z"/>
                    <path d="M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z"/>
                    <path d="M5.5 15V9"/>
                    <path d="M8 17.5h4.5a3 3 0 0 0 3-3V8"/>
                    <path d="m15.5 8 2.8 2.8L21 8"/>
                </svg>
            </span>
        }.into_any()),
        (Some("Issue"), _) if issue_state.as_deref() == Some("CLOSED") => Some(view! {
            <span class="title-state-icon issue closed" title="Closed issue">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M12 21a9 9 0 1 1 0-18a9 9 0 0 1 0 18Z"/>
                    <path d="M9 9l6 6"/>
                    <path d="M15 9l-6 6"/>
                </svg>
            </span>
        }.into_any()),
        (Some("Issue"), _) => Some(view! {
            <span class="title-state-icon issue open" title="Open issue">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M12 21a9 9 0 1 1 0-18a9 9 0 0 1 0 18Z"/>
                    <path d="M12 8.5v5"/>
                    <path d="M12 16.5h.01"/>
                </svg>
            </span>
        }.into_any()),
        _ => None,
    }
}

fn source_label(source: &str) -> &str {
    match source {
        "my_pr" => "My PR",
        "notification" => "Notification",
        other => other,
    }
}

#[component]
fn ReviewModal(
    content: ReadSignal<Option<String>>,
    set_content: WriteSignal<Option<String>>,
) -> impl IntoView {
    let is_open = move || content.get().is_some();

    view! {
        <Show when=is_open>
            <div class="modal-overlay" on:click=move |_| set_content.set(None)>
                <dialog open class="review-dialog" on:click=move |e| e.stop_propagation()>
                    <article>
                        <header class="modal-head">
                            <h2>"Review"</h2>
                            <button class="btn" on:click=move |_| set_content.set(None)>"Close"</button>
                        </header>
                        <pre id="review-content">{move || content.get().unwrap_or_default()}</pre>
                    </article>
                </dialog>
            </div>
        </Show>
    }
}
