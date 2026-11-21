<script setup lang="ts">
/**
 * The chrome every page sits in: a top bar with the destinations, and the page below it. On a
 * phone the destinations become a bottom bar. The market state lives in the bar at a desk and
 * under the page title on a phone.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute } from "vue-router";
import {
    Archive,
    BookOpen,
    ChevronDown,
    ListChecks,
    LogOut,
    Moon,
    ScrollText,
    Settings,
    Sun,
    User,
} from "lucide-vue-next";
import { api, auth, type Market } from "../api";
import { useAdmin } from "../admin";

const route = useRoute();
const { username: adminName } = useAdmin();
const links = [
    { id: "ledger", label: "Book", href: "/", icon: BookOpen },
    {
        id: "decisions",
        label: "Decisions",
        href: "/decisions",
        icon: ListChecks,
    },
    { id: "archived", label: "Archived", href: "/archived", icon: Archive },
    { id: "audit", label: "Audit", href: "/calls", icon: ScrollText },
    { id: "settings", label: "Settings", href: "/settings", icon: Settings },
] as const;

const active = computed(() => {
    const p = route.path;
    if (p.startsWith("/calls")) return "audit";
    if (p.startsWith("/settings")) return "settings";
    if (p.startsWith("/archived")) return "archived";
    if (p.startsWith("/decisions")) return "decisions";
    return "ledger";
});

const market = ref<Market | null>(null);
let timer: number | undefined;

async function loadMarket(): Promise<void> {
    try {
        market.value = await api.market();
    } catch {
        market.value = null;
    }
}

const marketLabel = computed(() => {
    const m = market.value;
    if (!m) return "";
    if (m.state === "open") return "Market open";
    return m.state === "holiday" ? "Market shut" : "Market closed";
});

const marketWhen = computed(() => {
    const m = market.value;
    if (!m) return "";
    const at = m.state === "open" ? m.closes_at : m.next_open_at;
    if (!at) return "";
    const when = new Date(at).toLocaleString("en-IN", {
        day: "2-digit",
        month: "short",
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
    });
    return `${m.state === "open" ? "Closes" : "Opens"} ${when}`;
});

const media = window.matchMedia("(prefers-color-scheme: dark)");
const stored = localStorage.getItem("sp-theme");
const dark = ref(stored ? stored === "dark" : media.matches);
const menuOpen = ref(false);

function applyTheme(): void {
    document.documentElement.dataset.theme = dark.value ? "dark" : "light";
    localStorage.setItem("sp-theme", dark.value ? "dark" : "light");
}

function toggleTheme(): void {
    dark.value = !dark.value;
    applyTheme();
}

function onSystemTheme(): void {
    if (!localStorage.getItem("sp-theme")) dark.value = media.matches;
}

function onDocumentClick(event: MouseEvent): void {
    if (!(event.target as Element | null)?.closest(".acct"))
        menuOpen.value = false;
}

function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") menuOpen.value = false;
}

async function signOut(): Promise<void> {
    try {
        await api.authLogout();
    } finally {
        auth.set(null);
        window.location.href = "/login";
    }
}

onMounted(() => {
    if (stored) applyTheme();
    media.addEventListener("change", onSystemTheme);
    document.addEventListener("click", onDocumentClick);
    document.addEventListener("keydown", onKeydown);
    void loadMarket();
    timer = window.setInterval(() => void loadMarket(), 60_000);
});

onBeforeUnmount(() => {
    media.removeEventListener("change", onSystemTheme);
    document.removeEventListener("click", onDocumentClick);
    document.removeEventListener("keydown", onKeydown);
    window.clearInterval(timer);
});
</script>

<template>
    <div class="app">
        <header class="topbar">
            <a class="brand" href="/" aria-label="Stock Predictor, Book">
                <img
                    src="/favicon.svg"
                    width="30"
                    height="30"
                    alt=""
                    aria-hidden="true"
                />
                <span>Stock Predictor</span>
            </a>
            <nav class="topnav" aria-label="Sections">
                <a
                    v-for="l in links"
                    :key="l.id"
                    :href="l.href"
                    :aria-label="l.label"
                    :class="{ on: active === l.id }"
                    :aria-current="active === l.id ? 'page' : undefined"
                >
                    <component
                        :is="l.icon"
                        :size="15"
                        :stroke-width="1.75"
                        aria-hidden="true"
                    />
                    <span>{{ l.label }}</span>
                </a>
            </nav>
            <div class="topbar-right">
                <div
                    v-if="market"
                    class="market-line"
                    role="status"
                    aria-live="polite"
                    :data-state="market.state"
                    :title="`${marketLabel} · ${market.reason} · ${market.session}`"
                >
                    <span class="dot" aria-hidden="true"></span>
                    <span>{{ marketLabel }} · {{ marketWhen }}</span>
                </div>
                <button
                    class="icon-btn"
                    type="button"
                    :aria-label="
                        dark
                            ? 'Switch to the light theme'
                            : 'Switch to the dark theme'
                    "
                    @click="toggleTheme"
                >
                    <Sun
                        v-if="dark"
                        :size="16"
                        :stroke-width="1.75"
                        aria-hidden="true"
                    />
                    <Moon
                        v-else
                        :size="16"
                        :stroke-width="1.75"
                        aria-hidden="true"
                    />
                </button>
                <div class="acct">
                    <button
                        class="acct-btn"
                        type="button"
                        aria-haspopup="menu"
                        :aria-expanded="menuOpen"
                        @click.stop="menuOpen = !menuOpen"
                    >
                        <span class="acct-avatar"
                            ><User
                                :size="14"
                                :stroke-width="1.75"
                                aria-hidden="true"
                        /></span>
                        <ChevronDown
                            :size="14"
                            :stroke-width="1.75"
                            aria-hidden="true"
                        />
                    </button>
                    <div
                        v-if="menuOpen"
                        class="acct-menu"
                        role="menu"
                        aria-label="Account"
                    >
                        <div class="who">
                            <b>{{ adminName }}</b
                            ><span>Signed in · local session</span>
                        </div>
                        <button type="button" role="menuitem" @click="signOut">
                            <LogOut
                                :size="14"
                                :stroke-width="1.75"
                                aria-hidden="true"
                            />
                            Sign out
                        </button>
                    </div>
                </div>
            </div>
        </header>

        <main id="main" class="main">
            <slot />
        </main>

        <nav class="tabbar" aria-label="Sections">
            <a
                v-for="l in links"
                :key="l.id"
                :href="l.href"
                :aria-label="l.label"
                :class="{ on: active === l.id }"
                :aria-current="active === l.id ? 'page' : undefined"
            >
                <span class="tab-icon">
                    <component
                        :is="l.icon"
                        :size="19"
                        :stroke-width="1.75"
                        aria-hidden="true"
                    />
                </span>
                <span>{{ l.label }}</span>
            </a>
        </nav>
    </div>
</template>
