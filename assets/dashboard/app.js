const statusEl = document.getElementById("status");
const threadsEl = document.getElementById("threads");
const refreshBtn = document.getElementById("refresh-btn");
const filterNotificationsEl = document.getElementById("filter-notifications");
const filterPrsEl = document.getElementById("filter-prs");
const filterDoneEl = document.getElementById("filter-done");
const filterNotDoneEl = document.getElementById("filter-not-done");
const groupByRepositoryEl = document.getElementById("group-by-repository");
const modal = document.getElementById("review-modal");
const closeModal = document.getElementById("close-modal");
const reviewContent = document.getElementById("review-content");
const pendingReviews = new Set();
const pendingFixes = new Set();
const pendingDone = new Set();
const pendingLaunches = new Set();

const FILTERS_STORAGE_KEY = "gigi.dashboard.filters";

let threadsState = [];
let activeReviewPrUrl = null;

const VSCODE_ICON_PATHS = [
  // Left chevron.
  "M9 8 5 12l4 4",
  // Right chevron.
  "m15 8 4 4-4 4",
  // Slash.
  "M14 6 10 18",
];

const TERMINAL_ICON_PATHS = [
  // Terminal window outline.
  "M4 5.5h16a1.5 1.5 0 0 1 1.5 1.5v10A1.5 1.5 0 0 1 20 18.5H4A1.5 1.5 0 0 1 2.5 17V7A1.5 1.5 0 0 1 4 5.5Z",
  // Prompt chevron.
  "m6.4 9 2.6 2.3-2.6 2.3",
  // Command line.
  "M11.7 13.8h4.9",
];

const NOTIFICATION_ICON_PATHS = [
  // Bell body.
  "M15.5 17.5h4l-1.1-1.1a2 2 0 0 1-.6-1.4V11a5.8 5.8 0 1 0-11.6 0v4a2 2 0 0 1-.6 1.4l-1.1 1.1h4",
  // Bell clapper.
  "M10 17.5a2 2 0 0 0 4 0",
];

const MY_PR_ICON_PATHS = [
  // Top node.
  "M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  // Bottom node.
  "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  // Connector.
  "M15.5 9v5.5a3 3 0 0 1-3 3H8",
  // Stem.
  "M5.5 15V9",
];

const CHECK_ICON_PATHS = [
  "m5 12.5 4 4 10-10",
];

const PR_OPEN_ICON_PATHS = [
  "M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  "M5.5 15V9",
  "M8 17.5h4.5a3 3 0 0 0 3-3V8",
  "m15.5 8 2.8 2.8L21 8",
];

const PR_MERGED_ICON_PATHS = [
  "M18 6.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  "M18 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  "M8 15V9.5a3 3 0 0 1 3-3h2",
  "M15.5 10.5V15",
];

const PR_CLOSED_ICON_PATHS = [
  "M8 17.5a2.5 2.5 0 1 1-5 0a2.5 2.5 0 0 1 5 0Z",
  "M5.5 15V8.5",
  "M14.5 8.5 20 14",
  "M20 8.5 14.5 14",
];

const ISSUE_OPEN_ICON_PATHS = [
  "M12 21a9 9 0 1 1 0-18a9 9 0 0 1 0 18Z",
  "M12 8.5v5",
  "M12 16.5h.01",
];

const ISSUE_CLOSED_ICON_PATHS = [
  "M12 21a9 9 0 1 1 0-18a9 9 0 0 1 0 18Z",
  "M9 9l6 6",
  "M15 9l-6 6",
];

const ICON_PATHS = {
  vscode: VSCODE_ICON_PATHS,
  terminal: TERMINAL_ICON_PATHS,
  notification: NOTIFICATION_ICON_PATHS,
  "my-pr": MY_PR_ICON_PATHS,
  check: CHECK_ICON_PATHS,
  "pr-open": PR_OPEN_ICON_PATHS,
  "pr-merged": PR_MERGED_ICON_PATHS,
  "pr-closed": PR_CLOSED_ICON_PATHS,
  "issue-open": ISSUE_OPEN_ICON_PATHS,
  "issue-closed": ISSUE_CLOSED_ICON_PATHS,
};

