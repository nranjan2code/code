# Vakyartha: explaining everyday help that continues

Status: historical content study and proposed copy. This document did not
implement or publish a site; later site work is recorded in
`docs/assets/public-site-followthrough-2026/README.md` and
`docs/assets/public-site-security-2026/README.md`.
Reviewed 27 September 2026.

## Recommendation

Make **“An AI helper that stays with the work”** the central product explanation.
Keep the friendly characters, short copy, and everyday situations. Give them a
story that spans a request, useful progress, a decision, and a return to the work.
The visitor should understand what Vakyartha remembers, what remains open, what
it can do with permission, and how they find out what happened.

“Commitment” is worth explaining once, in ordinary language: **ongoing work
Vakyartha keeps track of beyond a single conversation.** It is a commitment the
assistant takes on, not a new obligation for the person to manage. Do not frame
it as guaranteed success or an unconditional promise of unattended execution.

## What was studied

- Live homepage, Examples, How it works, Your control, Get started, Ways to use
  it, and Our name. Homepage also inspected in the browser.
- Public website source, build/export instructions, and the previous visual
  refresh review.
- Status-bearing design records: `47-commitment-kernel.md`,
  `52-outcome-directed-runtime.md`, `64-agent-owned-platform.md`,
  `29-personal-os.md`, `61-adaptive-assistant-experience.md`,
  `70-calm-agent-experience-implementation.md`, and `72-openxml-documents.md`.
- Targeted implementation checks: commitment upkeep and its server caller,
  closure evidence rules, initial criteria, client commitment panel, and the
  scheduled-run refusal for non-git workspaces.

This is a content and product-evidence review. It does not establish usability
results, conversion rates, or successful live execution of every example.
The general web fetch failed, but direct HTTP retrieval and the browser both
reached the live website. The previous visual review says “not published”; the
live site now contains that design, so that historical status is stale.

## Diagnosis of the current site

| Current content | What a visitor learns | What remains unexplained |
| --- | --- | --- |
| “Ask. Then go live your day.” | Help can reduce effort | What carries on, under what conditions, and how they return |
| Dinner, reply, packing, birthday examples | There are familiar things to ask | Almost every story stops at one response |
| “Ask, add a little context, and shape the result together” | Conversation is collaborative | How an ongoing responsibility survives the conversation |
| Eight characters with example activities | The product has personality | Character choices receive more explanation than continuity |
| Access and file review | The person retains control | Waiting for decisions, open work, evidence and spending limits |
| “Bring Vakyartha with you” | Several ways to use the product | Connection requirements and boundaries on sharing history |
| “Get started” leads to a source build | Setup instructions are honest | An everyday visitor faces developer tools before first value |

The visual direction is usable for the next content pass. More explanatory
paragraphs alone would weaken its economy. Replace repetitive examples with
progressive stories and reveal supporting detail on request.

## Message hierarchy

1. **Relief:** less to keep in your head.
2. **Continuity:** return to ongoing work and see what remains.
3. **Useful results:** a draft, reviewed file, clear comparison, or practical plan.
4. **Control:** chosen access, visible decisions, spending limits and honest status.
5. **Personality:** a familiar companion whose style the person can choose.
6. **Technical explanation:** open source, local models, execution and inspection.

Everyday readers need to recognise a situation before learning a product term.
Lead with “the update due Friday,” “the document I need to finish,” or “the trip
I’m still planning.” Introduce commitments after showing one.

## The architectural strengths worth making visible

The follow-up brief asks for the wider architecture to inform the public story.
These are the strongest translations. The middle column is proposed public
copy, not a claim that a new interface or demonstration has been implemented.

