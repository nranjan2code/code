# Answer design contract

Status: in progress. The coordinated instruction/typed-receipt correction is
built and passes focused and fixed-corpus live acceptance (2026-10-22).
Broader semantic quality and interactive-artifact revision remain open.
No replacement renderer or domain router.

## Decision and boundaries

The model chooses how to communicate the requested work from its meaning,
the information it actually has, and what the person needs to do with it.
The runtime checks the resulting structural promises. Neither tool availability
nor a successful card receipt establishes that the answer is useful or complete.

Preserve the recent Continuous answer implementation, compact file collections,
source order, exact placement, Canvas and artifact identities. An enclosing box
does not make prose into a useful visualization. Do not change their CSS or
restore the superseded presentation. Do not select forms from topic words,
provider names, canned domain recipes, card quotas or answer-length thresholds.

## One contract, several responsibilities

1. **Presentation guidance has one owner.** The code-owned presentation
   contract tells the main model to select forms by information relationships
   and requested actions. Tool descriptions/schemas define each available form.
   Skills describe how to perform and verify work, without prescribing a fixed
   sequence of answer forms. Skill companion tools are nonexclusive suggestions,
   never a capability filter or an instruction to avoid other formats.
2. **Choice follows evidence.** The initial plan names requested outcomes.
   As the information becomes available, refine it with the existing `plan`
   tool where distinct useful parts or specific forms become clear. Use a
   named form when promising that form; a generic card remains valid for an
   outcome composed from several forms. Do not create extra outcomes just to
   exercise tools. A direct answer, creative work or literal output can be prose.
3. **Each representation does useful work.** Choose a form because it helps
   inspection, comparison, sequence, quantitative understanding or interaction.
   Keep exact records and sources reachable without copying the same finding
   into summary, prose, details and sources. Prose connects or interprets rather
   than reprints a structured answer. Uncertain or inconsistent data must remain
   qualified; no required chart, invented datum or false numerical precision.
4. **Surfaces describe reality.** The web and desktop instructions describe
   typed forms and Markdown prose in the shared answer, not Markdown alone.
   Existing transport projections and capability admission remain authoritative.
5. **Receipt checks preserve form.** Keep actual semantic type and base form
   with the accepted card's named deliverable. A specifically planned form needs
   that semantic type or its actual base form, not just a card with the same
   deliverable name. A sources card cannot satisfy a promised chart. A custom
   layout built on stats can satisfy stats; it cannot satisfy a different named
   layout. Generic cards and text remain compatible with appropriate named cards.
   Replace only the addressed receipt when a card is replaced. Delegated cards
   stay unattributed unless the existing delegation contract attributes them.
   Failed/unrecorded calls provide no receipt. Use the same rule in the loop's
   completion/correction path and settled outcome evaluation.
6. **Delivery, evidence and quality are different.** A typed receipt proves
   delivery, not relevance, evidential support, good form selection or absence
   of semantic repetition. Keep existing evidence requirements and visible
   partial-result warnings. Do not add a hidden reviewer call to every turn or
   pretend deterministic validation can judge arbitrary meaning.

## Implementation batch

- Align the shipped prompt, rich surface descriptions, plan guidance, skill
  companion text and shipped procedure skills together. Immutable installed
  packages are updated through normal packaging; never patch their cached files.
- Preserve typed accepted receipts through planning, replacement and settlement;
  exercise wrong-kind substitution and base-form/custom-layout compatibility.
- Keep table numeric refusal as a separate measured field-contract issue:
  formatting or a missing-value marker must not silently disable numerical
  semantics. Do not guess decimal locales or strip punctuation to make it pass.
- Run focused structural checks and a frozen set of real turns on the configured
  OpenAI Luna route. Compare actual calls, plans, errors and projected content,
  then inspect the relevant renderers in the running client. Complete one
  integrated review before a commit; no release/install is implied by design.

## Acceptance fixed before implementation

The installed baseline uses a disposable workspace and the real configured
provider, with [seven fixed prompts](../audits/fixtures/answer-design-scenarios-2026-10-22.json): direct arithmetic, exact literal output,
record inspection, a measured series, an options tradeoff, a dated sequence,
and conflicting supplied evidence. The eighth acceptance category revises an
existing file/interactive artifact and checks its existing identity contract.
The original installed report remains a regression example, not a topic route.

For each scenario inspect requested coverage, fact fidelity, choice of forms,
duplication, evidence limitations, accepted/rejected calls, and actual visible
output. No scenario passes merely because any card exists. No scenario fails
merely because a particular optional form was not chosen. Exact output must
remain exact; unsupported data must not become an authoritative chart. Test
planned-kind substitution, replacement and settlement deterministically.

The baseline is allowed to contradict the original hypothesis: it already
produced a table, stats and a comparison. The investigation therefore does not
establish a global card outage or a universal failure rate. A local Luna design
review recommended this separation, with the explicit warning that structural
corrections do not guarantee semantic quality. Its review is advisory evidence,
not an acceptance verdict.

## Completion gate

All instruction owners agree; both runtime checks use actual active typed
receipts; focused regressions pass; the frozen live scenarios preserve requested
content and exhibit useful form selection without mandatory enrichment; client
inspection shows the expected output; recent display optimizations are unchanged.
Record any unresolved category honestly. Do not call the whole fix complete
because a build, a gallery or a model review passed.

## Verified batch, 2026-10-22

The instruction owners and typed receipt checks are aligned. Replacement,
wrong-form substitution and custom-layout/base-form compatibility have focused
coverage. Library/Agent/judge checks, changed-crate Clippy and dev binaries pass;
the [audit](../audits/answer-design-contract-2026-10-22.md) records exact counts,
the initial failures and their corrected reruns. All seven final fixed scenarios
settle with zero tool-result errors. A loaded-skill check observes the actual
new wrapper/procedure. A two-turn file revision preserves one Library item and
the version-parent relationship. The final chart and its values disclosure work
in the running browser. Client source and display optimizations are unchanged.

This closes the bounded correction, not arbitrary semantic quality. Generic
evidence warnings on supplied-only answers remain a separate open issue. No
additional prompt tuning or topic-specific acceptance rule closes that issue.
