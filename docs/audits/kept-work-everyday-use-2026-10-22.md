# Kept work — installed everyday-use checks, 2026-10-22

Status: historical — real model/tool runs and desktop-browser interaction
against installed 8.23.0/build 9a6fb63fc. The lifecycle proposal is not
implemented. This is bounded evidence, not full acceptance or a claim of
device/platform coverage.

## Method and scope

The maintainer asked to try everyday requests in local `vak-home`, without
expecting people to be technical. They then clarified that free-form edits
must create versions of the same artifact, with one Library card.

Initial requests ran through the installed CLI, each in a separate disposable
folder beneath
`/Users/nisheethranjan/vak-home/everyday-tryout-20261022.CjnARQ/`, with the
existing real data home/provider route. The CLI used its existing approved
FullAccess policy and `--yes`; no permission/configuration was widened or
changed. Prompts named user outcomes, not APIs, commands, filenames, server
ports or framework recipes. The active-home scheduling control used the
actual `vak-home` workspace. Existing files were not revised by the tests.

Generated results were opened using the real web client at the installed
gateway's `/app`, in the in-app browser on this Mac. The follow-up edit was
typed into the ordinary conversation composer. Native-app automation was
attempted but capture/control failed; its attempted meal prompt was not
submitted, verified through supported transcript readers. Native-shell and
phone/light/dark matrix acceptance are not claimed.

Records were read through `vak data cat`, `data grep`, `runs`, `triggers`
and the Library UI. No raw encrypted ledger/credential files were opened.

## Requests and observed outcomes

### Saved checklist — passes the basic saved-file request

Prompt: “Make me a simple packing checklist for a two-day work trip. Save it
so I can come back to it. Keep it short and practical.”

Vakyartha created and saved a readable checklist, with work essentials,
clothes, toiletries and pre-departure items. It appears in the Library.
No server or automation was created. Its CLI hand-off still names a filename
and workspace rather than a plain Open action; that is a surface limitation,
not proof of interactive-app delivery.

Session: `01a16279-c80b-7aa2-b4d7-107438fb099f`.

### Interactive meal planner — saved, but default use fails

Prompt: “Make me a little weekly meal planner I can use here. I want to pick
meals for each day and see a shopping list. Use a few example meals to start
and make it easy for me to open and use.”

A standalone page was saved. Its reported checks establish HTML markers and
JavaScript syntax. Opening it through the normal Library/card Open action
shows “Page code paused · offline preview.” The day/meal controls are generated
by scripts, so the visible planner has no days or meal choices; its shopping
list remains empty. A successful syntax check and a saved page did not
establish that the person could use it.

The preview permission explanation discloses code/network implications and
offers Run page code. That switch was inspected, not enabled. No security
boundary was bypassed to make the test pass. Functional meal selection,
clipboard access and saved-choice recovery remain unverified. The generated
program uses browser localStorage rather than Vakyartha durable state;
cross-origin/reopen persistence was not established.

Session: `01a16279-c80f-7382-af7a-d891e9b49889`.

### Updating clocks — falsely ready in the delivered default view

Prompt: “Make a small page showing the current time in Delhi, London and
New York. Keep the clocks updating while I have it open, and make it easy
for me to use here.”

The page was saved and marked Ready/Live page. Its test card said the three
checks passed, including “Refreshes every second”; the corresponding command
only asserted source markers and checked JavaScript syntax. In the actual
Canvas all three clocks show `--:--:--` because page code is paused. This is
an observed delivery/verification contract gap; it does not justify automatically
running model-authored scripts.

Session: `01a16279-f9b6-75e3-8578-3f8f1d0c8d84`.

### Free-form edit and Library versions — identity/lineage pass

In the meal planner's ordinary conversation composer, with no file/id/version
specified: “Could you add a little notes box for each day, and move the
shopping list to the top?”

The agent recalled the prior work, read the existing file and edited it. The
saved bytes contain the notes fields and put the shopping section before the
week. The artifact id remained
`art_00000000-0000-7bef-9a32-0066d471b032` in catalog search. The Library detail
showed Version 2, retained Version 1, and “Made from” linked Version 2 to the
original “Created standalone weekly planner” version. It recorded the exact
follow-up request. A filtered Library listing showed one planner card with
“2 versions”; no separate artifact/card was created for the edit.