| Architectural foundation | Attractive everyday message | A concrete way to show it |
| --- | --- | --- |
| Intent and outcome understanding (`47`, `52`) | **Start with what you want to get done.** Bring the goal, the rough idea, or the notes you have. | A messy paragraph becomes a clear request with one useful clarification. |
| Commitments and managed work (`47`, `42`) | **Keep the important things moving.** See what is done, what remains, and what is waiting for you. | Return to an unfinished proposal; the missing price is still visible. |
| Context and scoped memory (`68`, `23`, `64`) | **Pick up the thread.** Bring relevant earlier conversations and saved preferences into the work. | Recall the audience and tone agreed earlier, with editable saved preferences. |
| Learning and reviewed skills (`26`) | **Keep the ways of working that work for you.** Save useful preferences and approve reusable approaches. | Reuse an approved weekly-update format; distinguish a remembered preference from a newly approved skill. |
| Capability discovery and governed tools (`41`, `09`) | **Bring your files. Connect what you need.** Give your helper access to the tools that make the work useful. | Read a provided document; use an already configured search connection when needed. |
| Semantic presentation and interactive results (`30-output-engineering`, `65`, `66`) | **Something you can use.** Explore a comparison, inspect a chart, open a preview, or work with a draft. | Sort a comparison table, open the related file, then ask for a revision beside it. |
| Office reading, citations and review (`72`) | **Work with the documents you already have.** Find the important parts and review proposed changes. | A question about a spreadsheet points to cells; a document edit shows the changed passage. |
| Permission boundaries and scoped execution (`24`, `64`) | **You choose how much it can do.** Set access and review the decisions that need you. | Show the allowed folder and a real approval request; avoid an inaccurate “asks every time” promise. |
| Evidence-backed closure (`47`, `52`) | **See what happened. Know what is still open.** Look at the result and the checks behind it. | A draft is ready, delivery has not happened, and an unavailable check is labelled clearly. |
| Scheduled tasks, inbox and heartbeat (`29`) | **Make room for the things that repeat.** Set up recurring help and find the results and decisions in one place. | A verified configured routine with its next run and delivered result; disclose runtime and workspace prerequisites. |
| Resilience and partial-output preservation (`15`, `31`) | **Keep the progress you have made.** Temporary interruptions have recovery paths, and stopped work retains partial output. | Show an interrupted run's saved partial result. Avoid “nothing is ever lost” or “always finishes.” |
| Provider choice and spending controls (`01`, `29`, AGENTS invariants) | **Choose the AI service and set your limits.** Use a supported service or local model, with spending controls. | A plain-language usage explanation and configured limit; avoid promising all models have equal quality or capability. |
| Agent identity and channel boundaries (`64`) | **A familiar helper, in a style you choose.** Keep a conversation with your chosen Agent and use configured ways to reach it. | Show one chosen character in the app and a configured chat, without implying unrestricted shared history. |

The numeric shorthand in this table refers to the full design filenames listed
in this report or the architecture catalogue; for 30 it explicitly means
`30-output-engineering.md`, not `30-render-architecture.md`.

### Six messages for the public homepage

Use these as a story sequence, not twelve engineering features in a grid:

1. **Start with what’s on your mind.** An ordinary request, an unfinished draft,
   or an idea you want to explore.
2. **Pick up the thread.** Return to the work with relevant context available.
3. **Keep important things moving.** See progress, open work and decisions.
4. **Get something you can use.** A practical result you can inspect and refine.
5. **Stay in charge.** Choose access, review changes and set spending limits.
6. **Make it feel like you.** Choose a companion and keep useful preferences.

The first four establish usefulness and continuity. Control makes the promise
credible. Personality makes the experience recognisable. Deeper explanations of
learning, scheduling, model choice and resilience belong in expandable stories
and supporting pages.

### A stronger demonstration than a feature list

Use one scenario to reveal several architectural strengths naturally:

**“Help me finish the proposal for Friday.”**

- The person brings rough notes and an existing document.
- Vakyartha drafts an update, using the relevant prior discussion where it is
  available to that Agent and audience.
- It identifies an unresolved price instead of inventing one.
- The person returns with the price and asks for a shorter opening.
- The revised document is available for review, with the change visible.
- The result says the draft is ready; it does not claim it has been sent.

This single story makes understanding, context, continuation, useful output,
honesty and user control visible. Label it illustrative until the exact flow is
recorded from the running product. A separate routine story should demonstrate
scheduling; a deadline in this story is not proof of automatic background work.

### Important exclusions and qualifications

- **Voice:** `49-live-voice.md` ships a foundation but explicitly leaves realtime
  streaming and physical-microphone acceptance open. Do not sell effortless
  full-duplex voice conversation today. Demonstrate a verified supported voice
  path before making it a headline.
- **Adaptable presentation:** `57-adaptive-presentation-runtime.md` is in
  progress and contains historical UI/theme wording. Use the current UI and
  `AGENTS.md` for everyday labels; demonstrate implemented result types rather
  than promising a fully self-redesigning interface.
- **Learning:** useful saved notes and reviewed reusable skills do not mean
  autonomous self-modification or guaranteed improvement after every use.
- **Connected services:** extensibility does not mean every popular service is
  installed, connected or available to every person.
- **Privacy and ownership:** local storage does not mean all processing stays
  local. Relevant content can go to the chosen AI service and connected tools.
- **Roadmap:** the future encrypted storage/catalog/cloud model in the pending
  data architecture is not a current product claim.

Additional architecture records read for this expansion: `23-memory.md`,
`26-learning.md`, `31-network-resilience.md`, `41-capability-registry.md`,
`42-managed-work-contracts.md`, `49-live-voice.md`,
`57-adaptive-presentation-runtime.md`, `65-universal-adaptive-platform.md`,
`66-immersive-artifact-canvas.md`, and `68-context-engine.md`. Their stated
implementation status was checked; this was not an exhaustive code audit of
those subsystems.

