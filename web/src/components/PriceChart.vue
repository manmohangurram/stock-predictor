<script setup lang="ts">
/** Daily close with real axes: price down the left, dates along the bottom, averages where they belong. */
import { computed } from "vue";
import type { Analysis } from "../api";
import { day, money } from "../format";
import { LineChart } from "lucide-vue-next";
import AppEmpty from "./ui/AppEmpty.vue";

const props = withDefaults(
    defineProps<{ analysis: Analysis; compact?: boolean }>(),
    {
        compact: false,
    },
);
const a = computed(() => props.analysis);

/** The phone's tape: the shape of the move, no axes, stretched to the card. */
const spark = computed(() => {
    const bars = a.value.bars;
    if (bars.length < 2) return null;
    const closes = bars.map((b) => b.close);
    const lo = Math.min(...closes);
    const hi = Math.max(...closes);
    const pad = (hi - lo || lo * 0.02) * 0.08;
    const bottom = lo - pad;
    const span = hi + pad - bottom || 1;
    const W = 1000;
    const H = 240;
    const x = (i: number): number => (i * W) / (closes.length - 1);
    const y = (v: number): number => H - ((v - bottom) * H) / span;
    const line = closes
        .map((c, i) => `${x(i).toFixed(1)},${y(c).toFixed(1)}`)
        .join(" ");
    return {
        area: `0,${H} ${line} ${W},${H}`,
        line,
        lastPct: (y(closes[closes.length - 1]) / H) * 100,
        first: bars[0].ts,
        last: bars[bars.length - 1].ts,
    };
});

const w = 900;
const h = 196;
const right = 18;
const top = 12;
const bottom = 26;

const geo = computed(() => {
    const closes = a.value.bars.map((b) => b.close);
    if (closes.length < 2) return null;
    const fund = a.value.instrument.class === "mutual_fund";
    // A fund's exit is a NAV: the return the owner asked for, on what they paid.
    const priced =
        a.value.position && a.value.instrument.target_profit_pct !== null
            ? a.value.position.avg_cost *
              (1 + a.value.instrument.target_profit_pct / 100)
            : null;
    const target = fund ? priced : null;
    const refs = fund
        ? target === null
            ? []
            : [target]
        : a.value.indicators
          ? [a.value.indicators.sma50, a.value.indicators.sma200]
          : [];
    const rawLo = Math.min(...closes, ...refs);
    const rawHi = Math.max(...closes, ...refs);
    const air = (rawHi - rawLo || rawLo * 0.02) * 0.08;
    const lo = rawLo - air;
    const hi = rawHi + air;
    const span = hi - lo || 1;
    const ticks = [0, 1, 2, 3].map((n) => lo + ((hi - lo) * n) / 3);
    // The gutter is the widest tick, not a fixed 68 units the stretched card turns into ~88px.
    const widest = Math.max(...ticks.map((v) => money(v, 0).length));
    const padLeft = widest * 6 + 14;
    const x = (i: number): number =>
        padLeft + (i * (w - padLeft - right)) / (closes.length - 1);
    const y = (v: number): number =>
        top + ((hi - v) * (h - top - bottom)) / span;
    const path = closes
        .map(
            (c, i) =>
                `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(c).toFixed(1)}`,
        )
        .join(" ");
    const marks = [0, 0.25, 0.5, 0.75, 1].map((f) =>
        Math.round(f * (closes.length - 1)),
    );
    return { closes, fund, target, x, y, path, ticks, marks, padLeft };
});

const avgCost = computed(() => a.value.position?.avg_cost ?? null);

