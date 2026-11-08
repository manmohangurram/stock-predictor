# Settings page — three directions

Throwaway mockups from the design conversation, kept so the choice can be seen again.
Open the `.html` files directly (they are self-contained; the fonts fall back to system
faces when offline) or view the matching `.png`.

| File | Direction | Shape |
|---|---|---|
| `a.html` | Index rail | Six tab-divider sections on the left, one section at a time on the right |
| `b.html` | Two-page spread | Everything on one screen as an open book, collapsible, sticky receipt strip |
| `c.html` | Numbered schedule | One numbered form (01…30), units in the margin, sticky summary panel |

**Chosen: A, with strict tab separation.** `a-money.html`, `a-risk.html` and `a-cadence.html` are
the refined version — one section per tab, its own stamp, its own saved/unsaved state;
`a-cadence-m.png` is the same tab at 390px, where the rail becomes a scrollable strip.

All three are built from `.interface-design/system.md` tokens: paper/ink, 0.5–1px rules,
3px surfaces, IBM Plex Sans + Mono, stamps instead of primary buttons.

None of this is the product. The chosen direction replaces `web/src/pages/settings.ts`.
