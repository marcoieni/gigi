const dashboardRoot = document.getElementById("dashboard-root");
let refreshPromise = null;

async function refreshDashboard() {
  if (refreshPromise) {
    return refreshPromise;
  }

  refreshPromise = (async () => {
    const response = await fetch("/dashboard/fragment", {
      headers: { "x-requested-with": "gigi-dashboard" },
    });

    if (!response.ok) {
      throw new Error(await readError(response));
    }

    const dropdown = document.querySelector("details.repo-dropdown");
    const wasOpen = dropdown && dropdown.hasAttribute("open");
    dashboardRoot.innerHTML = await response.text();
    if (wasOpen) {
      const restored = document.querySelector("details.repo-dropdown");
      if (restored) {
        restored.setAttribute("open", "");
      }
    }
  })();

  try {
    await refreshPromise;
  } finally {
    refreshPromise = null;
  }
}

function statusNode() {
  return document.getElementById("status-text");
}

function setStatus(text) {
  const node = statusNode();
  if (node) {
    node.textContent = text;
  }
}

function setButtonPending(button, isPending) {
  if (!(button instanceof HTMLButtonElement)) {
    return;
  }

  const isIcon = button.classList.contains("icon-btn");
  const usesSpinner = isIcon || button.dataset.loadingMode === "spinner";

  if (!button.dataset.label) {
    button.dataset.label = button.textContent.trim();
  }

  if (usesSpinner && !button.dataset.contentBackup) {
    button.dataset.contentBackup = button.innerHTML;
  }

  if (!button.dataset.ariaLabelBackup) {
    button.dataset.ariaLabelBackup = button.getAttribute("aria-label") ?? "__missing__";
  }

  if (!button.dataset.minWidthBackup) {
    button.dataset.minWidthBackup = button.style.minWidth;
  }

  button.disabled = isPending;
  button.classList.toggle("loading", isPending);

  if (usesSpinner) {
    if (isPending) {
      button.style.minWidth = `${Math.ceil(button.getBoundingClientRect().width)}px`;
      button.innerHTML = '<span class="spinner" aria-hidden="true"></span>';
      button.setAttribute(
        "aria-label",
        button.dataset.loadingLabel || button.dataset.label || "Working..."
      );
    } else {
      if (button.dataset.contentBackup) {
        button.innerHTML = button.dataset.contentBackup;
      }

      button.style.minWidth = button.dataset.minWidthBackup;

      if (button.dataset.ariaLabelBackup === "__missing__") {
        button.removeAttribute("aria-label");
      } else {
        button.setAttribute("aria-label", button.dataset.ariaLabelBackup);
      }
    }
  } else {
    button.textContent = isPending
      ? button.dataset.loadingLabel || "Working..."
      : button.dataset.label;
  }
}

function encodeForm(form) {
  return new URLSearchParams(new FormData(form));
}

async function submitAsyncForm(form, submitter) {
  setButtonPending(submitter, true);

  try {
    setStatus("Working...");
    const response = await fetch(form.action, {
      method: (form.method || "post").toUpperCase(),
      body: encodeForm(form),
      headers: {
        "content-type": "application/x-www-form-urlencoded;charset=UTF-8",
      },
    });

    if (!response.ok) {
      throw new Error(await readError(response));
    }

    await refreshDashboard();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus(message);
  } finally {
    setButtonPending(submitter, false);
  }
}

async function readError(response) {
  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    try {
      const body = await response.json();
      if (typeof body?.error === "string" && body.error.length > 0) {
        return body.error;
      }
    } catch {
      // Ignore invalid error bodies.
    }
  }

  const text = await response.text();
  return text || `Request failed with status ${response.status}`;
}

document.addEventListener("submit", async (event) => {
  const form = event.target;
  if (!(form instanceof HTMLFormElement) || !form.matches("[data-async-form]")) {
    return;
  }

  event.preventDefault();
  await submitAsyncForm(form, event.submitter);
});

let repoFilterTimer = null;

document.addEventListener("change", async (event) => {
  const target = event.target;
  if (!(target instanceof HTMLInputElement)) {
    return;
  }

  // Repo filter: debounce so the user can toggle several repos.
  const repoForm = target.closest("#repo-filter-form");
  if (repoForm instanceof HTMLFormElement) {
    clearTimeout(repoFilterTimer);
    repoFilterTimer = setTimeout(async () => {
      await submitAsyncForm(repoForm);
    }, 600);
    return;
  }

  const form = target.closest("form[data-auto-submit-form]");
  if (!(form instanceof HTMLFormElement)) {
    return;
  }

  await submitAsyncForm(form);
});

document.addEventListener("click", (event) => {
  // Close repo dropdown when clicking outside.
  const openDropdown = document.querySelector("details.repo-dropdown[open]");
  if (openDropdown && !openDropdown.contains(event.target)) {
    openDropdown.removeAttribute("open");
  }

  const closeButton = event.target.closest("#close-modal");
  if (closeButton) {
    const modal = document.getElementById("review-modal");
    if (modal instanceof HTMLDialogElement) {
      modal.close();
    }
    return;
  }

  const reviewButton = event.target.closest(".review-open");
  if (!(reviewButton instanceof HTMLButtonElement)) {
    return;
  }

  const modal = document.getElementById("review-modal");
  const content = document.getElementById("review-content");
  const fixForm = document.getElementById("fix-form");
  if (!(modal instanceof HTMLDialogElement) || !(content instanceof HTMLElement)) {
    return;
  }

  const raw = reviewButton.dataset.reviewContent || "";
  const cleaned = raw.replace(/\s*REQUIRES_CODE_CHANGES:\s*(YES|NO)\s*/g, "\n").trim();
  content.textContent = cleaned;

  if (typeof hljs !== "undefined") {
    content.classList.add("language-markdown");
    delete content.dataset.highlighted;
    hljs.highlightElement(content);
  }

  if (fixForm instanceof HTMLFormElement) {
    const fixAction = reviewButton.dataset.fixAction;
    if (fixAction) {
      fixForm.action = fixAction;
      fixForm.style.display = "";
    } else {
      fixForm.style.display = "none";
    }
  }

  modal.showModal();
});

const events = new EventSource("/dashboard/events");
events.addEventListener("update", async () => {
  try {
    await refreshDashboard();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus(message);
  }
});

events.onerror = () => {
  setStatus("Live updates disconnected. Retrying...");
};