const refs = computed(() => {
    const g = geo.value;
    if (!g) return [];
    const at = (v: number, label: string, tone: string, dash: string) => ({
        v,
        label,
        tone,
        dash,
        y: g.y(v).toFixed(1),
    });
    if (g.fund)
        return g.target === null
            ? []
            : [at(g.target, "TARGET", "var(--stamp-oxide)", "4 4")];
    const out = [];
    if (avgCost.value !== null)
        out.push(at(avgCost.value, "AVG", "var(--ink-secondary)", "4 4"));
    const ind = a.value.indicators;
    if (ind) {
        out.push(at(ind.sma50, "SMA50", "var(--ink-tertiary)", "1 4"));
        out.push(at(ind.sma200, "SMA200", "var(--stamp-oxide)", "1 4"));
    }
    // Two refs a few rupees apart land on one line; spread the labels, not the values.
    const gap = 11;
    const sorted = [...out].sort((p, q) => Number(p.y) - Number(q.y));
    for (let i = 1; i < sorted.length; i++) {
        const prev = Number(sorted[i - 1].y);
        if (Number(sorted[i].y) - prev < gap)
            sorted[i].y = (prev + gap).toFixed(1);
    }
    const over = Number(sorted[sorted.length - 1].y) - (h - 4);
    if (over > 0) for (const r of sorted) r.y = (Number(r.y) - over).toFixed(1);
    return out;
});
</script>