closeModal.addEventListener("click", () => modal.close());
modal.addEventListener("close", () => {
  activeReviewPrUrl = null;
});
refreshBtn.addEventListener("click", async () => {
  await refreshNow();
  await loadThreads();
});
groupByRepositoryEl.addEventListener("change", async () => {
  try {
    await saveCurrentFilters();
  } catch (err) {
    setStatus(`Saving filters failed: ${err.message}`);
  }
  renderThreads();
  if (threadsState.length > 0) {
    setStatus(loadedStatusText());
  }
});

function setStatus(text) {
  statusEl.textContent = text;
}

function applyFilters(filters) {
  filterNotificationsEl.checked = filters.show_notifications;
  filterPrsEl.checked = filters.show_prs;
  filterDoneEl.checked = filters.show_done;
  filterNotDoneEl.checked = filters.show_not_done;
  groupByRepositoryEl.checked = filters.group_by_repository;
}

function currentFilters() {
  return {
    show_notifications: filterNotificationsEl.checked,
    show_prs: filterPrsEl.checked,
    show_done: filterDoneEl.checked,
    show_not_done: filterNotDoneEl.checked,
    group_by_repository: groupByRepositoryEl.checked,
  };
}

function filtersEqual(left, right) {
  return (
    left.show_notifications === right.show_notifications &&
    left.show_prs === right.show_prs &&
    left.show_done === right.show_done &&
    left.show_not_done === right.show_not_done &&
    left.group_by_repository === right.group_by_repository
  );
}

function readCachedFilters() {
  try {
    const raw = window.localStorage.getItem(FILTERS_STORAGE_KEY);
    if (!raw) {
      return null;
    }

    const parsed = JSON.parse(raw);
    if (
      typeof parsed?.show_notifications !== "boolean" ||
      typeof parsed?.show_prs !== "boolean" ||
      typeof parsed?.show_done !== "boolean" ||
      typeof parsed?.show_not_done !== "boolean" ||
      typeof parsed?.group_by_repository !== "boolean"
    ) {
      return null;
    }

    return parsed;
  } catch {
    return null;
  }
}

function cacheFilters(filters) {
  try {
    window.localStorage.setItem(FILTERS_STORAGE_KEY, JSON.stringify(filters));
  } catch {
    // Ignore storage failures and keep DB persistence as the fallback.
  }
}

function threadsPath() {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(currentFilters())) {
    params.set(key, String(value));
  }
  return `/api/threads?${params.toString()}`;
}

function repoUrl(repository) {
  return `https://github.com/${repository}`;
}

function sourceLabel(part) {
  if (part === "my_pr") return "My PR";
  if (part === "notification") return "Notification";
  return part;
}

function sourceIconName(part) {
  if (part === "my_pr") return "my-pr";
  if (part === "notification") return "notification";
  return null;
}

function renderThreads() {
  threadsEl.replaceChildren();
  threadsEl.classList.toggle("grouped", groupByRepositoryEl.checked);

  if (!groupByRepositoryEl.checked) {
    for (const thread of threadsState) {
      threadsEl.appendChild(threadCard(thread));
    }
    return;
  }

  for (const [repository, repoThreads] of groupThreadsByRepository(threadsState)) {
    threadsEl.appendChild(repositorySection(repository, repoThreads));
  }
}

function updateThreadsForPr(prUrl, mutate) {
  let changed = false;
  threadsState = threadsState.map((thread) => {
    if (thread.pr_url !== prUrl) return thread;
    changed = true;
    return mutate(thread);
  });
  if (changed) {
    renderThreads();
  }
}

function setButtonContent(button, label, isLoading) {
  button.replaceChildren();
  if (isLoading) {
    const spinner = document.createElement("span");
    spinner.className = "spinner";
    spinner.setAttribute("aria-hidden", "true");
    button.appendChild(spinner);
  }

  const text = document.createElement("span");
  text.textContent = label;
  button.appendChild(text);
}

async function api(path, options = {}) {
  const res = await fetch(path, {
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`${res.status} ${body}`);
  }
  const contentType = res.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    return res.json();
  }
  return null;
}

async function saveCurrentFilters() {
  const filters = currentFilters();
  cacheFilters(filters);
  await api("/api/dashboard/filters", {
    method: "POST",
    body: JSON.stringify(filters),
  });
}

