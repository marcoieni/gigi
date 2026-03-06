const statusEl = document.getElementById("status");
const threadsEl = document.getElementById("threads");
const refreshBtn = document.getElementById("refresh-btn");
const filterNotificationsEl = document.getElementById("filter-notifications");
const filterPrsEl = document.getElementById("filter-prs");
const filterDoneEl = document.getElementById("filter-done");
const filterNotDoneEl = document.getElementById("filter-not-done");
const modal = document.getElementById("review-modal");
const closeModal = document.getElementById("close-modal");
const reviewContent = document.getElementById("review-content");
const pendingReviews = new Set();
const pendingFixes = new Set();
const pendingDone = new Set();
const pendingLaunches = new Set();

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

closeModal.addEventListener("click", () => modal.close());
modal.addEventListener("close", () => {
  activeReviewPrUrl = null;
});
refreshBtn.addEventListener("click", async () => {
  await refreshNow();
  await loadThreads();
});
for (const filterEl of [
  filterNotificationsEl,
  filterPrsEl,
  filterDoneEl,
  filterNotDoneEl,
]) {
  filterEl.addEventListener("change", async () => {
    await loadThreads();
  });
}

function setStatus(text) {
  statusEl.textContent = text;
}

function currentFilters() {
  return {
    show_notifications: filterNotificationsEl.checked,
    show_prs: filterPrsEl.checked,
    show_done: filterDoneEl.checked,
    show_not_done: filterNotDoneEl.checked,
  };
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
  for (const thread of threadsState) {
    threadsEl.appendChild(threadCard(thread));
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

function launchKey(kind, threadKey) {
  return `${kind}:${threadKey}`;
}

function iconSvg(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");

  const paths =
    name === "vscode"
      ? VSCODE_ICON_PATHS
      : name === "terminal"
        ? TERMINAL_ICON_PATHS
        : name === "notification"
          ? NOTIFICATION_ICON_PATHS
          : name === "my-pr"
            ? MY_PR_ICON_PATHS
            : name === "check"
              ? CHECK_ICON_PATHS
        : [];

  for (const d of paths) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.appendChild(path);
  }
  return svg;
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

  const sources = thread.source.split(" + ");
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
  const markAuthoredPr = thread.source.split(" + ").includes("my_pr");
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
  if (thread.pr_state === "MERGED") {
    const mergedIcon = document.createElement("span");
    mergedIcon.className = "state-icon merged";
    mergedIcon.setAttribute("aria-label", "Merged pull request");
    mergedIcon.title = "Merged";
    title.appendChild(mergedIcon);
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
    !!thread.github_thread_id || thread.source.split(" + ").includes("my_pr");
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

async function loadThreads() {
  setStatus("Loading...");
  try {
    threadsState = await api(threadsPath());
    renderThreads();
    setStatus(`Loaded ${threadsState.length} items`);
  } catch (err) {
    setStatus(`Load failed: ${err.message}`);
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

loadThreads();
