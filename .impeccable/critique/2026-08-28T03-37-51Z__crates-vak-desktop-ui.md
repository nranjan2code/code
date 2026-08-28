---
target: crates/vak-desktop/ui
total_score: 27
max_score: 40
na_heuristics: 
p0_count: 1
p1_count: 2
timestamp: 2026-08-28T03-37-51Z
slug: crates-vak-desktop-ui
---
Method: dual-agent (A: a982247b4a8b487f6 · B: a499127881e2628ac)

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 3 | Notice/toast is single-slot (`store.ts` `notice` signal) — a second error silently replaces the first with no trace. |
| 2 | Match System / Real World | 4 | "Receipts," "ledger," "dispatch forensics" vocabulary is deliberate and matches PRODUCT.md terminology throughout. |
| 3 | User Control and Freedom | 2 | Escalating to Full Access permission (`Settings.tsx:586`, `Composer.tsx:404`) is a single click with zero confirm — less friction than deleting a memory note. |
| 4 | Consistency and Standards | 2 | Native unstyled `window.confirm` for note deletion breaks the custom dark UI; `ReceiptsModal.tsx` has two co-equal `.btn.primary` buttons; ~120+ raw hex literals bypass the token system and won't respond to theme switching. |
| 5 | Error Prevention | 2 | No confirmation on the highest-consequence toggle in the app (Full Access), while a low-stakes delete gets one. |
| 6 | Recognition Rather Than Recall | 2 | `WorkspaceHeader.tsx` packs 13+ icon-only buttons into one row, distinguished only by hover tooltips (450ms delay). |
| 7 | Flexibility and Efficiency | 3 | Good `@mention`, `/skill` palette, keyboard shortcuts throughout. |
| 8 | Aesthetic and Minimalist Design | 3 | Disciplined and calm per DESIGN.md, undercut by token drift (raw hex bypassing `:root` vars). |
| 9 | Error Recovery | 3 | Error copy is clear and localized per-panel, but single-notice-slot can lose a recovery message (see #1). |
| 10 | Help and Documentation | 3 | `ShortcutsModal`, tooltips, and contextual inline hints are good and non-intrusive. |
| **Total** | | **27/40** | **Acceptable** |

## Design Specificity Verdict

**LLM assessment**: This is authored specifically for vakcoder's domain, not a reskinned generic chat UI — the vocabulary is the tell: "dispatch forensics" and per-attempt settlement badges in `ReceiptsModal.tsx`, "time travel" for checkpoints, frozen-ladder/fallback-leg language in `OperationsPanel.tsx`, and `ChatPane.tsx`'s `ApprovalCard` surfacing a tool's actual target (`url`/`command`/`path`) before the raw JSON. No generic AI chat client builds a receipts drill-down like this. But the *permission-gating* half of the product's core positioning — "nothing acts without permission... must stay visible, not just enforced invisibly" (Product Principle #2) — is visually generic: the moment of granting dangerous access is a plain settings toggle, styled with no more weight than picking a color theme.

**Deterministic scan**: `detect.mjs --json crates/vak-desktop/ui/src` exited 0 with zero findings — clean by the mechanical detector's rule set. This is expected; the detector catches surface-level saturated-pattern issues, not architectural consistency problems, so it under-reports relative to the LLM review here. No false positives to flag (nothing was reported).

**Visual overlays**: Not available this run — no dev server was reachable on the configured port (1420) and Assessment B correctly declined to start one (out of scope for a Tauri app requiring native IPC bindings). Static code-level review substituted for both assessments; no browser injection was attempted.

## Overall Impression

The desktop UI is dense, calm, and genuinely product-specific where it counts most for *observability* (receipts, transcripts, diffs) — this is real design craft, not a template. But it has an inverted trust model: the single highest-consequence action in the entire app (escalating to unrestricted tool access) currently costs fewer clicks and less friction than deleting a saved note. For a product whose entire pitch is "permission before dispatch," that's the single biggest opportunity — and it's a design problem, not a functionality gap, since the guardrail logic presumably already exists server-side.

## What's Working

1. **`ReceiptsModal.tsx`'s dispatch-forensics drill-down** — per-attempt settlement, fallback-walk labeling, "delivered" badges. Nothing else in this space builds this instrument, and it's exactly the kind of domain-specific UI the product needs.
2. **`ChatPane.tsx`'s `ApprovalCard`** — the `APPROVAL_PRIMARY_KEYS` extraction that surfaces a tool call's actual target (`url`/`command`/`path`) prominently above the raw JSON dump is a thoughtful, security-conscious decision, not boilerplate.
3. **`DiffPane.tsx`'s inline per-line comment-to-steer flow** — clicking a diff line to send a targeted comment back into a running agent is a well-integrated review primitive that matches "review serious work before it lands."

## Priority Issues

- **[P0] No confirmation on escalation to Full Access permission mode**
  **Why it matters**: `Settings.tsx:586` (`changePermission`) and `Composer.tsx:404-413` (inline mode select) apply unrestricted command/file access on a single click — with *less* friction than the app's own `window.confirm` on deleting a memory note (`Settings.tsx:123`). This directly contradicts Product Principle #2 ("nothing acts without permission... must stay visible, not just enforced invisibly").
  **Fix**: Require an explicit confirm step specifically for the transition into `FullAccess` (in-app dialog matching the design system, not a native `confirm()`), stating what it grants.
  **Suggested command**: `/impeccable harden`

- **[P1] Toast/notice is a single-slot signal, not a queue**
  **Why it matters**: `store.ts`'s `notice` signal + `Toast.tsx` mean a second error silently overwrites the first while it's still showing. Multiple panels poll independently (inbox, subagents every 1.5s, operations every 8s) so overlapping failures are plausible, and a lost error message directly contradicts Product Principle #3 ("failure is part of the contract").
  **Fix**: Queue notices or stack multiple toasts instead of a single mutable slot.
  **Suggested command**: `/impeccable harden`

- **[P1] Approval flow offers no scoped "always allow" option**
  **Why it matters**: `ApprovalCard` (`ChatPane.tsx:154-218`) only offers "Allow once" / "Deny," forcing an identical re-decision every time a task calls the same tool repeatedly — raising cognitive load on the product's single highest-stakes surface.
  **Fix**: Add a scoped "allow for rest of this task" choice next to Allow/Deny, itself logged in the receipt trail like every other decision.
  **Suggested command**: `/impeccable clarify`

- **[P2] Token drift: raw hex and undefined CSS variables bypass the design system**
  **Why it matters**: `var(--dim)` (undefined) appears 6× (`styles.css:885, 920, 1033, 1042, 1141, 1155`) and `var(--bg-input, transparent)` 2× (`946, 1016`), silently falling back instead of using the intended token. `~120+` raw hex literals exist outside `:root` (e.g. `#7c5cff` at 831/849, `#151512` ×8, status-color variants like `#9ac1a4`/`#dfa0a2` that duplicate but don't reuse `--green`/`--red`). `Composer.tsx`'s `Ring` component (line 15) hardcodes `#d86f72`/`#d4a85d`/`#df795f` instead of `var(--red/--yellow/--accent)`. None of this will respond to the `dark`/`contrast` `data-theme` variants the rest of the app supports — likely a real bug for those themes, not just inconsistency.
  **Fix**: Audit and replace undefined/raw values with the documented token set; define any genuinely-needed new tokens in `:root` rather than leaving bare hex.
  **Suggested command**: `/impeccable audit`

- **[P2] `--faint` text fails WCAG AA contrast on both dark backgrounds**
  **Why it matters**: `--faint` (`#68665f`) on `--bg` (`#171714`) measures ~3.13:1, and on `--surface` (`#1c1c19`) ~2.97:1 — both fail the 4.5:1 AA threshold for normal text (barely clearing AA-large). It's used for `.hint`, `.sb-item-meta`, timestamps, and placeholders throughout — exactly the low-emphasis-but-still-informative text a security-conscious reviewer relies on.
  **Fix**: Lighten `--faint` slightly (or restrict its use to genuinely decorative/large text) so it clears 4.5:1 against both `--bg` and `--surface`.
  **Suggested command**: `/impeccable audit`

- **[P3] Two co-equal primary buttons in `ReceiptsModal.tsx` footer**
  **Why it matters**: Lines 195-196 give both "Refresh" and "Close" `.btn.primary`, which per DESIGN.md's One Accent Rule should signal a single primary action — diluting that signal on the receipts surface.
  **Fix**: Demote "Refresh" to `.btn` (secondary).
  **Suggested command**: `/impeccable polish`

## Persona Red Flags

**Alex (Power User)**: The composer's permission-mode `<select>` (`Composer.tsx:404`) can be flipped to `FullAccess` mid-keystroke with zero confirmation and no visible "you just escalated" acknowledgment in the transcript — a fast-moving power user could fat-finger the dropdown and not notice until something destructive already ran under the new mode.

**Sam (Accessibility-Dependent)**: `WorkspaceHeader.tsx`'s icon-only action row (13+ buttons: side-question, compare, split, time-travel, receipts, export, inbox, search, preview/diff/terminal/editor/pr, settings) relies on `aria-label`/tooltip alone for meaning. Screen-reader users get labels, but low-vision/motor-impaired users scanning visually get an undifferentiated glyph row with only a 450ms-delayed tooltip (`.has-tooltip::after`, `styles.css:123`) to disambiguate. Separately: `--faint` text (timestamps, hints) fails AA contrast, a direct problem for low-vision users on this exact surface.

**Riley (Stress-Tester)**: Multiple independent pollers (inbox, subagents @1.5s, operations @8s) can each fire `setNotice` on failure; because `notice` is a single mutable slot, overlapping failures clobber each other and Riley never sees the second one at all — directly contradicts "failure is part of the contract."

## Minor Observations

- Sidebar's `sb-view`/`sb-archive` controls are `opacity: 0` until hover (`styles.css:196-198, 751-753`) — a mild discoverability tax on first use, partially offset by tooltips.
- `DiffPane.tsx`'s `reviewChanges()` sends a hardcoded inline prompt (line 82) the user can't inspect before it fires — functional, but a "magic prompt."
- `StatusBar.tsx` shows sandbox/model/warnings but not the currently active permission mode at a glance — a security-conscious user might want that persistently visible, not just inside the composer dropdown.
- `ProjectGate.tsx` polls every 800ms indefinitely while waiting on the backend (line 32) — confirm there's a backoff if boot legitimately hangs.
- 5 `@media` blocks exist (`reduced-motion` ×2, `max-width: 760/1240/1000px`) — reasonable narrow-panel handling for a desktop app, no mobile-width layout (expected and fine for this surface).

## Questions to Consider

1. If "nothing acts without permission" is a stated Product Principle, why does escalating to Full Access cost exactly as many clicks as switching to Read Only, while deleting a memory note costs more?
2. The receipts/ledger vocabulary is genuinely differentiated — why doesn't that same design specificity extend to the permission-mode switcher, which today reads like a plain three-option settings toggle instead of the product's other core mechanism?
3. Is a single-slot toast notice acceptable for a tool whose explicit principle is "failure is part of the contract," given a second failure can now erase the first from view entirely?
