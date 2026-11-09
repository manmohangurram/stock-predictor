/**
 * The stored request/response, rendered so a person can read it.
 *
 * The bodies are whatever the provider was sent and sent back, so this never assumes a shape:
 * it looks for the common ones (OpenAI-style `messages`, Gemini `candidates`) and otherwise shows
 * the JSON. Raw is always one click away.
 */

export interface CallDetail {
  id: number;
  ts: string;
  endpoint: string;
  purpose: string | null;
  round: number | null;
  trigger: string | null;
  decision_reason: string | null;
  instrument_name: string | null;
  instrument_symbol: string | null;
  prompt_template: string | null;
  completion_tokens: number | null;
  request_json: string | null;
  response_json: string | null;
  prompt_tokens: number | null;
  validator_verdict: string | null;
  error: string | null;
}

interface Block {
  label: string;
  text: string;
}

const parse = (raw: string | null): unknown => {
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
};

const asRecord = (v: unknown): Record<string, unknown> | null =>
  v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;

/** The prompt as sent: role by role, which is how it was written. */
export function promptBlocks(requestJson: string | null): Block[] {
  const body = asRecord(parse(requestJson));
  if (!body) return [];
  const messages = body.messages ?? body.contents ?? body.input;
  if (Array.isArray(messages)) {
    return messages.flatMap((m, i) => {
      const rec = asRecord(m);
      if (!rec) return [];
      const role = String(rec.role ?? rec.type ?? `message ${i + 1}`);
      const content = rec.content ?? rec.parts ?? rec.text;
      const text =
        typeof content === "string"
          ? content
          : Array.isArray(content)
            ? content
                .map((part) => {
                  const p = asRecord(part);
                  return typeof part === "string" ? part : String(p?.text ?? JSON.stringify(part));
                })
                .join("\n")
            : JSON.stringify(content ?? "");
      return [{ label: role, text }];
    });
  }
  return [];
}

/** The reply as received: content if it parses, the raw text otherwise. */
export function replyText(responseJson: string | null): string {
  const body = asRecord(parse(responseJson));
  if (!body) return typeof parse(responseJson) === "string" ? String(parse(responseJson)) : "";
  // OpenAI-compatible, and DeepSeek which copies it.
  const choice = Array.isArray(body.choices) ? asRecord(body.choices[0]) : null;
  const message = choice ? asRecord(choice.message) : null;
  if (message && typeof message.content === "string") return message.content;
  // Gemini.
  const candidate = Array.isArray(body.candidates) ? asRecord(body.candidates[0]) : null;
  const parts = candidate ? asRecord(candidate.content)?.parts : null;
  if (Array.isArray(parts)) {
    return parts.map((p) => String(asRecord(p)?.text ?? "")).join("");
  }
  if (typeof body.content === "string") return body.content;
  if (body.error) return JSON.stringify(body.error, null, 2);
  return "";
}

/** What the model was told it could do, if anything. */
export function declaredTools(requestJson: string | null): string[] {
  const body = asRecord(parse(requestJson));
  const tools = body?.tools;
  if (!Array.isArray(tools)) return [];
  return tools.map((t) => {
    const rec = asRecord(t);
    const fn = rec?.function ? asRecord(rec.function) : null;
    return String(fn?.name ?? rec?.name ?? rec?.type ?? "tool");
  });
}

export function pretty(raw: string | null, max = 20000): string {
  const value = parse(raw);
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return text.length > max ? `${text.slice(0, max)}\n… truncated in the view; the database has it all` : text;
}

/** The model's answer, once its JSON is understood. */
/** One leg of a plan: the reply's unit when the model answered with several moves. */
export interface ReplyMove {
  action: string;
  symbol: string;
  qty: number | null;
  price: number | null;
  stop: number | null;
  target: number | null;
  why: string | null;
}

export interface Reply {
  /** "BUY" | "SELL" | "PASS" | "NEED_MORE" — or null when the body was not that shape. */
  verdict: string | null;
  entry: number | null;
  stop: number | null;
  target: number | null;
  qty: number | null;
  confidence: string | null;
  rationale: string | null;
  /** PASS carries "why"; NEED_MORE carries what it wants. */
  why: string | null;
  wantsHeadlines: number[];
  wantsHistory: boolean;
  cites: number[];
  /** A plan's legs, in the order the model gave them. Empty for a single decision. */
  moves: ReplyMove[];
  /** Anything else the body said, so nothing is silently dropped. */
  rest: Record<string, unknown> | null;
}

const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);
const str = (v: unknown): string | null => (typeof v === "string" && v.trim() ? v.trim() : null);

/** `{"action":"decide","signal":{…}}` and its bare variants, from one parser. */
export function parseReply(content: string): Reply {
  const empty: Reply = {
    verdict: null, entry: null, stop: null, target: null, qty: null, confidence: null,
    rationale: null, why: null, wantsHeadlines: [], wantsHistory: false, cites: [],
    moves: [], rest: null,
  };
  let body: unknown;
  try {
    body = JSON.parse(content);
  } catch {
    return empty;
  }
  const outer = asRecord(body);
  if (!outer) return empty;
  const decision = asRecord(outer.signal) ?? outer;
  const action = str(decision.action) ?? str(outer.action);
  const verdict =
    action === null ? null : action === "decide" ? "DECIDE" : action.toUpperCase();
  const confidence = asRecord(decision.confidence) ? null : str(decision.confidence);
  const more = asRecord(decision.need_more) ?? asRecord(outer.need_more) ?? asRecord(outer.more);
  const headlines = more?.headlines ?? decision.headlines;
  return {
    verdict,
    entry: num(decision.entry),
    stop: num(decision.stop),
    target: num(decision.target),
    qty: num(decision.qty),
    confidence: typeof confidence === "string" ? confidence : null,
    rationale: str(decision.rationale) ?? str(outer.rationale),
    why: str(decision.why) ?? str(outer.why) ?? str(outer.reason) ?? str(outer.pass_reason),
    wantsHeadlines: Array.isArray(headlines)
      ? headlines
          .map((h) => (typeof h === "number" ? h : num(asRecord(h)?.headline_id)))
          .filter((n): n is number => n !== null)
      : [],
    wantsHistory: Boolean(more?.price_history ?? more?.history),
    cites: Array.isArray(decision.cites)
      ? decision.cites.filter((c): c is number => typeof c === "number")
      : [],
    moves: Array.isArray(outer.moves)
      ? outer.moves
          .map(asRecord)
          .filter((m): m is Record<string, unknown> => m !== null)
          .map((m) => ({
            action: (str(m.action) ?? "").toUpperCase(),
            symbol: str(m.symbol) ?? "",
            qty: num(m.qty),
            price: num(m.price),
            stop: num(m.stop),
            target: num(m.target),
            why: str(m.why),
          }))
      : [],
    rest: outer,
  };
}
