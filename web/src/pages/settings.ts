import type { Settings } from "../api";
import { escapeHtml, money } from "../format";

/** The one rule the form enforces client-side; the server enforces it again. */
export const shouldDisableSave = (budget: number, committed: number): boolean =>
  !Number.isFinite(budget) || budget < committed;

export function renderSettings(s: Settings): string {
  const below = shouldDisableSave(s.budget, s.committed);
  const currencies = ["INR", "USD", "EUR", "GBP"];
  return `<div class="shell">
  <header class="strip detail-head">
    <div class="who">
      <a class="back" href="/">← Ledger</a>
      <h1>Settings</h1>
      <span class="lbl">Budget, currency and the model key</span>
    </div>
    <div><span class="lbl">Available</span><b class="num total">${money(s.available, 0)}</b></div>
    <div><span class="lbl">Committed</span><b class="num">${money(s.committed, 0)}</b></div>
    <div><span class="lbl">Budget</span><b class="num">${money(s.budget, 0)}</b></div>
  </header>
  <div class="body">
    <section class="main">
      <form id="settings-form" class="trade-form">
        <div class="field">
          <label class="lbl" for="budget">Budget</label>
          <input id="budget" name="budget" type="number" min="0" step="1"
                 value="${Math.round(s.budget)}" data-committed="${s.committed}" />
          <p class="help">Committed is ${money(s.committed, 0)} ${s.currency}. The budget cannot go below it.</p>
          ${below ? `<p class="guard">Committed is ${money(s.committed, 0)} — the budget cannot go below it.</p>` : ""}
        </div>
        <div class="field">
          <label class="lbl" for="trading_budget">Trading pot</label>
          <input id="trading_budget" name="trading_budget" type="number" min="0" step="1"
                 value="${Math.round(s.trading_budget)}" />
          <p class="help">
            Separate capital for the trading mode. ${money(s.trading_available, 0)} ${s.currency} free.
            It never draws on the long-term budget, and the long-term budget never draws on it.
          </p>
        </div>
        <div class="field">
          <label class="lbl" for="currency">Currency</label>
          <select id="currency" name="currency" ${s.currency_locked ? "disabled" : ""}>
            ${currencies.map((c) => `<option value="${c}" ${c === s.currency ? "selected" : ""}>${c}</option>`).join("")}
          </select>
          <p class="help">${
            s.currency_locked
              ? "Currency is fixed once the ledger has entries."
              : "Locked permanently after the first recorded transaction."
          }</p>
        </div>
        <div class="field">
          <label class="lbl" for="deepseek_key">DeepSeek API key</label>
          <input id="deepseek_key" name="deepseek_key" type="password" autocomplete="off"
                 placeholder="${s.deepseek_configured ? "•••••••• configured — leave blank to keep" : "sk-…"}" />
          <p class="help">${
            s.deepseek_configured
              ? "Configured. The key is never shown again, here or over the API."
              : "Without a key the rules run alone and no model call is made."
          }</p>
        </div>
        <fieldset class="field-group">
          <legend class="lbl">Phone notifications (ntfy)</legend>
          <div class="field">
            <label class="lbl" for="ntfy_topic">Topic</label>
            <input id="ntfy_topic" name="ntfy_topic" value="${escapeHtml(s.ntfy_topic)}"
                   placeholder="stock-predicter-7f3k9m2q" autocomplete="off" />
            <p class="help">
              Subscribe to this topic in the ntfy app. Anyone who knows the name can read it —
              make it long and random.
            </p>
          </div>
          <div class="field">
            <label class="lbl" for="ntfy_url">Server</label>
            <input id="ntfy_url" name="ntfy_url" value="${escapeHtml(s.ntfy_url)}"
                   placeholder="https://ntfy.sh" autocomplete="off" />
            <p class="help">Use your own ntfy instance here if you self-host one.</p>
          </div>
          <div class="field">
            <label class="lbl" for="ntfy_token">Access token</label>
            <input id="ntfy_token" name="ntfy_token" type="password" autocomplete="off"
                   placeholder="${s.ntfy_token_configured ? "•••••••• configured — leave blank to keep" : "tk_… (only for a protected topic)"}" />
            <p class="help">${
              s.ntfy_token_configured
                ? "Configured. The token is never shown again, here or over the API."
                : "Optional. Needed only when the topic requires authentication."
            }</p>
          </div>
          <div class="field">
            <label class="lbl" for="public_url">Link from the notification</label>
            <input id="public_url" name="public_url" value="${escapeHtml(s.public_url)}"
                   placeholder="http://192.168.1.10:8080" autocomplete="off" />
            <p class="help">Tapping the push opens this host. Leave blank for no link.</p>
          </div>
          <div class="row">
            <button class="btn" type="button" id="test-notify">Send a test notification</button>
            <span class="help" id="notify-status" role="status"></span>
          </div>
        </fieldset>

        <div class="row">
          <button class="btn" type="submit" ${below ? "disabled" : ""}>Save</button>
        </div>
      </form>
    </section>
    <aside class="side">
      <div class="lbl">What this controls</div>
      <div class="pair"><span>Hard ceiling</span><span class="num">${money(s.budget, 0)}</span></div>
      <div class="pair"><span>Spent and reserved</span><span class="num">${money(s.committed, 0)}</span></div>
      <div class="pair"><span>Free to deploy</span><span class="num">${money(s.available, 0)}</span></div>
      <p class="cites" style="margin-top:12px">
        A budget change is recorded as a ledger entry, so the history of the ceiling stays auditable.
      </p>
    </aside>
  </div>
</div>`;
}