function launchKey(kind, threadKey) {
  return `${kind}:${threadKey}`;
}

function iconSvg(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");

  const paths = ICON_PATHS[name] || [];

  for (const d of paths) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.appendChild(path);
  }
  return svg;
}

function threadStateConfig(thread) {
  const subjectType = thread.subject_type;
  const prState = thread.pr_state || (thread.sources.includes("my_pr") ? "OPEN" : null);

  if (subjectType === "PullRequest" && prState) {
    if (prState === "MERGED") {
      return {
        iconName: "pr-merged",
        status: "merged",
        kind: "pull-request",
        label: "Merged pull request",
        title: "Merged pull request",
      };
    }
    if (prState === "CLOSED") {
      return {
        iconName: "pr-closed",
        status: "closed",
        kind: "pull-request",
        label: "Closed pull request",
        title: "Closed pull request",
      };
    }
    return {
      iconName: "pr-open",
      status: "open",
      kind: "pull-request",
      label: "Open pull request",
      title: "Open pull request",
    };
  }

  if (subjectType === "Issue" && thread.issue_state) {
    if (thread.issue_state === "CLOSED") {
      return {
        iconName: "issue-closed",
        status: "closed",
        kind: "issue",
        label: "Closed issue",
        title: "Closed issue",
      };
    }
    return {
      iconName: "issue-open",
      status: "open",
      kind: "issue",
      label: "Open issue",
      title: "Open issue",
    };
  }

  return null;
}

function threadStateIcon(thread) {
  const config = threadStateConfig(thread);
  if (!config) {
    return null;
  }

  const icon = document.createElement("span");
  icon.className = `title-state-icon ${config.kind} ${config.status}`;
  icon.setAttribute("aria-label", config.label);
  icon.title = config.title;
  icon.appendChild(iconSvg(config.iconName));
  return icon;
}

function metaSeparator() {
  const separator = document.createElement("span");
  separator.className = "meta-separator";
  separator.textContent = "•";
  separator.setAttribute("aria-hidden", "true");
  return separator;
}

function repositoryLink(repository) {
  const link = document.createElement("a");
  link.className = "thread-link repo-link";
  link.href = repoUrl(repository);
  link.target = "_blank";
  link.rel = "noreferrer";
  link.textContent = repository;
  return link;
}

function sourceBadge(part) {
  const iconName = sourceIconName(part);
  if (!iconName) {
    const text = document.createElement("span");
    text.textContent = sourceLabel(part);
    return text;
  }

  const badge = document.createElement("span");
  badge.className = "source-badge";
  badge.title = sourceLabel(part);
  badge.setAttribute("aria-label", sourceLabel(part));
  badge.appendChild(iconSvg(iconName));
  return badge;
}

function buildMeta(meta, thread) {
  meta.replaceChildren();
  meta.appendChild(repositoryLink(thread.repository));
  meta.appendChild(metaSeparator());

  const sources = thread.sources;
  sources.forEach((part, index) => {
    meta.appendChild(sourceBadge(part));
    if (index < sources.length - 1) {
      meta.appendChild(document.createTextNode(" "));
    }
  });

  meta.appendChild(metaSeparator());

  const updated = document.createElement("span");
  updated.textContent = thread.updated_at;
  meta.appendChild(updated);
}

function groupThreadsByRepository(threads) {
  const groups = new Map();
  for (const thread of threads) {
    if (!groups.has(thread.repository)) {
      groups.set(thread.repository, []);
    }
    groups.get(thread.repository).push(thread);
  }
  // Sort groups by latest thread update time, descending
  return new Map(
    [...groups.entries()].sort((a, b) => {
      const latestA = a[1].reduce((max, t) => t.updated_at > max ? t.updated_at : max, "");
      const latestB = b[1].reduce((max, t) => t.updated_at > max ? t.updated_at : max, "");
      return latestB.localeCompare(latestA);
    })
  );
}

