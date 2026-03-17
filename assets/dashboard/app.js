const events = new EventSource("/dashboard/events");

events.addEventListener("update", () => {
  window.location.reload();
});

events.onerror = () => {
  // Let the browser keep retrying the SSE connection.
};
