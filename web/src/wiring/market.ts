import { api } from "../api";
import { dayTime, escapeHtml } from "../format";


/**
 * The rail's market chip: open, closed, or probably a holiday.
 *
 * "Probably a holiday" is a reading of the data, not a calendar — a weekday inside session hours
 * with no prices recorded today. Saying "open" while every poll returns yesterday's close would be
 * worse than saying nothing.
 */
export async function wireMarket(): Promise<void> {
  // Two homes, one reading: the rail's foot on a wide screen, under the title on a phone.
  const slots = [...document.querySelectorAll<HTMLElement>("[data-market]")];
  if (!slots.length) return;
  try {
    const m = await api.market();
    const open = m.state === "open";
    const label = open ? "Market open" : m.state === "holiday" ? "Market shut" : "Market closed";
    const when = open
      ? m.closes_at
        ? `Closes ${dayTime(m.closes_at)}`
        : ""
      : m.next_open_at
        ? `Opens ${dayTime(m.next_open_at)}`
        : "";
    const body = `<span class="lbl"><span class="dot" aria-hidden="true"></span>${escapeHtml(label)}</span>
      <span class="reason">${escapeHtml(when)}</span>`;
    for (const el of slots) {
      el.dataset.state = m.state;
      el.title = [
        label,
        open ? "trading" : m.reason,
        `${m.session}`,
        m.state === "holiday" ? "no prices today — the holiday calendar is not tracked" : "",
      ]
        .filter(Boolean)
        .join(" · ");
      el.innerHTML = body;
    }
  } catch {
    for (const el of slots) el.textContent = "";
  }
}
