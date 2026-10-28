import { api, dashboardCached, type Hit } from "./api";
import { renderCalls } from "./pages/calls";
import { qtyLimit, renderInstrument } from "./pages/instrument";
import { renderLedger } from "./pages/ledger";
import { renderPositions } from "./pages/positions";
import { renderSettings, shouldDisableSave } from "./pages/settings";

const app = document.querySelector<HTMLDivElement>("#app")!;
let dashboardEtag: string | null = null;
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
      // Deep link into a section: the element only exists after this render.
      if (location.hash) {
        document.querySelector(location.hash)?.scrollIntoView({ block: "start" });
      }
    } else if (path === "/positions") {
      app.innerHTML = renderPositions(await api.dashboard());
      wireRows();
    } else if (path === "/calls") {
      app.innerHTML = renderCalls(await api.calls());
    } else if (path.startsWith("/i/")) {
      const symbol = decodeURIComponent(path.slice(3));
      // One request: the analysis payload carries the instrument, its signals and its fills.
      const analysis = await api.analysis(symbol);
      app.innerHTML = renderInstrument(analysis, analysis, analysis.trades);
      wireTradeForm(symbol);
      wireTargetProfit(symbol);
    } else {
      const mode = (new URLSearchParams(location.search).get("mode") as
        | "long"
        | "trade"
        | null) ?? "long";
      // One request: the dashboard payload carries the rail's recent fills.
      const res = await dashboardCached(mode, dashboardEtag);
      if (res.notModified) return; // nothing changed: keep the DOM, and the user's open rows
      dashboardEtag = res.etag ?? null;
      const d = res.data!;
      app.innerHTML = nav + renderLedger(d, d.recent_trades);
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
  const sideSelect = form?.querySelector<HTMLSelectElement>('select[name="side"]');
  const qtyInput = form?.querySelector<HTMLInputElement>('input[name="qty"]');

  // Cap the quantity only while selling. Applying it to a buy made the browser reject any
  // purchase larger than the current holding before the request was ever sent.
  const syncLimit = () => {
    if (!sideSelect || !qtyInput) return;
    const held = qtyInput.dataset.held ? Number(qtyInput.dataset.held) : null;
    const limit = qtyLimit(sideSelect.value as "buy" | "sell", held);
    if (limit === null) qtyInput.removeAttribute("max");
    else qtyInput.max = String(limit);
    qtyInput.setCustomValidity("");
  };
  sideSelect?.addEventListener("change", syncLimit);
  syncLimit();
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const side = String(data.get("side")) as "buy" | "sell";
    const mode = (String(data.get("mode") ?? "long") as "long" | "trade");
    const qty = Number(data.get("qty"));
    const price = Number(data.get("price"));
    const at = String(data.get("at") ?? "").trim();
    if (!(qty > 0) || !(price > 0)) {
      if (status) status.textContent = "Quantity and price must be positive.";
      return;
    }
    if (status) status.textContent = "Recording…";
    try {
      const res = await api.recordTrade(symbol, side, qty, price, at || undefined, mode);
      if (status) {
        status.textContent = `Recorded. Cash ${Math.round(res.cash)}, realised ${Math.round(res.realised)}.`;
      }
      await route();
    } catch (e) {
      if (status) status.textContent = String(e);
    }
  });
}

function readLimits(data: FormData, mode: "long" | "trade") {
  const num = (name: string) => Number(data.get(`${mode}_${name}`) ?? 0);
  return {
    min_profit_pct: num("min_profit_pct"),
    max_profit_pct: num("max_profit_pct"),
    min_trade_amount: num("min_trade_amount"),
    max_trade_amount: num("max_trade_amount"),
    max_instrument_pct: num("max_instrument_pct"),
  };
}

function wireTargetProfit(symbol: string): void {
  const form = document.querySelector<HTMLFormElement>(".target-form");
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const raw = String(new FormData(form).get("pct") ?? "").trim();
    const pct = raw === "" ? null : Number(raw);
    if (pct !== null && (!Number.isFinite(pct) || pct <= 0)) {
      window.alert("Enter a positive percentage, or leave it blank.");
      return;
    }
    try {
      await api.setTargetProfit(symbol, pct);
      await route();
    } catch (e) {
      window.alert(String(e));
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
    const llmKey = String(data.get("llm_key") ?? "").trim();
    const trading = Number(data.get("trading_budget"));
    const ntfyToken = String(data.get("ntfy_token") ?? "").trim();
    const patch = {
      budget,
      currency,
      trading_budget: trading,
      ntfy_url: String(data.get("ntfy_url") ?? "").trim(),
      ntfy_topic: String(data.get("ntfy_topic") ?? "").trim(),
      public_url: String(data.get("public_url") ?? "").trim(),
      llm_provider: String(data.get("llm_provider") ?? "deepseek"),
      llm_model: String(data.get("llm_model") ?? "").trim(),
      llm_base_url: String(data.get("llm_base_url") ?? "").trim(),
      limits_long: readLimits(data, "long"),
      limits_trade: readLimits(data, "trade"),
      ...(ntfyToken ? { ntfy_token: ntfyToken } : {}),
      ...(llmKey ? { llm_key: llmKey } : {}),
    };
    const res = await api.putSettings(patch);
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
  document.querySelector<HTMLButtonElement>("#test-notify")?.addEventListener("click", async (ev) => {
    const button = ev.currentTarget as HTMLButtonElement;
    const status = document.querySelector<HTMLElement>("#notify-status");
    button.disabled = true;
    if (status) status.textContent = "Sending…";
    const res = await api.testNotify();
    button.disabled = false;
    if (status) {
      status.textContent = res.ok
        ? "Sent. Check your phone."
        : `Failed: ${res.body.error ?? "unknown error"}`;
    }
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
