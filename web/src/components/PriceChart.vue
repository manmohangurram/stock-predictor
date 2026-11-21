<script setup lang="ts">
/** Daily close with real axes: price down the left, dates along the bottom, averages where they belong. */
import { computed } from "vue";
import type { Analysis } from "../api";
import { day, money } from "../format";

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
        first: bars[0].ts,
        last: bars[bars.length - 1].ts,
    };
});

const w = 900;
const h = 196;
const left = 68;
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
    const x = (i: number): number =>
        left + (i * (w - left - right)) / (closes.length - 1);
    const y = (v: number): number =>
        top + ((hi - v) * (h - top - bottom)) / span;
    const path = closes
        .map(
            (c, i) =>
                `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(c).toFixed(1)}`,
        )
        .join(" ");
    const ticks = [0, 1, 2, 3].map((n) => lo + ((hi - lo) * n) / 3);
    const marks = [0, 0.25, 0.5, 0.75, 1].map((f) =>
        Math.round(f * (closes.length - 1)),
    );
    return { closes, fund, target, x, y, path, ticks, marks };
});

const refs = computed(() => {
    const g = geo.value;
    if (!g) return [];
    const at = (v: number, label: string, tone: string) => ({
        v,
        label,
        tone,
        y: g.y(v).toFixed(1),
    });
    if (g.fund)
        return g.target === null
            ? []
            : [at(g.target, "TARGET", "var(--stamp-oxide)")];
    const ind = a.value.indicators;
    return ind
        ? [
              at(ind.sma50, "SMA50", "var(--ink-tertiary)"),
              at(ind.sma200, "SMA200", "var(--stamp-oxide)"),
          ]
        : [];
});
</script>

<template>
    <p v-if="!geo" class="empty">
        No price history stored yet — open this instrument once the next poll
        has run.
    </p>
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
                        stop-color="var(--ink)"
                        stop-opacity=".14"
                    />
                    <stop offset="1" stop-color="var(--ink)" stop-opacity="0" />
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
        </svg>
        <figcaption class="spark-cap">
            <span>{{ day(spark.first) }}</span><span>{{
                day(spark.last)
            }}</span>
        </figcaption>
    </figure>
    <figure v-else class="chart">
        <svg
            :viewBox="`0 0 ${w} ${h}`"
            role="img"
            :aria-label="`${a.instrument.name}, close price, last ${geo.closes.length} sessions`"
        >
            <template v-for="(v, n) in geo.ticks" :key="`g${n}`">
                <line
                    :x1="left"
                    :x2="w - right"
                    :y1="geo.y(v).toFixed(1)"
                    :y2="geo.y(v).toFixed(1)"
                    stroke="var(--rule-subtle)"
                    stroke-dasharray="2 4"
                />
                <text
                    :x="left - 8"
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
                :x1="left"
                :x2="w - right"
                :y1="h - bottom"
                :y2="h - bottom"
                stroke="var(--rule)"
            />
            <line
                :x1="left"
                :x2="left"
                :y1="top"
                :y2="h - bottom"
                stroke="var(--rule)"
            />
            <path
                :d="`${geo.path} L${geo.x(geo.closes.length - 1).toFixed(1)},${h - bottom} L${left},${h - bottom} Z`"
                fill="var(--ink)"
                opacity=".06"
            />
            <path
                :d="geo.path"
                fill="none"
                stroke="var(--ink)"
                stroke-width="1.75"
            />
            <template v-for="(r, n) in refs" :key="`r${n}`">
                <line
                    :x1="left"
                    :x2="w - right"
                    :y1="r.y"
                    :y2="r.y"
                    :stroke="r.tone"
                    stroke-dasharray="4 4"
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
    </figure>
</template>

<style scoped>
.spark {
    margin: 0;
    padding: 6px 12px 0;
}
.spark svg {
    width: 100%;
    height: 110px;
    display: block;
}
.spark-cap {
    display: flex;
    justify-content: space-between;
    font-family: var(--font-ledger);
    font-size: 9px;
    color: var(--ink-tertiary);
    margin-top: 4px;
}
</style>
