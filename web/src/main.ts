import { api } from "./api";
import { renderCalls } from "./pages/calls";
import { renderInstrument } from "./pages/instrument";
import { renderLedger } from "./pages/ledger";
import { renderPositions } from "./pages/positions";
import { renderSettings } from "./pages/settings";

const app = document.querySelector<HTMLDivElement>("#app")!;

async function route(): Promise<void> {
  const path = window.location.pathname;
  try {
    if (path === "/settings") {
      app.innerHTML = renderSettings(await api.settings());
      wireSettings();
    } else if (path === "/positions") {
      app.innerHTML = renderPositions(await api.dashboard());
    } else if (path.startsWith("/i/")) {
      app.innerHTML = renderInstrument(await api.analysis(decodeURIComponent(path.slice(3))));
    } else if (path === "/calls") {
      app.innerHTML = renderCalls(await api.calls());
    } else {
      app.innerHTML = renderLedger(await api.dashboard());
      wireRows();
    }
  } catch (e) {
    app.innerHTML = `<main class="page"><div class="header"><h1>Something failed</h1>
      <div class="double-rule"></div></div>
      <p class="guard">${String(e)}</p><button class="btn" onclick="location.reload()">Retry</button></main>`;
  }
}

function wireRows(): void {
  document.querySelectorAll<HTMLTableRowElement>(".row-main").forEach((row) => {
    row.addEventListener("click", () => {
      const detail = row.nextElementSibling as HTMLElement | null;
      if (detail) detail.hidden = !detail.hidden;
    });
  });
}

function wireSettings(): void {
  const form = document.querySelector<HTMLFormElement>("#settings-form");
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const budget = Number(data.get("budget"));
    const currency = String(data.get("currency"));
    const res = await api.putSettings({ budget, currency });
    if (!res.ok) {
      window.alert(res.body.error === "below_committed" ? `Committed is ${res.body.committed} — the budget cannot go below it.` : String(res.body.error));
      return;
    }
    await route();
  });
  document.querySelector<HTMLInputElement>("#budget")?.addEventListener("input", (ev) => {
    const form = ev.currentTarget as HTMLInputElement;
    const save = document.querySelector<HTMLButtonElement>(`#settings-form button[type="submit"]`);
    if (save) save.disabled = Number(form.value) < Number(form.dataset.committed ?? 0);
  });
}

route();
window.setInterval(() => {
  if (window.location.pathname === "/") void route();
}, 60_000);
