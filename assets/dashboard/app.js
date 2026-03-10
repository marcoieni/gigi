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

    dashboardRoot.innerHTML = await response.text();
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

  if (!button.dataset.label) {
    button.dataset.label = button.textContent.trim();
  }

  if (isIcon && !button.dataset.svgBackup) {
    const svg = button.querySelector("svg");
    if (svg) {
      button.dataset.svgBackup = svg.outerHTML;
    }
  }

  button.disabled = isPending;
  button.classList.toggle("loading", isPending);

  if (isIcon) {
    if (isPending) {
      button.innerHTML = '<span class="spinner"></span>';
    } else if (button.dataset.svgBackup) {
      button.innerHTML = button.dataset.svgBackup;
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
  const submitter = event.submitter;
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
});

document.addEventListener("change", (event) => {
  const target = event.target;
  if (!(target instanceof HTMLInputElement)) {
    return;
  }

  const form = target.closest("form[data-auto-submit-form]");
  if (!(form instanceof HTMLFormElement)) {
    return;
  }

  if (form.requestSubmit) {
    form.requestSubmit();
  } else {
    form.dispatchEvent(new Event("submit", { cancelable: true, bubbles: true }));
  }
});

document.addEventListener("click", (event) => {
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
  if (!(modal instanceof HTMLDialogElement) || !(content instanceof HTMLElement)) {
    return;
  }

  content.textContent = reviewButton.dataset.reviewContent || "";
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
