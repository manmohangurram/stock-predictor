<script setup lang="ts">
/**
 * One decision and the two things you can do about it. A plan is several legs read as one
 * argument: approve opens a sign-off sheet with an editable fill per leg, because the price a
 * proposal named is rarely the price you get, and half a swap is worse than none.
 */
import { computed, onMounted, ref } from "vue";
import { api, type Mode, type SignalRow } from "../api";
import { dayTime, floorUnits, money, units } from "../format";
import { renderConfidence } from "../components/confidence";
import { debitOf, legValue, rewardRisk } from "../lib/decision";
import { livePrice } from "../lib/live";

const props = defineProps<{
    legs: SignalRow[];
    plan: boolean;
    mode: Mode;
    /** The purse this decision would spend from. */
    free: number;
}>();
const emit = defineEmits<{ reload: [] }>();

const first = computed(() => props.legs[0]);
const pending = computed(() => first.value.status === "pending");
const approved = computed(() => first.value.status === "approved");
const isFund = computed(
    () => first.value.plan_scope === "fund" || first.value.class === "mutual_fund",
);
const title = computed(() =>
    !props.plan
        ? `${first.value.name || first.value.symbol} · 1 move`
        : isFund.value
          ? "Fund plan"
          : props.mode === "trade"
            ? "Short-term plan"
            : "Stock plan",
);
const planCode = computed(() => (first.value.plan_id ?? "").slice(-6));
const horizon = computed(() => (props.mode === "trade" ? "SHORT" : "LONG"));

const prose = computed(() => {
    const s = first.value;
    return props.plan ? (s.plan_rationale ?? "") : (s.rationale ?? s.reasons[0] ?? "");
});

const needs = computed(() => debitOf(props.legs));
const short = computed(() => Math.max(0, needs.value - props.free));

const form = ref<"none" | "approve" | "reject">("none");
const fills = ref<Record<string, { qty: string; price: string }>>({});
const reason = ref("");
const failure = ref("");
const busy = ref(false);
/** The exchange's price now, per symbol: a decision written before the open carries yesterday's. */
const live = ref<Record<string, number>>({});

onMounted(async () => {
    if (!pending.value || isFund.value) return;
    for (const l of props.legs) {
        const price = await livePrice(l.symbol);
        if (price !== null) live.value[l.symbol] = price;
    }
});

function open(kind: "approve" | "reject"): void {
    form.value = form.value === kind ? "none" : kind;
    failure.value = "";
    if (kind === "approve") {
        fills.value = Object.fromEntries(
            props.legs.map((l) => [
                l.id,
                {
                    qty: String(isFund.value ? (l.amount ?? "") : (l.qty ?? "")),
                    price: String(live.value[l.symbol] ?? l.entry ?? ""),
                },
            ]),
        );
    }
}

/** The sign-off's own arithmetic: what the sheet would take out of the purse as typed. */
const typed = computed(() =>
    debitOf(
        props.legs.map((l) => {
            const f = fills.value[l.id];
            if (!f) return l;
            const price = Number(f.price);
            const amount = Number(f.qty);
            const qty =
                isFund.value && amount > 0 && price > 0 ? floorUnits(amount / price) : amount;
            return {
                ...l,
                qty,
                entry: price,
                amount: isFund.value ? amount : null,
            };
        }),
    ),
);

function legQty(l: SignalRow): string {
    if (isFund.value) return money(legValue(l));
    if (l.qty === null || l.qty <= 0) return "—";
    return l.entry === null ? units(l.qty) : `${units(l.qty)} @ ${money(l.entry)}`;
}

const rr = (l: SignalRow): string | null => {
    const r = rewardRisk(l);
    return r === null ? null : `${r.toFixed(1)}R`;
};

async function post(): Promise<void> {
    busy.value = true;
    failure.value = "";
    try {
        const edited = props.legs.map((l) => {
            const f = fills.value[l.id] ?? { qty: "", price: "" };
            const price = Number(f.price);
            const amount = Number(f.qty);
            const qty =
                isFund.value && amount > 0 && price > 0 ? floorUnits(amount / price) : amount;
            return { id: l.id, qty, price };
        });
        if (props.plan) {
            await api.approveSignal(first.value.id, { fills: edited });
        } else {
            const [one] = edited;
            await api.approveSignal(first.value.id, {
                qty: one.qty,
                price: one.price,
            });
        }
        form.value = "none";
        emit("reload");
    } catch (e) {
        failure.value = String(e);
    } finally {
        busy.value = false;
    }
}