## Proposed homepage copy and order

### 1. Hero

Eyebrow: **Less to keep in your head.**

Headline: **An AI helper that stays with the work.**

Supporting copy:

> Bring the everyday jobs, unfinished plans and ideas you want to make real.
> Vakyartha helps you move them forward and keep track of what still needs you.

Primary link: **See how it follows through** → the continuity story.

Secondary link: **Explore everyday examples** → Examples.

Retain “Ask. Then go live your day.” as a closing brand line if desired. The
supporting explanation must make background-running requirements discoverable.
Keep MIT/source information available in the footer and setup page.

### 2. A continuity story

Headline: **Some things take more than one conversation.**

Use one short, explicitly illustrative story:

| Moment | Example content |
| --- | --- |
| Start | “Help me get this proposal ready for Friday.” |
| First result | “Here’s a draft from your notes. The price still needs your input.” |
| Return | “The price is ₹18,000. Make the opening shorter, too.” |
| Review | “The draft is updated. Review the changes before keeping the new file.” |

This establishes continuity without suggesting that a Friday deadline alone
creates a background schedule. A second, expanded story can explain an actual
configured recurring task after its setup-to-delivery journey has been verified.

### 3. Explain the important idea

Headline: **A place for what’s still open.**

> For work that carries on, Vakyartha can keep a commitment: the goal, progress,
> and what still needs attention. You can see what is open and what is waiting.

Expansion label: **What is a commitment?**

> An ongoing piece of work that can outlast one conversation. A quick question
> can stay a quick question. Recurring work needs a schedule and a running
> installation; connected services need to be set up and allowed.

Any depicted progress view should use real product screenshots or be labelled
as an illustration. Do not present a new simplified commitment interface as
already available.

### 4. Everyday breadth

Preserve the three existing scenes, but vary the kind of help:

- **Make today easier:** dinner plan, short shopping list, a reply to revise.
- **Move something forward:** finish a proposal, review a document, understand a
  spreadsheet with references to the relevant cells.
- **Make room for an idea:** develop a birthday plan, learn a topic across a
  conversation, shape a side project into a useful first result.

Each expanded example should show **request → result → next step**. At least
one should contain missing information, and one should show a revision.
Characters accompany these stories; any character can help with any supported
task. Do not imply eight separately installed specialists are required.

### 5. Trust in everyday words

Headline: **Help you can keep an eye on.**

- **See where things stand.** Find the result, what remains open, and what needs
  a decision.
- **Choose what it can use.** Control access to folders and connected tools.
- **Review changes.** Inspect supported file changes before accepting them.
- **Set spending limits.** Put bounds on AI usage and see when work reaches them.

Link to Your control for scope and caveats. Avoid “always asks before acting”:
permitted actions may proceed without another prompt.

### 6. Closing and setup

Headline: **Start with one thing on your mind.**

Link: **See setup options**.

Adjacent expectation: **Currently available to build for macOS and Linux.**

Do not imply a hosted consumer account or one-click installer exists. A future
assisted setup or notification list needs an actual supported flow before being
offered as a button.

## Changes to the rest of the website

| Page | Recommended change |
| --- | --- |
| Examples | Organise by recognisable needs; include immediate help, returning to work, and a separately labelled configured routine. Keep characters visible. |
| How it works | Tell the full story: ask → work together → decide when needed → review the result → return later. Show a blocked step as well as a success. |
| Your control | Add spending, waiting for approval, incomplete results, and memory boundaries to the existing access and privacy explanation. |
| Ways to use it | Name desktop, browser and supported connected chats plainly. Explain that connections need setup and history/access are scoped. Avoid implying automatic cross-channel identity or shared history. |
| Get started | Put current availability and requirements first; distinguish new setup from opening an existing installation. Explain software cost versus AI-service usage cost. |
| Our name | Connect “meaning of a sentence” to listening, understanding the intended result, and helping carry it forward. Keep this brief. |
| Footer | Keep documentation, source, wallpapers and technical detail accessible without competing with the main journey. |

Suggested top navigation remains compact: **Examples · How it works · Your
control · Get started**. Make continuity the centre of How it works rather than
adding a technical-sounding “Commitment kernel” page to primary navigation.

## Questions an everyday visitor needs answered

Use short answers with optional detail:

1. What can I give it to do today?
2. Can I come back to the same work later?
3. What does it remember, and who can see that information?
4. Can it do something regularly, and what needs to stay running?
5. What happens when it needs my approval or cannot continue?
6. Does it send messages or change files automatically?
7. How do I see what was actually completed?
8. What can it cost, and can I limit spending?
9. Where do my requests and files go?
10. What do I need to install or connect?

