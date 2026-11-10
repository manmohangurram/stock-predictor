import { api } from "../api";

/** Run now, and run-again: both post a prompt and reload into the running state. */
export function wireRuns(reload: () => Promise<void>): void {
  document.querySelectorAll<HTMLButtonElement>("[data-run-again]").forEach((button) => {
    button.addEventListener("click", async () => {
      button.disabled = true;
      button.textContent = "Starting…";
      try {
        await api.runNow(button.dataset.runAgain!);
        await reload();
      } catch (e) {
        button.textContent = String(e);
      }
    });
  });
  const form = document.querySelector<HTMLFormElement>("[data-run-now]");
  if (!form) return;
  const status = form.querySelector<HTMLElement>("[data-run-status]");
  form.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const prompt = String(data.get("prompt") ?? "poll");
    const symbol = String(data.get("symbol") ?? "").trim();
    if (prompt === "decide" && !symbol) {
      if (status) status.textContent = "Name the instrument to ask about.";
      return;
    }
    if (status) status.textContent = "Starting…";
    try {
      await api.runNow(prompt, symbol || undefined);
      await reload();
    } catch (e) {
      if (status) status.textContent = String(e);
    }
  });
}

/** Approve posts the fill you type; reject records a reason. Both stay on the page. */
