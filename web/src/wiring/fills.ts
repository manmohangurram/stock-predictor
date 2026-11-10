import { api } from "../api";
import { qtyLimit } from "../pages/instrument";

export function wireFillEdits(reload: () => Promise<void>): void {
  const panel = document.querySelector<HTMLElement>("#record-panel");
  document.querySelectorAll<HTMLButtonElement>("[data-record-toggle]").forEach((button) => {
    button.addEventListener("click", () => {
      if (!panel) return;
      panel.hidden = !panel.hidden;
      button.setAttribute("aria-expanded", String(!panel.hidden));
      if (!panel.hidden) panel.querySelector<HTMLInputElement>('input[name="qty"]')?.focus();
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-edit-fill]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.editFill!;
      const row = document.querySelector<HTMLElement>(`[data-fill-edit="${id}"]`);
      if (!row) return;
      row.hidden = !row.hidden;
      button.setAttribute("aria-expanded", String(!row.hidden));
      if (!row.hidden) row.querySelector<HTMLInputElement>('input[name="qty"]')?.focus();
    });
  });
  document.querySelectorAll<HTMLButtonElement>("[data-cancel-fill]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.cancelFill!;
      const row = document.querySelector<HTMLElement>(`[data-fill-edit="${id}"]`);
      if (row) row.hidden = true;
    });
  });

  // Delete asks once, in place: the button becomes the question, and clicking it again acts.
  document.querySelectorAll<HTMLButtonElement>("[data-delete-fill]").forEach((button) => {
    const original = button.textContent ?? "Delete this fill";
    button.addEventListener("click", async () => {
      const id = Number(button.dataset.deleteFill);
      if (button.dataset.armed !== "1") {
        button.dataset.armed = "1";
        button.textContent = "Delete? Click again";
        window.setTimeout(() => {
          if (button.dataset.armed === "1") {
            button.dataset.armed = "0";
            button.textContent = original;
          }
        }, 4000);
        return;
      }
      button.disabled = true;
      button.textContent = "Deleting…";
      try {
        await api.deleteTrade(id);
        await reload();
      } catch (e) {
        button.disabled = false;
        button.dataset.armed = "0";
        button.textContent = original;
        const status = document.querySelector<HTMLElement>(`[data-fill-status="${id}"]`);
        if (status) status.textContent = String(e);
      }
    });
  });

  document
    .querySelectorAll<HTMLFormElement>("[data-edit-form]")
    .forEach((form) => {
      form.addEventListener("submit", async (ev) => {
        ev.preventDefault();
        const id = Number(form.dataset.editForm);
        const status = document.querySelector<HTMLElement>(`[data-fill-status="${id}"]`);
        const value = (name: string) =>
          form.querySelector<HTMLInputElement>(`[name="${name}"]`)?.value ?? "";
        if (status) status.textContent = "Saving…";
        try {
          await api.editTrade(id, {
            qty: Number(value("qty")),
            price: Number(value("price")),
            at: value("at") || undefined,
          });
          await reload();
        } catch (e) {
          if (status) status.textContent = String(e);
        }
      });
    });
}

export function wireTradeForm(symbol: string, reload: () => Promise<void>): void {
  const form = document.querySelector<HTMLFormElement>(".trade-form");
  const status = document.querySelector<HTMLElement>("#trade-status");
  const sideInputs = form ? Array.from(form.querySelectorAll<HTMLInputElement>('input[name="side"]')) : [];
  const sideValue = (): "buy" | "sell" =>
    (sideInputs.find((r) => r.checked)?.value as "buy" | "sell") ?? "buy";
  const qtyInput = form?.querySelector<HTMLInputElement>('input[name="qty"]');

  // Cap the quantity only while selling. Applying it to a buy made the browser reject any
  // purchase larger than the current holding before the request was ever sent.
  const syncLimit = () => {
    if (!qtyInput) return;
    const held = qtyInput.dataset.held ? Number(qtyInput.dataset.held) : null;
    const limit = qtyLimit(sideValue(), held);
    if (limit === null) qtyInput.removeAttribute("max");
    else qtyInput.max = String(limit);
    qtyInput.setCustomValidity("");
  };
  sideInputs.forEach((r) => r.addEventListener("change", syncLimit));
  syncLimit();
  form?.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const data = new FormData(form);
    const side = sideValue();
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
      await reload();
    } catch (e) {
      if (status) status.textContent = String(e);
    }
  });
}


/** A decision row opens to reveal the whole message; Approve opens it straight onto the form. */