function repositorySection(repository, repoThreads) {
  const section = document.createElement("section");
  section.className = "repo-group";

  const header = document.createElement("header");
  header.className = "repo-group-header";

  const title = document.createElement("h2");
  title.className = "repo-group-title";
  title.appendChild(repositoryLink(repository));
  header.appendChild(title);

  const count = document.createElement("span");
  count.className = "repo-group-count";
  count.textContent = `${repoThreads.length} ${repoThreads.length === 1 ? "item" : "items"}`;
  header.appendChild(count);

  section.appendChild(header);

  const grid = document.createElement("div");
  grid.className = "threads repo-group-threads";
  for (const thread of repoThreads) {
    grid.appendChild(threadCard(thread));
  }
  section.appendChild(grid);

  return section;
}

function iconButton(iconName, label, isLoading, onClick) {
  const button = document.createElement("button");
  button.className = `btn icon-btn ${isLoading ? "loading" : ""}`;
  button.type = "button";
  button.disabled = isLoading;
  button.title = label;
  button.setAttribute("aria-label", label);

  if (isLoading) {
    const spinner = document.createElement("span");
    spinner.className = "spinner";
    spinner.setAttribute("aria-hidden", "true");
    button.appendChild(spinner);
  } else {
    button.appendChild(iconSvg(iconName));
  }

  button.addEventListener("click", onClick);
  return button;
}

async function launchProject(kind, thread) {
  const key = launchKey(kind, thread.thread_key);
  pendingLaunches.add(key);
  renderThreads();
  setStatus(kind === "vscode" ? "Opening VS Code..." : "Opening Terminal...");

  try {
    await api(`/api/open/${kind}`, {
      method: "POST",
      body: JSON.stringify({
        repository: thread.repository,
        pr_url: thread.pr_url,
      }),
    });
    setStatus(kind === "vscode" ? "VS Code opened" : "Terminal opened");
  } catch (err) {
    setStatus(`Open failed: ${err.message}`);
  } finally {
    pendingLaunches.delete(key);
    renderThreads();
  }
}

function parsePrUrl(prUrl) {
  const match = prUrl.match(/^https:\/\/github\.com\/([^/]+)\/([^/]+)\/pull\/(\d+)/);
  if (!match) return null;
  return { owner: match[1], repo: match[2], number: Number(match[3]) };
}

async function openReview(prUrl) {
  const parsed = parsePrUrl(prUrl);
  if (!parsed) return;
  activeReviewPrUrl = prUrl;
  const review = await api(
    `/api/prs/${parsed.owner}/${parsed.repo}/${parsed.number}/review/latest`
  );
  reviewContent.textContent = review?.content_md || "No review stored yet.";
  modal.showModal();
}

async function doFixes(prUrl) {
  const parsed = parsePrUrl(prUrl);
  if (!parsed) return;
  setStatus("Running fixes...");
  try {
    await api(`/api/prs/${parsed.owner}/${parsed.repo}/${parsed.number}/fix`, { method: "POST" });
    setStatus("Fix run completed");
  } catch (err) {
    setStatus(`Fix run failed: ${err.message}`);
  }
}

async function runReview(prUrl) {
  const parsed = parsePrUrl(prUrl);
  if (!parsed) return;
  pendingReviews.add(prUrl);
  renderThreads();
  setStatus("Running review...");
  try {
    await api(`/api/prs/${parsed.owner}/${parsed.repo}/${parsed.number}/review`, {
      method: "POST",
    });
    const latestReview = await api(
      `/api/prs/${parsed.owner}/${parsed.repo}/${parsed.number}/review/latest`
    );
    updateThreadsForPr(prUrl, (thread) => ({
      ...thread,
      latest_requires_code_changes: latestReview?.requires_code_changes ?? null,
    }));
    if (activeReviewPrUrl === prUrl && modal.open) {
      reviewContent.textContent = latestReview?.content_md || "No review stored yet.";
    }
    setStatus("Review completed");
  } catch (err) {
    setStatus(`Review failed: ${err.message}`);
  } finally {
    pendingReviews.delete(prUrl);
    renderThreads();
  }
}

async function markDone(thread) {
  const pendingKey = thread.thread_key;
  const markAuthoredPr = thread.sources.includes("my_pr");
  pendingDone.add(pendingKey);
  renderThreads();
  setStatus("Marking done...");
  try {
    await api("/api/threads/done", {
      method: "POST",
      body: JSON.stringify({
        github_thread_id: thread.github_thread_id,
        pr_url: thread.pr_url,
        mark_authored_pr: markAuthoredPr,
      }),
    });
    await loadThreads();
    setStatus("Marked done");
  } catch (err) {
    setStatus(`Mark done failed: ${err.message}`);
  } finally {
    pendingDone.delete(pendingKey);
    renderThreads();
  }
}

