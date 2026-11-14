# web

The front end: Vue 3 + vue-router, hand-written CSS tokens, daisyUI (`d-` prefixed) for controls.

- **`UI_STANDARDS.md`** — tokens, type, alignment, components, states, copy, and the review checklist.
  Binding for every change here.
- `docs/testing/flows.md` — what every page and control must do, walked before a release.
- `src/views/` — one file per route; `src/components/` — the kit; `src/api.ts` + `src/format.ts` —
  the contract and its formatting.

```sh
npm install
npm run dev      # vite, proxying /api to 127.0.0.1:8080
npm test         # vitest, pure logic
npm run build    # what the server serves
```