<template>
    <AppEmpty
        v-if="!geo"
        compact
        :icon="LineChart"
        text="No price history stored yet — open this instrument once the next poll has run."
    />
    <figure v-else-if="compact && spark" class="spark">
        <svg
            viewBox="0 0 1000 240"
            preserveAspectRatio="none"
            aria-hidden="true"
        >
            <defs>
                <linearGradient id="spark-fill" x1="0" y1="0" x2="0" y2="1">
                    <stop
                        offset="0"
                        stop-color="var(--brand)"
                        stop-opacity=".22"
                    />
                    <stop
                        offset="1"
                        stop-color="var(--brand)"
                        stop-opacity="0"
                    />
                </linearGradient>
            </defs>
            <polygon :points="spark.area" fill="url(#spark-fill)" />
            <polyline
                :points="spark.line"
                fill="none"
                stroke="var(--ink)"
                stroke-width="2"
                stroke-linejoin="round"
                vector-effect="non-scaling-stroke"
            />
            <circle
                cx="996"
                :cy="spark.lastY"
                r="4"
                fill="var(--brand)"
                stroke="var(--paper-raised)"
                stroke-width="2"
                vector-effect="non-scaling-stroke"
            />
        </svg>
        <span class="spark-dot" :style="{ top: `${spark.lastPct}%` }"></span>
        <figcaption class="spark-cap">
            <span>{{ day(spark.first) }}</span>
            <span v-if="avgCost !== null">avg cost {{ money(avgCost) }}</span>
            <span>{{ day(spark.last) }}</span>
        </figcaption>
    </figure>
    <figure v-else class="chart">
        <svg
            :viewBox="`0 0 ${w} ${h}`"
            role="img"
            :aria-label="`${a.instrument.name}, close price, last ${geo.closes.length} sessions`"
        >
            <defs>
                <linearGradient id="chart-fill" x1="0" y1="0" x2="0" y2="1">
                    <stop
                        offset="0"
                        stop-color="var(--brand)"
                        stop-opacity=".22"
                    />
                    <stop
                        offset="1"
                        stop-color="var(--brand)"
                        stop-opacity="0"
                    />
                </linearGradient>
            </defs>
            <template v-for="(v, n) in geo.ticks" :key="`g${n}`">
                <line
                    :x1="geo.padLeft"
                    :x2="w - right"
                    :y1="geo.y(v).toFixed(1)"
                    :y2="geo.y(v).toFixed(1)"
                    stroke="var(--rule-subtle)"
                    stroke-dasharray="2 4"
                />
                <text
                    :x="geo.padLeft - 8"
                    :y="(geo.y(v) + 3.5).toFixed(1)"
                    text-anchor="end"
                    font-size="10"
                    font-family="var(--font-ledger)"
                    fill="var(--ink-tertiary)"
                >
                    {{ money(v, 0) }}
                </text>
            </template>
            <line
                :x1="geo.padLeft"
                :x2="w - right"
                :y1="h - bottom"
                :y2="h - bottom"
                stroke="var(--rule)"
            />
            <line
                :x1="geo.padLeft"
                :x2="geo.padLeft"
                :y1="top"
                :y2="h - bottom"
                stroke="var(--rule)"
            />
            <path
                :d="`${geo.path} L${geo.x(geo.closes.length - 1).toFixed(1)},${h - bottom} L${geo.padLeft},${h - bottom} Z`"
                fill="url(#chart-fill)"
            />
            <path
                :d="geo.path"
                fill="none"
                stroke="var(--ink)"
                stroke-width="2"
                stroke-linejoin="round"
            />
            <circle
                :cx="geo.x(geo.closes.length - 1).toFixed(1)"
                :cy="geo.y(geo.closes[geo.closes.length - 1]).toFixed(1)"
                r="4"
                fill="var(--brand)"
                stroke="var(--paper-raised)"
                stroke-width="2"
            />
            <template v-for="(r, n) in refs" :key="`r${n}`">
                <line
                    :x1="geo.padLeft"
                    :x2="w - right"
                    :y1="r.y"
                    :y2="r.y"
                    :stroke="r.tone"
                    :stroke-dasharray="r.dash"
                />
                <text
                    :x="w - right - 8"
                    stroke="var(--paper-raised)"
                    stroke-width="3"
                    paint-order="stroke"
                    :y="(Number(r.y) - 5).toFixed(1)"
                    text-anchor="end"
                    font-size="10"
                    font-family="var(--font-ledger)"
                    :fill="r.tone"
                >
                    {{ r.label }} {{ money(r.v, 0) }}
                </text>
            </template>
            <template v-for="(i, n) in geo.marks" :key="`d${n}`">
                <text
                    :x="geo.x(i).toFixed(1)"
                    :y="h - 10"
                    :text-anchor="
                        n === 0
                            ? 'start'
                            : n === geo.marks.length - 1
                              ? 'end'
                              : 'middle'
                    "
                    font-size="10"
                    font-family="var(--font-ledger)"
                    fill="var(--ink-tertiary)"
                >
                    {{ day(a.bars[i].ts) }}
                </text>
            </template>
        </svg>
        <figcaption
            class="chart-cap"
            :style="{ paddingLeft: (geo.padLeft / w) * 100 + '%' }"
        >
            <span><i class="k-line"></i>Close</span>
            <span v-if="avgCost !== null"
                ><i class="k-dash"></i>avg cost {{ money(avgCost) }}</span
            >
            <span v-if="geo.fund && geo.target !== null"
                ><i class="k-dash"></i>target {{ money(geo.target) }}</span
            >
            <span v-if="!geo.fund && a.indicators"
                ><i class="k-dot"></i>SMA50
                {{ money(a.indicators.sma50, 0) }}</span
            >
            <span v-if="!geo.fund && a.indicators"
                ><i class="k-dot ox"></i>SMA200
                {{ money(a.indicators.sma200, 0) }}</span
            >
        </figcaption>
    </figure>
</template>

<style scoped>
.spark {
    margin: 0;
    padding: 4px 16px 0;
    position: relative;
}
.spark svg {
    width: 100%;
    height: 110px;
    display: block;
}
.spark-dot {
    position: absolute;
    right: 14px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--brand);
    box-shadow: 0 0 0 2px var(--paper-raised);
    transform: translateY(-50%);
}
.spark-cap {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-family: var(--font-ledger);
    font-size: 9px;
    color: var(--ink-tertiary);
    margin-top: 4px;
}
.chart-cap {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    padding: 6px 18px 10px 0;
    font-size: 10px;
    color: var(--ink-tertiary);
}
.chart-cap i {
    display: inline-block;
    width: 14px;
    height: 0;
    margin-right: 5px;
    vertical-align: 3px;
    border-top: 2px solid var(--ink);
}
.chart-cap i.k-dash {
    border-top: 1px dashed var(--ink-secondary);
}
.chart-cap i.k-dot {
    border-top: 1px dotted var(--ink-tertiary);
}
.chart-cap i.k-dot.ox {
    border-top-color: var(--stamp-oxide);
}
</style>