function threadCard(thread) {
  const card = document.createElement("article");
  card.className = "thread";

  const titleHref = thread.subject_url || thread.pr_url;
  const title = document.createElement("h3");
  const stateIcon = threadStateIcon(thread);
  if (stateIcon) {
    title.appendChild(stateIcon);
  }
  if (titleHref) {
    const titleLink = document.createElement("a");
    titleLink.className = "thread-link";
    titleLink.href = titleHref;
    titleLink.target = "_blank";
    titleLink.rel = "noreferrer";
    titleLink.textContent = thread.subject_title;
    title.appendChild(titleLink);
  } else {
    title.textContent = thread.subject_title;
  }
  card.appendChild(title);

  const meta = document.createElement("p");
  meta.className = "meta";
  buildMeta(meta, thread);
  card.appendChild(meta);

  const row = document.createElement("div");
  row.className = "row";

  const actions = document.createElement("div");
  actions.className = "icon-actions";
  actions.appendChild(
    iconButton(
      "vscode",
      thread.pr_url ? "Open PR in VS Code" : "Open project in VS Code",
      pendingLaunches.has(launchKey("vscode", thread.thread_key)),
      async () => {
        await launchProject("vscode", thread);
      }
    )
  );
  actions.appendChild(
    iconButton(
      "terminal",
      thread.pr_url ? "Open PR in Terminal" : "Open project in Terminal",
      pendingLaunches.has(launchKey("terminal", thread.thread_key)),
      async () => {
        await launchProject("terminal", thread);
      }
    )
  );
  row.appendChild(actions);

  if (thread.pr_url) {
    const hasReview = thread.latest_requires_code_changes !== null;
    const needsChanges = thread.latest_requires_code_changes === true;
    const reviewPending = pendingReviews.has(thread.pr_url);
    const fixPending = pendingFixes.has(thread.pr_url);
    const reviewBtn = document.createElement("button");
    reviewBtn.className = `pill ${!hasReview ? "pending" : needsChanges ? "unsafe" : "safe"}`;
    reviewBtn.textContent = hasReview ? (needsChanges ? "Fixes needed" : "Safe") : "No review";
    reviewBtn.addEventListener("click", () => openReview(thread.pr_url));
    reviewBtn.disabled = reviewPending;
    row.appendChild(reviewBtn);

    const runReviewBtn = document.createElement("button");
    runReviewBtn.className = `btn ${reviewPending ? "loading" : ""}`;
    runReviewBtn.disabled = reviewPending || fixPending;
    setButtonContent(
      runReviewBtn,
      reviewPending ? "Reviewing..." : hasReview ? "Re-review" : "Review now",
      reviewPending
    );
    runReviewBtn.addEventListener("click", async () => {
      await runReview(thread.pr_url);
    });
    row.appendChild(runReviewBtn);

    if (needsChanges) {
      const fixBtn = document.createElement("button");
      fixBtn.className = `btn ${fixPending ? "loading" : ""}`;
      fixBtn.disabled = fixPending || reviewPending;
      setButtonContent(fixBtn, fixPending ? "Fixing..." : "Do fixes", fixPending);
      fixBtn.addEventListener("click", async () => {
        pendingFixes.add(thread.pr_url);
        renderThreads();
        try {
          await doFixes(thread.pr_url);
          await loadThreads();
        } finally {
          pendingFixes.delete(thread.pr_url);
          renderThreads();
        }
      });
      row.appendChild(fixBtn);
    }
  }

  const canMarkDone =
    !!thread.github_thread_id || thread.sources.includes("my_pr");
  if (canMarkDone) {
    const donePending = pendingDone.has(thread.thread_key);
    const doneBtn = document.createElement("button");
    doneBtn.className = `btn icon-btn ${donePending ? "loading" : ""}`;
    doneBtn.type = "button";
    doneBtn.title = thread.done ? "Done" : "Mark done";
    doneBtn.setAttribute("aria-label", thread.done ? "Done" : "Mark done");
    doneBtn.replaceChildren();
    if (donePending) {
      const spinner = document.createElement("span");
      spinner.className = "spinner";
      spinner.setAttribute("aria-hidden", "true");
      doneBtn.appendChild(spinner);
    } else {
      doneBtn.appendChild(iconSvg("check"));
    }
    doneBtn.disabled = !!thread.done || donePending;
    doneBtn.addEventListener("click", async () => {
      await markDone(thread);
    });
    row.appendChild(doneBtn);
  }

  if (thread.unread) {
    const unread = document.createElement("span");
    unread.className = "unread-dot";
    unread.setAttribute("aria-label", "Unread");
    unread.title = "Unread";
    row.appendChild(unread);
  }

  card.appendChild(row);
  return card;
}

