import { api, type Hit } from "./api";
import { renderCalls } from "./pages/calls";
import { renderInstrument } from "./pages/instrument";
import { renderLedger } from "./pages/ledger";
import { renderPositions } from "./pages/positions";
import { renderSettings, shouldDisableSave } from "./pages/settings";

const app = document.querySelector<HTMLDivElement>("#app")!;
const nav = `<nav class="topnav">
  <a href="/">Ledger</a><a href="/positions">Positions</a>
  <a href="/settings">Settings</a><a href="/calls">Audit</a>
</nav>`;

function failure(e: unknown): string {
  return `<div class="shell"><div class="error-band">Something failed: ${String(e)}
    <button class="btn" onclick="location.reload()">Retry</button></div></div>`;
}

async function route(): Promise<void> {
  const path = window.location.pathname;
  try {
    if (path === "/settings") {
      const s = await api.settings();
      app.innerHTML = renderSettings(s);
      wireSettings();
      await route2();
    } else if (path === "/positions") {
      app.innerHTML = renderPositions(await api.dashboard());
      wireRows();
    } else if (path === "/calls") {
      app.innerHTML = renderCalls(await api.calls());
    } else if (path.startsWith("/i/")) {
      const symbol = decodeURIComponent(path.slice(3));
      const [analysis, detail, trades] = await Promise.all([
        api.analysis(symbol),
        api.instrument(symbol),
        api.instrumentTrades(symbol),
      ]);
      app.innerHTML = renderInstrument(analysis, detail, trades);
      wireTradeForm(symbol);
    } else {
      const [d, trades] = await Promise.all([api.dashboard(), api.trades()]);
      app.innerHTML = nav + renderLedger(d, trades);
      wireRows();
      wireSearch();
    }
  } catch (e) {
    app.innerHTML = failure(e);
  }
}

/** Append the nav to pages that build their own shell. */
async function route2(): Promise<void> {
  const shell = document.querySelector(".shell");
  if (shell && !document.querySelector(".topnav")) {
    shell.insertAdjacentHTML("afterbegin", nav);
  }
}

function wireRows(): void {
  document.querySelectorAll<HTMLElement>("[data-symbol]").forEach((row) => {
    const go = () => {
      const symbol = row.dataset.symbol;
      if (symbol) window.location.href = `/i/${encodeURIComponent(symbol)}`;
    };
    row.addEventListener("click", go);
    row.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        go();
      }
    });
  });
}

function wireSearch(): void {
  const input = document.querySelector<HTMLInputElement>("#search");
  const box = document.querySelector<HTMLDivElement>("#results");
  if (!input || !box) return;

  let timer: number | undefined;
  let last: Hit[] = [];

  const draw = (hits: Hit[]) => {
    box.innerHTML = hits.length
      ? hits
          .map(
            (h, i) => `<button class="hit" type="button" data-i="${i}" role="option"
            aria-label="Add ${h.code}">
            <b class="num">${h.code}</b>
            <span>${h.name}</span>
            <em>${h.venue || h.class}</em>
          </button>`,
          )
          .join("")
      : input.value.trim().length >= 2
        ? `<p class="empty">Nothing found.</p>`
        : "";
    box.querySelectorAll<HTMLButtonElement>(".hit").forEach((b) => {
      b.addEventListener("click", async () => {
        const hit = last[Number(b.dataset.i)];
        if (!hit) return;
        b.disabled = true;
        b.textContent = `Adding ${hit.code}…`;
        try {
          await api.addInstrument(hit.code, hit.name, hit.class);
          window.location.href = `/i/${encodeURIComponent(hit.code)}`;
        } catch (e) {
          b.textContent = `Failed: ${String(e)}`;
        }
      });
    });
  };

  input.addEventListener("input", () => {
    window.clearTimeout(timer);
    const q = input.value.trim();
    if (q.length < 2) {
      last = [];
      draw([]);
      return;
    }
    timer = window.setTimeout(async () => {
      try {
        last = await api.search(q);
        draw(last);
      } catch {
        box.innerHTML = `<p class="empty">Search unavailable.</p>`;
      }
    }, 250);
  });
}

function wireTradeForm(symbol: string): void {
  const form = document.querySelector<HTMLFormElement>(".trade-form");
  const status = document.querySelector<HTMLElement>("#trade-status");
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const side = String(data.get("side")) as "buy" | "sell";
    const qty = Number(data.get("qty"));
    const price = Number(data.get("price"));
    if (!(qty > 0) || !(price > 0)) {
      if (status) status.textContent = "Quantity and price must be positive.";
      return;
    }
    if (status) status.textContent = "Recording…";
    try {
      const res = await api.recordTrade(symbol, side, qty, price);
      if (status) {
        status.textContent = `Recorded. Cash ${Math.round(res.cash)}, realised ${Math.round(res.realised)}.`;
      }
      await route();
    } catch (e) {
      if (status) status.textContent = String(e);
    }
  });
}

function wireSettings(): void {
  const form = document.querySelector<HTMLFormElement>("#settings-form");
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const budget = Number(data.get("budget"));
    const currency = String(data.get("currency"));
    const key = String(data.get("deepseek_key") ?? "").trim();
    const res = await api.putSettings(
      key ? { budget, currency, deepseek_key: key } : { budget, currency },
    );
    if (!res.ok) {
      window.alert(
        res.body.error === "below_committed"
          ? `Committed is ${res.body.committed} — the budget cannot go below it.`
          : String(res.body.error),
      );
      return;
    }
    await route();
  });
  document.querySelector<HTMLInputElement>("#budget")?.addEventListener("input", (ev) => {
    const el = ev.currentTarget as HTMLInputElement;
    const save = document.querySelector<HTMLButtonElement>('#settings-form button[type="submit"]');
    if (save) save.disabled = shouldDisableSave(Number(el.value), Number(el.dataset.committed ?? 0));
  });
}

route();
window.setInterval(() => {
  if (window.location.pathname === "/") void route();
}, 60_000);
