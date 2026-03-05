const statusEl = document.getElementById("status");
const threadsEl = document.getElementById("threads");
const refreshBtn = document.getElementById("refresh-btn");
const modal = document.getElementById("review-modal");
const closeModal = document.getElementById("close-modal");
const reviewContent = document.getElementById("review-content");

closeModal.addEventListener("click", () => modal.close());
refreshBtn.addEventListener("click", async () => {
  await refreshNow();
  await loadThreads();
});

function setStatus(text) {
  statusEl.textContent = text;
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
  setStatus("Running review...");
  try {
    await api(`/api/prs/${parsed.owner}/${parsed.repo}/${parsed.number}/review`, {
      method: "POST",
    });
    setStatus("Review completed");
  } catch (err) {
    setStatus(`Review failed: ${err.message}`);
  }
}

async function markDone(threadId) {
  setStatus("Marking done...");
  await api(`/api/threads/${threadId}/done`, { method: "POST" });
  setStatus("Marked done");
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
    const reviewBtn = document.createElement("button");
    reviewBtn.className = `pill ${needsChanges ? "unsafe" : "safe"}`;
    reviewBtn.textContent = hasReview ? (needsChanges ? "Fixes needed" : "Safe") : "No review";
    reviewBtn.addEventListener("click", () => openReview(thread.pr_url));
    row.appendChild(reviewBtn);

    const runReviewBtn = document.createElement("button");
    runReviewBtn.className = "btn";
    runReviewBtn.textContent = hasReview ? "Re-review" : "Review now";
    runReviewBtn.addEventListener("click", async () => {
      await runReview(thread.pr_url);
      await loadThreads();
    });
    row.appendChild(runReviewBtn);

    if (needsChanges) {
      const fixBtn = document.createElement("button");
      fixBtn.className = "btn";
      fixBtn.textContent = "Do fixes";
      fixBtn.addEventListener("click", async () => {
        await doFixes(thread.pr_url);
        await loadThreads();
      });
      row.appendChild(fixBtn);
    }
  }

  if (thread.github_thread_id) {
    const doneBtn = document.createElement("button");
    doneBtn.className = "btn";
    doneBtn.textContent = thread.done ? "Done" : "Mark done";
    doneBtn.disabled = !!thread.done;
    doneBtn.addEventListener("click", async () => {
      await markDone(thread.github_thread_id);
      await loadThreads();
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
    const threads = await api("/api/threads");
    threadsEl.replaceChildren();
    for (const thread of threads) {
      threadsEl.appendChild(threadCard(thread));
    }
    setStatus(`Loaded ${threads.length} items`);
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