async function reject(): Promise<void> {
    busy.value = true;
    failure.value = "";
    try {
        await api.rejectSignal(first.value.id, reason.value.trim() || undefined);
        form.value = "none";
        emit("reload");
    } catch (e) {
        failure.value = String(e);
    } finally {
        busy.value = false;
    }
}

const posted = computed(() => {
    const l = first.value;
    if (l.filled_qty === null || l.filled_price === null) return "Posted";
    return `Posted ${units(l.filled_qty)} @ ${money(l.filled_price)}`;
});
</script>

<template>
    <article class="decision" :class="first.status">
        <div class="dhead">
            <span
                class="pill"
                :class="{
                    'p-a': pending,
                    'p-b': approved,
                    'p-n': !pending && !approved,
                }"
                >{{ first.status.toUpperCase() }}</span
            >
            <b>{{ title }}</b>
            <span class="tag" :class="{ short: horizon === 'SHORT' }">{{ horizon }}</span>
            <span v-if="plan && planCode" class="planid">plan {{ planCode }}</span>
            <span v-if="plan && planCode" class="dotsep" aria-hidden="true">·</span>
            <span class="when">{{ dayTime(first.created_at) }}</span>
            <RouterLink
                v-if="first.conversation_id"
                class="call"
                :to="`/calls?conversation=${encodeURIComponent(first.conversation_id)}`"
                >see the call</RouterLink
            >
            <span v-else class="call rules">rules only</span>
            <span
                v-if="pending && first.confidence"
                class="conf"
                v-html="renderConfidence(first.confidence)"
            />
            <span class="aside">
                <template v-if="!pending">{{ approved ? "posted" : "" }}</template>
                <template v-else-if="free <= 0 && needs > 0"
                    >needs {{ money(needs) }} · no free cash</template
                >
                <template v-else-if="short > 0"
                    >needs {{ money(needs) }} · short {{ money(short) }}</template
                >
                <template v-else>needs {{ money(needs) }} · free {{ money(free) }}</template>
            </span>
        </div>

        <div class="legs">
            <div v-for="l in legs" :key="l.id" class="leg">
                <span class="pill" :class="l.action === 'SELL' ? 'p-s' : 'p-b'">{{
                    l.action
                }}</span>
                <div class="legbody">
                    <div class="legtop">
                        <RouterLink class="sym-link" :to="`/i/${encodeURIComponent(l.symbol)}`"
                            ><b>{{ l.name || l.symbol }}</b></RouterLink
                        >
                        <span v-if="l.symbol !== l.name" class="sym">{{ l.symbol }}</span>
                        <span class="qty num">{{ legQty(l) }}</span>
                    </div>
                    <div class="legmeta">
                        <template v-if="isFund">
                            <span>at NAV</span><span>no stop or target</span>
                        </template>
                        <template v-else>
                            <span
                                >stop <b>{{ l.stop === null ? "—" : money(l.stop) }}</b></span
                            >
                            <span
                                >target <b>{{ l.target === null ? "—" : money(l.target) }}</b></span
                            >
                            <span v-if="live[l.symbol] !== undefined" class="now"
                                >now <b>{{ money(live[l.symbol]) }}</b></span
                            >
                            <span v-if="rr(l)" class="up">{{ rr(l) }}</span>
                        </template>
                    </div>
                </div>
            </div>
        </div>

        <p v-if="prose || !pending" class="why">
            <b>{{ pending ? "Why" : approved ? "Result" : "Why rejected" }}</b><template
                v-if="pending"
                >{{ prose }}</template
            ><template v-else-if="approved"
                >{{ posted }} — position, trade row and cash moved together.</template
            ><template v-else>{{ first.status_reason ?? "No reason was given." }}</template>
        </p>

        <div v-if="pending" class="acts">
            <button
                class="d-btn d-btn-xs d-btn-primary"
                type="button"
                :aria-expanded="form === 'approve'"
                @click="open('approve')"
            >
                Approve
            </button>
            <button
                class="d-btn d-btn-xs"
                type="button"
                :aria-expanded="form === 'reject'"
                @click="open('reject')"
            >
                {{ form === "reject" ? "Cancel" : "Reject" }}
            </button>
            <span class="hint">{{
                plan ? "Every leg posts together, or none does" : "One move, one transaction"
            }}</span>
        </div>

        <form v-if="form === 'approve'" class="signoff" @submit.prevent="post">
            <div class="sfh">Sign off — edit qty or price</div>
            <div class="fields">
                <template v-for="l in legs" :key="l.id">
                    <label class="sf">
                        <span>{{ isFund ? "Amount" : `${l.symbol} qty` }}</span>
                        <input
                            v-model="fills[l.id]!.qty"
                            class="d-input input-sm num"
                            type="number"
                            min="0"
                            step="any"
                            :aria-label="isFund ? 'Amount in rupees' : `${l.symbol} quantity`"
                        />
                    </label>
                    <label class="sf">
                        <span>{{ isFund ? "NAV" : "Price" }}</span>
                        <input
                            v-model="fills[l.id]!.price"
                            class="d-input input-sm num"
                            type="number"
                            min="0"
                            step="any"
                            :aria-label="`${l.symbol} price`"
                        />
                    </label>
                </template>
            </div>
            <div class="amt">
                <span>Debit total</span>
                <b class="num">{{ money(typed) }}</b>
            </div>
            <button class="d-btn d-btn-xs d-btn-primary" type="submit" :disabled="busy">
                {{ busy ? "Submitting…" : "Submit" }}
            </button>
            <p v-if="failure" class="hint failure" role="alert">
                {{ failure }}
            </p>
        </form>

        <form v-if="form === 'reject'" class="rejectbar" @submit.prevent="reject">
            <label class="sf grow">
                <span>Reason (optional)</span>
                <input
                    v-model="reason"
                    class="d-input input-sm"
                    type="text"
                    placeholder="Why this one is turned down"
                />
            </label>
            <button class="d-btn d-btn-xs" type="submit" :disabled="busy">
                {{ busy ? "Rejecting…" : "Reject" }}
            </button>
            <p v-if="failure" class="hint failure" role="alert">
                {{ failure }}
            </p>
        </form>
    </article>
