<script setup lang="ts">
import { RouterView, useRoute } from "vue-router";
import AppShell from "./components/AppShell.vue";

const route = useRoute();
</script>

<template>
    <RouterView v-if="route.path === '/login'" />
    <AppShell v-else>
        <RouterView />
    </AppShell>
</template>
