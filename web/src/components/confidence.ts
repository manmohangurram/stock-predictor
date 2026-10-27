/**
 * How sure the model said it was. Shape carries the level (one, two or three filled bars) and
 * colour only reinforces it — never the other way round, so it survives colour blindness.
 *
 * A signal with no confidence did not get one from the model, so nothing is drawn. Rendering an
 * empty gauge would read as "scored zero", which is a different and false claim.
 */
export type Level = "low" | "medium" | "high" | null;

const FILLED: Record<"low" | "medium" | "high", number> = { low: 1, medium: 2, high: 3 };

export const levelOf = (raw: string | null | undefined): Level => {
  const v = (raw ?? "").trim().toLowerCase();
  return v === "low" || v === "medium" || v === "high" ? v : null;
};

export const hasConfidence = (raw: string | null | undefined): boolean => levelOf(raw) !== null;

/** Empty string when there is no score: absence is the honest rendering. */
export function renderConfidence(raw: string | null | undefined): string {
  const level = levelOf(raw);
  if (!level) return "";
  const filled = FILLED[level];
  const bars = [0, 1, 2].map((i) => `<i data-on="${i < filled}"></i>`).join("");
  return `<span class="conf" data-level="${level}" title="Confidence: ${level}"
    role="img" aria-label="Confidence: ${level}">${bars}</span>`;
}

/** Words, for the decision card. Empty when the model gave no score. */
export function confidenceLabel(raw: string | null | undefined): string {
  const level = levelOf(raw);
  return level ? `${level} confidence` : "";
}
