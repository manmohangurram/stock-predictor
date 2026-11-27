import { api } from "../api";

/**
 * The price on the exchange now: the newest 15-minute print, asked live and never stored. A
 * decision written before the open carries yesterday's close, so the card shows both.
 */
export async function livePrice(symbol: string): Promise<number | null> {
  try {
    const r = await api.chart(symbol, "1d");
    const last = r.bars[r.bars.length - 1];
    return last ? last.close : null;
  } catch {
    return null;
  }
}
