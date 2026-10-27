import { defineConfig } from "vite";

export default defineConfig({
  server: {
    proxy: { "/api": "http://127.0.0.1:8080" },
  },
  build: { outDir: "dist", emptyOutDir: true },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
