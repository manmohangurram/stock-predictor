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

export function wireCallRows(): void {
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
