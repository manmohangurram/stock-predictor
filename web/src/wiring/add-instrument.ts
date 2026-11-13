import { api, type Hit } from "../api";
import { escapeHtml } from "../format";

/// One dialog, opened from the header or ⌘K/Ctrl-K, chosen with ↑↓ ⏎.
export function wireAddDialog(): void {
  const dialog = document.querySelector<HTMLDialogElement>("#add-dialog");
  const body = document.querySelector<HTMLElement>(".add-dialog-body");
  if (!dialog || !body) return;
  if (!dialog.contains(body)) dialog.append(body);
  body.hidden = false;

  const input = dialog.querySelector<HTMLInputElement>("#search")!;
  const box = dialog.querySelector<HTMLDivElement>("#results")!;
  let timer: number | undefined;
  let hits: Hit[] = [];
  let local: Hit[] = [];
  let active = 0;

  const SHOWN = 6;
  /**
   * Only Indian listings. The book is denominated in rupees and follows NSE/BSE rules (T+1,
   * circuit bands), so a NYSE ADR would be priced in dollars and the app would read that price as
   * rupees — a +11,000% "return" that is really a currency mismatch.
   */
  const indian = (h: Hit) =>
    h.class === "mutual_fund" || h.venue === "NSE" || h.venue === "Bombay" || /\.(NS|BO)$/.test(h.code);

  const groups = (list: Hit[]) => {
    const kept = list.filter(indian);
    const stock = kept.filter((h) => h.class !== "mutual_fund");
    const fund = kept.filter((h) => h.class === "mutual_fund");
    return [
      { label: "Stocks", items: stock },
      { label: "Mutual funds", items: fund },
    ].filter((g) => g.items.length);
  };

  const draw = () => {
    const parts: string[] = [];
    let index = 0;
    local = hits.filter(indian);
    const offered = local.slice(0, SHOWN);
    for (const g of groups(offered)) {
      parts.push(`<div class="dlg-group">${g.label}</div>`);
      for (const h of g.items) {
        const i = index++;
        parts.push(`<div class="dlg-opt ${i === active ? "sel" : ""}" role="option"
          aria-selected="${i === active}" data-i="${i}" tabindex="-1">
          <b>${escapeHtml(h.name)}</b>
          <span class="meta">${escapeHtml(h.code)} · ${h.class === "mutual_fund" ? "fund" : "stock"}</span>
        </div>`);
      }
    }
    box.innerHTML = parts.length
      ? `${parts.join("")}${
          local.length > SHOWN
            ? `<p class="dlg-more">${local.length - SHOWN} more — keep typing to narrow it</p>`
            : ""
        }`
      : input.value.trim().length >= 2
        ? `<p class="empty">Nothing matches that.</p>`
        : `<p class="empty">Type a name, a ticker, or an AMFI code.</p>`;
    box.querySelectorAll<HTMLElement>(".dlg-opt").forEach((el) => {
      el.addEventListener("click", () => {
        active = Number(el.dataset.i);
        draw();
        void add();
      });
    });
  };

  const add = async () => {
    const hit = local[active];
    if (!hit) return;
    const row = box.querySelector<HTMLElement>(`.dlg-opt[data-i="${active}"]`);
    if (row) row.textContent = `Adding ${hit.code}…`;
    try {
      await api.addInstrument(hit.code, hit.name, hit.class);
      window.location.href = `/i/${encodeURIComponent(hit.code)}`;
    } catch (e) {
      if (row) row.textContent = `Failed: ${String(e)}`;
    }
  };

  const open = () => {
    if (!dialog.open) dialog.showModal();
    input.value = "";
    hits = [];
    active = 0;
    draw();
    input.focus();
  };

  document.querySelectorAll<HTMLElement>("[data-add-open]").forEach((b) => {
    b.addEventListener("click", open);
  });
  document.addEventListener("keydown", (ev) => {
    if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "k") {
      ev.preventDefault();
      open();
    }
  });
  dialog.addEventListener("click", (ev) => {
    if (ev.target === dialog) dialog.close(); // the backdrop
  });
  dialog.addEventListener("close", () => input.blur());

  input.addEventListener("input", () => {
    window.clearTimeout(timer);
    const q = input.value.trim();
    if (q.length < 2) {
      hits = [];
      active = 0;
      draw();
      return;
    }
    timer = window.setTimeout(async () => {
      try {
        hits = await api.search(q);
        active = 0;
        draw();
      } catch {
        box.innerHTML = `<p class="empty">Search unavailable.</p>`;
      }
    }, 250);
  });

  input.addEventListener("keydown", (ev) => {
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
      ev.preventDefault();
      const count = Math.min(hits.length, 6);
      if (!count) return;
      active = (active + (ev.key === "ArrowDown" ? 1 : -1) + count) % count;
      draw();
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      void add();
    }
  });
}
