/**
 * How sure the model said it was. Shape carries the level (one, two or three filled bars) and
 * colour only reinforces it — never the other way round, so it survives colour blindness.
 * A signal with no confidence means the rules produced it with no model judgement behind it.
 */
export type Level = "low" | "medium" | "high" | null;

const FILLED: Record<"low" | "medium" | "high", number> = { low: 1, medium: 2, high: 3 };

export const levelOf = (raw: string | null | undefined): Level => {
  const v = (raw ?? "").trim().toLowerCase();
  return v === "low" || v === "medium" || v === "high" ? v : null;
};

export function renderConfidence(raw: string | null | undefined): string {
  const level = levelOf(raw);
  if (!level) {
    return `<span class="conf" data-level="rules" title="Rules only — no model judgement"
      role="img" aria-label="Confidence: rules only, no model judgement"
      ><i></i><i></i><i></i></span>`;
  }
  const filled = FILLED[level];
  const bars = [0, 1, 2]
    .map((i) => `<i data-on="${i < filled}"></i>`)
    .join("");
  return `<span class="conf" data-level="${level}" title="Confidence: ${level}"
    role="img" aria-label="Confidence: ${level}">${bars}</span>`;
}

/** The same thing with words, for places with room for them. */
export function confidenceLabel(raw: string | null | undefined): string {
  const level = levelOf(raw);
  return level ? `${level} confidence` : "rules only";
}