function loadedStatusText() {
  if (!groupByRepositoryEl.checked) {
    return `Loaded ${threadsState.length} items`;
  }

  const repositoryCount = new Set(threadsState.map((thread) => thread.repository)).size;
  return `Loaded ${threadsState.length} items in ${repositoryCount} ${repositoryCount === 1 ? "repository" : "repositories"}`;
}

async function loadThreads() {
  setStatus("Loading...");
  try {
    threadsState = await api(threadsPath());
    renderThreads();
    setStatus(loadedStatusText());
  } catch (err) {
    setStatus(`Load failed: ${err.message}`);
  }
}

async function initializeFilters() {
  const cachedFilters = readCachedFilters();
  if (cachedFilters) {
    applyFilters(cachedFilters);
  }

  const storedFilters = await api("/api/dashboard/filters");
  if (!cachedFilters) {
    applyFilters(storedFilters);
    cacheFilters(storedFilters);
    return;
  }

  if (!filtersEqual(cachedFilters, storedFilters)) {
    await api("/api/dashboard/filters", {
      method: "POST",
      body: JSON.stringify(cachedFilters),
    });
  }
}

async function refreshNow() {
  setStatus("Refreshing from GitHub...");
  try {
    const stats = await api("/api/refresh", { method: "POST" });
    setStatus(
      `Refreshed: notifications=${stats.notifications_fetched}, my_prs=${stats.authored_prs_fetched}, reviews=${stats.reviews_run}`
    );
  } catch (err) {
    setStatus(`Refresh failed: ${err.message}`);
  }
}

for (const filterEl of [
  filterNotificationsEl,
  filterPrsEl,
  filterDoneEl,
  filterNotDoneEl,
]) {
  filterEl.addEventListener("change", async () => {
    try {
      await saveCurrentFilters();
    } catch (err) {
      setStatus(`Saving filters failed: ${err.message}`);
    }
    await loadThreads();
  });
}

async function initializeDashboard() {
  try {
    await initializeFilters();
  } catch (err) {
    setStatus(`Loading saved filters failed: ${err.message}`);
  }
  await loadThreads();
  connectEventSource();
}

function connectEventSource() {
  const evtSource = new EventSource("/api/events");

  evtSource.addEventListener("poll_complete", async (e) => {
    try {
      const stats = JSON.parse(e.data);
      setStatus(
        `Auto-refreshed: notifications=${stats.notifications_fetched}, my_prs=${stats.authored_prs_fetched}, reviews=${stats.reviews_run}`
      );
    } catch {
      // ignore parse errors
    }
    await loadThreads();
  });

  evtSource.addEventListener("review_complete", async (e) => {
    try {
      const data = JSON.parse(e.data);
      const prUrl = data.pr_url;
      if (prUrl) {
        const parsed = parsePrUrl(prUrl);
        if (parsed) {
          const latestReview = await api(
            `/api/prs/${parsed.owner}/${parsed.repo}/${parsed.number}/review/latest`
          );
          updateThreadsForPr(prUrl, (thread) => ({
            ...thread,
            latest_requires_code_changes: latestReview?.requires_code_changes ?? null,
          }));
          if (activeReviewPrUrl === prUrl && modal.open) {
            reviewContent.textContent = latestReview?.content_md || "No review stored yet.";
          }
        }
      }
    } catch {
      // ignore parse errors
    }
  });

  evtSource.onerror = () => {
    // EventSource auto-reconnects; nothing extra needed.
  };
}

initializeDashboard();