</template>

<style scoped>
.decision {
    border: 1px solid var(--rule-subtle);
    border-left: 3px solid var(--brand);
    border-radius: var(--radius-card);
    background: var(--paper-raised);
    padding: 13px 16px;
    margin-bottom: 12px;
    box-shadow: var(--shadow-card);
}
.decision.approved {
    border-left-color: var(--green);
}
.decision.rejected {
    border-left-color: var(--ink-tertiary);
    opacity: 0.82;
}
.dhead {
    display: flex;
    align-items: center;
    gap: 9px;
    flex-wrap: wrap;
    font-size: 11.5px;
    color: var(--ink-tertiary);
}
.dhead > b {
    font-size: 13px;
    color: var(--ink);
    font-weight: 650;
}
.planid {
    font-family: var(--font-ledger);
    font-size: 9.5px;
    color: var(--ink-tertiary);
    border: 1px solid var(--rule-subtle);
    border-radius: 4px;
    padding: 1px 4px;
}
.dotsep {
    color: var(--rule-strong);
}
.dhead .call {
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--brand);
    text-decoration: none;
    border-bottom: 1px solid transparent;
}
.dhead .call:hover {
    border-bottom-color: var(--brand);
}
.dhead .call.rules {
    color: var(--ink-tertiary);
}
.aside {
    margin-left: auto;
    font-family: var(--font-ledger);
    font-size: 10.5px;
    color: var(--ink-tertiary);
}
.legs {
    margin-top: 10px;
    display: grid;
    gap: 5px;
}
.leg {
    display: grid;
    grid-template-columns: 52px minmax(0, 1fr);
    gap: 12px;
    align-items: start;
    padding: 5px 0;
    border-bottom: 1px dotted var(--rule-subtle);
}
.leg:last-child {
    border-bottom: 0;
}
.legbody {
    min-width: 0;
}
.legtop {
    display: flex;
    align-items: baseline;
    gap: 8px;
}
.sym-link {
    color: inherit;
    text-decoration: none;
}
.sym-link:hover b {
    color: var(--brand);
}
.legtop b {
    font-size: 12.5px;
    min-width: 0;
    overflow-wrap: anywhere;
}
.legtop b {
    font-size: 12.5px;
}
.legtop .qty {
    margin-left: auto;
    font-size: 12px;
    color: var(--ink);
    white-space: nowrap;
}
.sym {
    font-family: var(--font-ledger);
    font-size: 10.5px;
    color: var(--ink-tertiary);
}
.legmeta {
    display: flex;
    gap: 18px;
    font-size: 10.5px;
    color: var(--ink-tertiary);
    margin-top: 2px;
    flex-wrap: wrap;
}
.legmeta b {
    font-family: var(--font-ledger);
    font-weight: 500;
    color: var(--ink-secondary);
}
.why {
    font-size: 11.5px;
    color: var(--ink-secondary);
    border-top: 1px dashed var(--rule-subtle);
    padding-top: 9px;
    margin: 8px 0 0;
    max-width: none;
}
.why b {
    color: var(--ink);
    font-weight: 650;
    margin-right: 5px;
}
.acts {
    display: flex;
    gap: 7px;
    align-items: center;
    margin-top: 10px;
    flex-wrap: wrap;
}
.acts .hint {
    margin-left: auto;
}
.hint {
    font-size: 11px;
    color: var(--ink-tertiary);
}
.signoff {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: 10px;
    align-items: end;
    background: var(--paper-inset);
    border: 1px dashed var(--rule-strong);
    border-radius: var(--radius-sm);
    padding: 12px;
    margin-top: 11px;
}
.fields {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 10px;
    align-items: end;
    min-width: 0;
}
.rejectbar {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: end;
    background: var(--paper-inset);
    border: 1px dashed var(--rule-strong);
    border-radius: var(--radius-sm);
    padding: 12px;
    margin-top: 11px;
}
.signoff .sfh {
    grid-column: 1 / -1;
    font-size: 9.5px;
    letter-spacing: 0.11em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
.sf {
    display: grid;
    gap: 4px;
    min-width: 0;
}
.sf span {
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.sf.grow {
    flex: 1 1 320px;
    max-width: none;
}
.signoff .failure {
    grid-column: 1 / -1;
}
.amt {
    text-align: right;
    margin-left: auto;
}
.amt span {
    display: block;
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ink-tertiary);
    font-weight: 700;
}
.amt b {
    font-size: 15px;
    font-weight: 500;
}
.failure {
    flex-basis: 100%;
    margin: 0;
    color: var(--stamp-oxide);
}
@media (max-width: 640px) {
    .decision {
        padding: 12px 14px;
    }
    .dhead {
        gap: 6px 7px;
    }
    .dhead > .pill {
        order: 1;
    }
    .dhead > b {
        order: 2;
        font-size: 12.5px;
    }
    .dhead .tag {
        order: 3;
    }
    .dhead .when {
        order: 4;
        font-size: 10.5px;
    }
    .dhead .planid,
    .dhead .dotsep {
        display: none;
    }
    .dhead .conf {
        order: 5;
        margin-left: auto;
    }
    .aside {
        order: 6;
        flex-basis: 100%;
        margin-left: 0;
        font-size: 10.5px;
    }
    .leg {
        grid-template-columns: 46px minmax(0, 1fr);
        gap: 8px;
    }
    .legtop b {
        font-size: 12px;
    }
    .legtop .qty {
        font-size: 11.5px;
    }
    .legmeta {
        gap: 0;
        font-size: 10.5px;
        margin-top: 3px;
    }
    .legmeta span + span::before {
        content: "·";
        margin: 0 5px;
        color: var(--rule-strong);
    }
    .why {
        font-size: 11px;
        color: var(--ink-tertiary);
        padding-top: 8px;
        margin-top: 9px;
    }
    .why b {
        display: none;
    }
    .acts {
        justify-content: flex-end;
        gap: 6px;
        margin-top: 10px;
    }
    .acts .hint {
        display: none;
    }
    .acts button:first-of-type {
        order: 2;
    }
    .acts button:nth-of-type(2) {
        order: 1;
    }
    .signoff {
        grid-template-columns: minmax(0, 1fr);
    }
    .signoff .amt {
        text-align: left;
        margin-left: 0;
    }
    .signoff .d-btn {
        justify-self: start;
    }
}
</style>
