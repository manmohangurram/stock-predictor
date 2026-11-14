import { reactive } from "vue";

/** Counts the rail shows beside a section. Set by whichever view knows them. */
export const railBadges = reactive<{ ledger: string }>({ ledger: "" });
