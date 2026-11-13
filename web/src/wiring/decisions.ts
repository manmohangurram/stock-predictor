import { api } from "../api";

/// Approve posts the fill you typed; reject records a reason.
export function wireDecisionActions(reload: () => Promise<void>): void {
  // Both the ledger and the instrument page render the same actions, so this works on either.
  const card = document.body;
  if (!card.querySelector("[data-approve]")) return;
  const field = (id: string, name: string) =>
    card.querySelector<HTMLInputElement>(`[data-approve-form="${id}"] [name="${name}"]`);
  const status = (id: string) => card.querySelector<HTMLElement>(`[data-status="${id}"]`);

  card.querySelectorAll<HTMLButtonElement>("[data-approve]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.approve!;
      const form = card.querySelector<HTMLFormElement>(`[data-approve-form="${id}"]`);
      if (!form) return;
      form.hidden = !form.hidden;
      button.setAttribute("aria-expanded", String(!form.hidden));
    });
  });
  card.querySelectorAll<HTMLButtonElement>("[data-reject]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.reject!;
      const form = card.querySelector<HTMLFormElement>(`[data-reject-form="${id}"]`);
      if (!form) return;
      form.hidden = !form.hidden;
    });
  });
  card.querySelectorAll<HTMLButtonElement>("[data-cancel]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.cancel!;
      const approve = card.querySelector<HTMLFormElement>(`[data-approve-form="${id}"]`);
      const reject = card.querySelector<HTMLFormElement>(`[data-reject-form="${id}"]`);
      if (approve) approve.hidden = true;
      if (reject) reject.hidden = true;
    });
  });
  card.querySelectorAll<HTMLFormElement>("[data-approve-form]").forEach((form) => {
    form.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const id = Number(form.dataset.approveForm);
      const el = status(String(id));
      if (el) el.textContent = "Posting…";
      try {
        // A plan posts one fill per leg; a single decision posts its own.
        const plan = form.dataset.plan;
        const fills = plan
          ? plan.split(",").map((leg) => {
              const read = (n: string) =>
                Number(form.querySelector<HTMLInputElement>(`[name="${n}-${leg}"]`)?.value);
              // A fund is bought with rupees at a NAV; the book stores units, so divide.
              const amount = read("amount");
              const price = read("price");
              const units = Number.isFinite(amount) && amount > 0 && price > 0 ? amount / price : read("qty");
              return { id: Number(leg), qty: units, price };
            })
          : undefined;
        await api.approveSignal(id, {
          qty: fills ? undefined : Number(field(String(id), "qty")?.value),
          price: fills ? undefined : Number(field(String(id), "price")?.value),
          at: field(String(id), "at")?.value || undefined,
          fills,
        });
        await reload();
      } catch (e) {
        if (el) el.textContent = String(e);
      }
    });
  });
  card.querySelectorAll<HTMLFormElement>("[data-reject-form]").forEach((form) => {
    form.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const id = Number(form.dataset.rejectForm);
      const reason = form.querySelector<HTMLInputElement>('[name="reason"]')?.value ?? "";
      const el = status(String(id));
      if (el) el.textContent = "Rejecting…";
      try {
        await api.rejectSignal(id, reason || undefined);
        await reload();
      } catch (e) {
        if (el) el.textContent = String(e);
      }
    });
  });
}

/**
 * Fills are correctable. The panel opens in place of the row it corrects, so you can see the
 * number you are changing next to the one you are changing it to.
 */