Limit: the already-open Library detail/list retained the old version count
and timestamp during the change. Reopening the detail fetched Version 2;
a full page reload/re-entry showed the listing's new count and timestamp.
Identity/version storage works in this case; live metadata reconciliation
without manual refresh is not established. The edited interactive preview
still has code paused, so usable notes interaction is not a pass.

### Three-minute status page in a try-out folder — fails the full request

Prompt: “Make me a small status page showing whether https://example.com
is reachable. Check it once a minute for the next three minutes, keep the
latest result visible here, then stop. I do not want messages sent anywhere
else.”

The initial request returned HTTP 200. The agent did not create the requested
status page. It registered three one-time model-prompt automations at 12:51,
12:52 and 12:53 IST, claiming their results would appear in the conversation.
An initial minute-resolution time was already past and was corrected.

No scheduled Run or result appeared by 12:53:22 IST. The code explains the
coverage boundary: `of_this_space` in `crates/vak-server/src/automations.rs`
filters to the server's Space. The resident gateway serves `vak-home`, not
the independently bound try-out folder. The shared registry accepted the
triggers without establishing a responsible runner. This is not evidence
that every installed scheduler is broken; it is evidence that registration
and “will run” are different claims.

All three test triggers were paused through the supported CLI after the
observation. No channel delivery was configured. Session:
`01a1627a-95f2-7723-934e-b11aa15af3fb`.

### One-time check in the active home — execution and delivery pass

Control prompt in actual `vak-home`: “For a quick try-out, check whether
https://example.com is reachable once, two minutes from now, and show the
result here. After that stop. Do not send it anywhere else.”

Vakyartha registered a deterministic script for 12:56 IST. The installed
gateway claimed the slot and completed Run
`run_01a16280-e1f9-7541-8572-bb9227358dcc` at 12:56:16 IST. The source
conversation gained a scheduled result, `HTTP 200`, after the initiating
CLI process had ended. This establishes one actual scheduled run/delivery,
not a recurring cadence or a hosted status page. No model was required for
the scheduled script execution.

Usability gap: the scheduled transcript includes the full shell command and
the result is the bare protocol status. It does not give the person a plain
“The site is reachable, checked at …” result. The scheduled setup itself
was expressed in ordinary words without manual environment configuration.

Session: `01a1627e-f36d-7373-8363-9eb08e0b8d05`. Trigger:
`trg_01a1627f-373c-7fd1-b594-9b4ca2aef833`. It is one-shot, with no further
slot after completion; it is paused after verification as test cleanup.

## Evidence locations and limits

The full script-admission matrix was not run: allowed execution, permitted
versus denied connections, WebRTC/network isolation, resource cancellation,
revision-bound consent, revoke/reload/reopen recovery, clipboard and native
engine parity remain unqualified here. Observing paused scripts establishes
the default behaviour only, not the correctness of the whole permission model.
The follow-up design and explicit gates are doc 103 §6.1 and plan K2.

Local logs: `/tmp/vak-everyday-packing.log`, `/tmp/vak-everyday-meals.log`,
`/tmp/vak-everyday-updates.log`, `/tmp/vak-everyday-monitor.log`,
`/tmp/vak-everyday-home-schedule.log`.

Local screenshots: `/tmp/vak-everyday-clock-paused.png`,
`/tmp/vak-everyday-meals-paused.png`,
`/tmp/vak-everyday-planner-history.png`,
`/tmp/vak-everyday-library-one-card.png`. Screenshots remain local rather
than copying unrelated real-home UI context into repository assets.

These are real installed observations, with initial generation through CLI
and the edit/view/history checks through the web surface. They do not certify
the new proposal, restricted-mode admission, native WebKit, other engines,
mobile, durable browser state or arbitrary domains. Runtime fixes remain
open in the tracker and [kept-work plan](../plans/kept-work-plan.md).
