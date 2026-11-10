import { renderExchangeDetail } from "../pages/calls";
import { api } from "../api";

const openCalls = new Set<number>();

async function openCall(id: number): Promise<void> {
  const cell = document.querySelector<HTMLTableCellElement>(`#call-${id} td`);
  const row = document.querySelector<HTMLTableRowElement>(`#call-${id}`);
  const button = document.querySelector<HTMLButtonElement>(`.view-call[data-id="${id}"]`);
  if (!cell || !row) return;
  row.hidden = false;
  button?.setAttribute("aria-expanded", "true");
  cell.innerHTML = `<p class="empty">Reading the stored exchange…</p>`;
  try {
    cell.innerHTML = renderExchangeDetail(await api.callDetail(id));
  } catch (e) {
    cell.innerHTML = `<p class="empty">Could not read call ${id}: ${String(e)}</p>`;
  }
}

/** Copy, for the one reader who wants the JSON in an editor rather than on a screen. */
function wireCopy(): void {
  document.addEventListener("click", async (event) => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button.copy");
    if (!button) return;
    // The visible <pre> is truncated on purpose; the full text sits beside it, hidden.
    const source = button
      .closest(".call-block")
      ?.querySelector<HTMLElement>(".copy-src")?.textContent;
    if (!source) return;
    try {
      await navigator.clipboard.writeText(source);
      button.dataset.copied = "true";
      button.title = `Copied ${source.length.toLocaleString("en-IN")} characters`;
      setTimeout(() => {
        delete button.dataset.copied;
        button.title = "Copy all of it";
      }, 1400);
    } catch (e) {
      button.title = `Could not copy: ${String(e)}`;
    }
  });
}

export function wireCallRows(): void {
  wireCopy();
  // ?call=12 opens that row directly, so a specific exchange can be linked to.
  const wanted = new URLSearchParams(location.search).get("call");
  document.querySelectorAll<HTMLButtonElement>(".view-call").forEach((button) => {
    button.addEventListener("click", async () => {
      const id = Number(button.dataset.id);
      const row = document.querySelector<HTMLTableRowElement>(`#call-${id}`);
      if (!row) return;
      if (!row.hidden) {
        row.hidden = true;
        button.setAttribute("aria-expanded", "false");
        openCalls.delete(id);
        return;
      }
      openCalls.add(id);
      await openCall(id);
    });
    if (wanted && Number(wanted) === Number(button.dataset.id)) button.click();
  });
  // Re-open what was open before this render.
  for (const id of openCalls) {
    if (document.querySelector(`#call-${id}`)) void openCall(id);
  }
}
