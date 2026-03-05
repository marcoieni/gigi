const statusEl = document.getElementById("status");
const threadsEl = document.getElementById("threads");
const refreshBtn = document.getElementById("refresh-btn");
const modal = document.getElementById("review-modal");
const closeModal = document.getElementById("close-modal");
const reviewContent = document.getElementById("review-content");
const pendingReviews = new Set();
const pendingFixes = new Set();
const pendingDone = new Set();

let threadsState = [];
let activeReviewPrUrl = null;

closeModal.addEventListener("click", () => modal.close());
modal.addEventListener("close", () => {
  activeReviewPrUrl = null;
});
refreshBtn.addEventListener("click", async () => {
  await refreshNow();
  await loadThreads();
});

function setStatus(text) {
  statusEl.textContent = text;
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

async function markDone(threadId) {
  pendingDone.add(threadId);
  renderThreads();
  setStatus("Marking done...");
  try {
    await api(`/api/threads/${threadId}/done`, { method: "POST" });
    threadsState = threadsState.filter((thread) => thread.github_thread_id !== threadId);
    setStatus("Marked done");
  } catch (err) {
    setStatus(`Mark done failed: ${err.message}`);
  } finally {
    pendingDone.delete(threadId);
    renderThreads();
  }
}

function threadCard(thread) {
  const card = document.createElement("article");
  card.className = "thread";

  const titleHref = thread.subject_url || thread.pr_url;
  const title = document.createElement("h3");
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
  meta.textContent = `${thread.repository} • ${thread.source} • ${thread.updated_at}`;
  card.appendChild(meta);

  const row = document.createElement("div");
  row.className = "row";

  if (thread.pr_url) {
    const hasReview = thread.latest_requires_code_changes !== null;
    const needsChanges = thread.latest_requires_code_changes === true;
    const reviewPending = pendingReviews.has(thread.pr_url);
    const fixPending = pendingFixes.has(thread.pr_url);
    const reviewBtn = document.createElement("button");
    reviewBtn.className = `pill ${needsChanges ? "unsafe" : "safe"}`;
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

  if (thread.github_thread_id) {
    const donePending = pendingDone.has(thread.github_thread_id);
    const doneBtn = document.createElement("button");
    doneBtn.className = `btn ${donePending ? "loading" : ""}`;
    setButtonContent(doneBtn, donePending ? "Saving..." : thread.done ? "Done" : "Mark done", donePending);
    doneBtn.disabled = !!thread.done || donePending;
    doneBtn.addEventListener("click", async () => {
      await markDone(thread.github_thread_id);
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
    threadsState = await api("/api/threads");
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