Answer memory questions as scoped retention and retrieval, not perfect recall.
Answer ongoing-work questions with setup conditions beside the claim, not hidden
in terms. Explain unknown, blocked and unsuccessful outcomes as normal states.

## Claim boundaries from the implementation review

| Topic | Evidence supports | Avoid implying |
| --- | --- | --- |
| Durable commitments | Agent-owned records, open states, episodes, blockers and closure rules | Every request automatically becomes a commitment |
| Continuation | Work can retain identity across turns and durable records outlive sessions | Perfect interpretation of every correction or replacement |
| Background work | Task scheduling and services exist | A saved commitment automatically dispatches new work indefinitely |
| Commitment upkeep | Clock/condition maintenance changes state; its server caller reports selected events | Every resumed commitment triggers a model run or user notification |
| Verification | Fulfilment must meet the commitment’s configured evidence requirement | Every answer is independently verified; some criteria are semantic or absent |
| Memory | Agent and audience boundaries, authorised recall | Everything is remembered everywhere or shared across all characters/chats |
| Office work | Document reading, citations, changes and review are documented as shipped | A complete Office replacement or that all new-file journeys were reverified here |
| Calm interface | Some experience slices are implemented; design 70 explicitly remains in progress | Every reference mockup is a screenshot of the current app |
| Setup | Public source build for macOS/Linux; AI service or local model required | Instant consumer onboarding or a hosted service account |

Two concrete checks deserve attention before using recurring personal examples
as headline proof:

- `commitments::maintain` explicitly does not dispatch a model. The inspected
  server upkeep loop logs resumed commitments; that transition alone is not
  proof of automatic follow-through. Test the entire schedule/dispatch/delivery
  path intended for the marketing example.
- Scheduled runs currently refuse a non-git workspace. That is documented M0
  debt awaiting the authorised data-architecture work. Do not imply ordinary
  personal folders support frictionless recurring work today.

Neither finding prevents explaining commitments. They determine the difference
between a credible product explanation and an unverified automatic-work claim.

## Wording guide

| Internal term | Everyday explanation |
| --- | --- |
| Commitment | Ongoing work; what is still open |
| Intent | What you want to get done |
| Episode | A step forward; a period of work |
| Suspension / human gate | Waiting for your decision |
| Evidence / receipt | What was checked; what happened |
| Authority / envelope | What you have allowed it to do |
| Spend cap | Spending limit |
| Artifact | Document, plan, spreadsheet, draft, or the actual thing produced |
| Surface | App, browser, or connected chat |
| Provider | AI service |

Keep “commitment” in the explanatory layer because it expresses a useful product
idea. Keep kernel, resolver, ledger and routing out of the everyday sales story.

## Order of work and acceptance

1. **First content pass:** hero explanation, one continuity story, commitment
   explanation, richer examples, and clear setup expectations.
2. **Proof pass:** record one actual return-to-work journey, one approval wait,
   and one configured routine with delivery. Use observed outputs in demos.
3. **Supporting pages:** align How it works, Your control, Ways to use it and
   Get started with those same claims.
4. **Audience check:** ask a few nontechnical readers what it does, what carries
   on, what needs permission, and how they would start. This study includes no
   user testing, so comprehension and conversion gains remain hypotheses.

The content pass is successful when a visitor can explain, in their own words:
“I can bring it something to work on, return to it, see what remains, and keep
control of the decisions.” They should also understand the current setup burden.

Keep the existing visual constraints: short default copy, expandable examples,
varied scenes, all eight character identities, no repeated feature-card wall,
keyboard support, reduced motion and both themes. Before publishing an eventual
implementation, verify desktop/mobile layouts and links, rebuild the committed
bundle, and confirm every demonstrative claim against the running product.

## Evidence links

- Live: [Home](https://vakyartha.com/), [Examples](https://vakyartha.com/outcomes),
  [How it works](https://vakyartha.com/tour), [Your control](https://vakyartha.com/security),
  [Get started](https://vakyartha.com/install), [Ways to use it](https://vakyartha.com/surfaces).
- Repository: `crates/vak-server/site/README.md`;
  `docs/design/47-commitment-kernel.md`; `docs/design/52-outcome-directed-runtime.md`;
  `docs/design/64-agent-owned-platform.md`; `docs/design/29-personal-os.md`;
  `docs/design/70-calm-agent-experience-implementation.md`;
  `docs/design/72-openxml-documents.md`.
- Code: `crates/vak-core/src/commitments.rs` (`seed_criteria`, `maintain`);
  `crates/vak-commit/src/types.rs` (`may_close`);
  `crates/vak-server/src/lib.rs` (commitment upkeep, scheduled-run admission);
  `crates/vak-client-ui/src/components/CommitmentsPanel.tsx`.
