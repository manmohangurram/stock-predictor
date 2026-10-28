export type StampKind = "buy" | "sell" | "estimate" | "blocked";

/** Stamps are literal uppercase, not CSS-uppercased: the text must exist in the markup. */
export const stamp = (kind: StampKind, label: string): string =>
  `<span class="stamp" data-stamp="${kind}">${label.toUpperCase()}</span>`;
