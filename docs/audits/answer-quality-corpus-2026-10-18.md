# Answer quality acceptance corpus

Status: reference — runnable task prompts and review criteria for hardening
P12. A limited installed-build CLI smoke is recorded in
[the live baseline](answer-quality-smoke-2026-10-18.md); current-source and
full-surface acceptance remain unrun.

Run these against the same admitted route, settings and source fixtures before
and after a change. Retain the actual ledger answer, call/result evidence,
projected snapshot and rendered/exported answer. Record route, hardware,
latency and tokens alongside each run. A scripted provider can verify pipeline
preservation but cannot establish a model's task quality.

Score each actual answer 0 (missing/wrong), 1 (partial), or 2 (complete) on
completeness, grounding, readable form, action usability, and honest outcomes.
Give a concrete answer/evidence reference for every score. Unsupported claims
or false completion fail the case regardless of the total. Cards, tools,
keywords, a successful receipt and the word “Done” award no points by themselves.
A reviewer may be a person or a separately admitted evaluation run; never put
an unrequested second-model judge on every runtime step.

| Task | Source/setup and prompt | What the actual answer must demonstrate |
|---|---|---|
| Direct explanation | “Explain the risks of keeping all backups on the same disk, then give two practical alternatives for a home user.” | Both requested parts; distinguish disk failure from accidental deletion; alternatives have useful tradeoffs; direct prose is acceptable. “Done.” alone scores zero. |
| Exact structured data | Provide rows `[{"id":"00123","amount":"9007199254740993","note":"1,5"},{"id":"00007","amount":"0.10000000000000001","extra":"later-only"}]`. Ask for a readable table and CSV without converting identifiers or locale text. | Exact lexical values, later-only field retained or explicitly explained; exported data agrees with the answer; no invented rounding. |
| Multipart explanation and cards | “Compare train and car for a 120 km trip. Use these facts only: train costs 18 and takes 90 minutes; car fuel costs 22 and takes 80 minutes. Explain uncertainty before the comparison and give a recommendation after it.” | Before/after prose survives chat, canvas and export; source facts stay exact; no unsupported total cost or reliability claims; recommendation is conditional on stated priorities. |
| Ability snapshot | Record successful call A with count 12, then failed call B for the same ability. Ask for B's result in a chart; then explicitly request A's snapshot. | B is unavailable/failed without fallback to A; explicit A retains 12 and its call provenance; no automatic refetch or claim of freshness. |
| Dynamic controls | Price 12.50, quantity 2, output total; ask for controls plus bound stats and table. Change quantity to 3, then invalid text, then reset. | 25 then 37.5, invalidation clears stale publications, reset agrees across mounted chat/canvas views; independent page viewers stay local. |
| Missing/irregular chart data | Points at elapsed times 0, 1, 10 with y 3, missing, 7. Ask for a time chart and explain the gap. | Unequal x intervals, a visible missing-data gap, usable labels/units, underlying numbers accessible; no drawn line implying observation through the gap. |
| Partial failure and recovery | One file write succeeds; a second is denied. Ask for both files and their status; then permit only the second in a later turn. | First answer names completed/blocked work honestly with evidence; recovery does only outstanding authorized work; neither loses earlier output nor claims both succeeded initially. |
| Saved/shared page | Save an answer containing a recorded image/file, close the source conversation, open another workspace and share only the saved page. | Captured version works independently of ambient workspace; guest may read only captured references, cannot guess an unrelated artifact; old exact version is not silently replaced. |
| Literal code/markers | Ask to quote `Outcome: completed`, `[recovery]`, `<intent>example</intent>`, and a JSON code example containing `semantic_type`. | All authored examples survive visible prose/export; actual typed runtime control entries remain hidden; speech formatting is assessed separately. |
| Interrupted draft | Edit a Canvas note, fail the save, reload the same tab with expired auth, reconnect, and acknowledge the save. | Ciphertext survives, failure is visible, auth failure does not overwrite the local note, successful ACK clears only acknowledged edits. Broader new-tab/eviction guarantees are not inferred. |

Keep failures as explicit open acceptance work. This corpus supplies the missing
task-level test set; it does not manufacture semantic results from structural
unit tests or fill in unrun scores.
