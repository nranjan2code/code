Scheduled Work You Can See
==========================

Status: in progress: S0 built 2026-10-15 (the owner's time zone, set in
Settings, `at` in local time, a cron always names its zone, the gap
rule, with fixed-clock tests); S1 built 2026-10-15 (`home`, `made_by`,
delivery into the conversation); S2 to S5 not built.
Agreed with the maintainer on 2026-10-15. Paths below describe targets and may not exist yet. Rewrites the
experience of automations; the engine stays `vak_core::triggers`
(doc 73 §8, invariant 38).

## 1. What is wrong

Checked on the installed 8.1.0 on 2026-10-15: the engine fires on time
(a one-minute script ran every minute; a prompt answered "pong"; asking
the Agent "every weekday at 8am, a motivational line" created
`0 8 * * 1-5` in Asia/Kolkata). What fails is everything a person sees.
The maintainer reported all four of these:

- **The result never shows where it was asked for.** A prompt
  automation runs in a conversation of its own, in a copy of the folder,
  and its only output is an Inbox note ("automation 'x' finished"). The
  conversation where the person asked shows nothing at the time. `vak
  inbox` read the Agent's own scope while the scheduler writes the
  shared one, so the CLI showed nothing at all (fixed 2026-10-15).
- **Setting one up needs cron.** The app's dialog asks for a cron
  expression or a number of minutes; no person writes `0 8 * * 1-5`.
- **Finding and managing one is hard.** The dialog is reached from one
  place, shows ids and raw fields, and offers enable, run now and delete
  but no change of time, no history, no plain words. The Agent's tool
  can list, add, enable, disable and remove, but cannot change a
  schedule or run one now.
- **It ran at the wrong time.** Not reproduced yet; S0 audits every way a
  slot can be computed (an interval's anchor, a cron with no timezone
  read in the host's zone, daylight-saving gaps and repeats, catch-up,
  one start per slot) with tests that fix the clock.

## 2. Decisions

The maintainer, 2026-10-15:

- **D1 Results go to the conversation.** A scheduled result appears in
  the conversation it belongs to, as the Agent's turn marked as
  scheduled, with a notification; the Inbox lists it too.
- **D2 Three ways to steer, one contract.** A card in the chat when an
  automation is made or changed (pause or resume, run now, change time,
  delete); an Automations page with everything; and asking the Agent
  ("pause my morning line", "move it to 9"). All three call the same
  operations.
- **D3 Rewrite, don't patch.** There are no users: a screen or contract
  that is wrong is replaced, with the tests that pinned it (invariant
  30).

## 3. The model

A trigger (`vak_core::triggers::Trigger`, a Document) keeps its engine
fields and gains, additively (invariant 29):

- **`home`**: the conversation it belongs to, `{agent, session}`. Set
  from the conversation of the call that created it; one made on the
  Automations page names the conversation chosen there (default: the
  Agent's most recent). This replaces the meaning of "no `deliver_to`
  means the Inbox".
- **`deliver_to`** stays the one way to send it elsewhere too (a
  channel target such as `telegram:12345`), never a second field.
- **`made_by`**: the turn or the person that last created or changed it
  (`{session, turn}` from the call's trace key, or the owner), so the
  chat can show the card under the turn that made the change without
  reading any tool's name.

How a schedule is said is one function, `triggers::describe(&Schedule,
now) -> String` ("Weekdays at 8:00 AM", "Every 2 hours", "Once, on
Friday 17 October at 9:00 AM"), used by the tool's results, the CLI,
the card and the page, so every surface says the same words.

## 4. Delivery into the conversation (D1)

When a prompt run settles, its result is appended to the home
conversation as a turn of its own (`vak_core::scheduled::deliver`): the
automation's own words as the directive and the run's answer as the
Agent's message, both marked with `MessageMeta::scheduled` (the trigger,
its name, its run id, the run's own conversation for "see how it got
there", and whether it failed). Two ordinary messages, typed rather than
a new entry type, so every ledger path (derivation, the turn index,
recall, search, export) treats it as the turn it is (invariant 1: it is
reconstructed from the ledger), and a turn card closes it so the next
request follows it like any turn: "make it shorter" in reply works. The
directive is the owner's words from when they made the automation,
never words attributed to them now. What the run's turns held is
recorded on the delivered turn (`CallEffect::Provenance`, invariant 43).
Appending waits at the conversation's turn boundary
(`automations::deliver_home`), so it never lands inside a turn the
person is in. The projection marks both items (`scheduled_*` metadata);
the client draws a "Scheduled · <name> · <time>" head in place of the
person's bubble, then the answer; the turn's end notifies like any turn
(doc 29 P6). The run's cards arrive in their deterministic text form,
as channels get them (`text_with_run_cards`); drawing them as cards in
the home conversation is open. A script automation delivers only when
it prints or fails. A run refused before it starts delivers its reason,
never silence (invariant 38's `RoutineFailed`). The Inbox keeps its note.

The run itself still works in a copy (invariant 38), and anything it
changed still comes back for Review; delivery is the answer, not the
changes.

## 5. Steering (D2)

**Operations**, one set, each an existing or new route over
`vak_core::triggers` (and so over `claim_due`/`delete`, the data
rules): create, change (name, what it does, schedule, where it
delivers), pause, resume, run now, delete, and history (its run
records). The server routes, the CLI (`vak triggers`), the Agent's tool
and the client all call these.

**The Agent's tool** gains `update` (any field) and `run_now`, accepts
an automation by name as well as id (a name that matches two is refused
with both), and answers with `describe`'s words and the next run.

**The card in chat.** When a call in a turn creates or changes an
automation (`made_by` names that call), the conversation shows an
automation card under the answer: its name, `describe`'s words, next
run, last result, and buttons: Pause or Resume, Run now, Change time,
Delete. It is drawn by the client from the trigger record, not a card
the model emits (card forms are the model's; this is the runtime's own
record, like doc 93 §9.1's activity line).

**The Automations page** replaces `AutomationsModal`: every automation
of the Agent (and, filtered, of all Agents), each with plain words,
next run, last result and its state; a history of runs with their
results (opening a result opens its turn in the conversation); create
and change through a schedule picker, never a cron box: Once at…,
Every day at…, Weekdays at…, Every week on… at…, Every N minutes or
hours, and "Custom" (cron) only behind Show technical details. The
timezone defaults to the device's and is shown.

## 6. Times (S0)

Found in the engine (2026-10-15):

- **No one knows the person's time zone.** A cron with no `timezone` is
  read in the host's local time, and the per-turn time line names the
  host's zone and tells the model the person "may be in another time
  zone" on chat and web. On a server in UTC, "8 AM" is 8 AM UTC.
  Rewrite: the owner's time zone is one setting (`[owner] timezone` in
  the Shared layer, shown in Settings, defaulting to this machine's
  zone), the time line names it as the person's, and every schedule
  stores its zone explicitly when it is made (a cron with none is made
  in the owner's zone, never left to the host).
- **A one-off reminder asks the model for arithmetic.** `due_at` must be
  an RFC3339 instant with its offset, converted by the model from the
  person's words. Rewrite: the tool takes the local date and time
  (`at: "2026-10-17 09:00"`) and the runtime reads it in the owner's
  zone; an offset given is honoured, a time already past is refused with
  the current time.
- **A daylight-saving gap skips the day.** A cron slot inside a skipped
  hour moves to the next day's match. Rewrite: it runs once, at the
  first valid minute after the gap; a repeated hour runs once, at its
  first occurrence (as now).

Every slot computation gets a test with a fixed clock: interval anchored
at its creation (not at the process start), cron in its own timezone
(the person's, captured at creation, never the host's when they differ),
a slot inside a daylight-saving gap runs once at the next valid time, a
repeated hour runs once, catch-up after sleep runs the latest missed
slot once (or none when disabled), and two processes start a slot once
(`claim_due`). What a test finds wrong is fixed in the engine.

## 7. Stages

Each is done when its "done when" holds with the full gate green and,
for a screen, in the running app at 1440 × 900 and 390 × 844, light and
dark. Tracked in `docs/plans/open-items.md` "Scheduled work".

| Stage | What | Done when |
|---|---|---|
| **S0** | Times (§6): the owner's time zone, `at` in local time, the gap rule, fixed-clock tests | "remind me at 9" is 9 in the owner's zone on any host; each case has a fixed-clock test |
| **S1** | `home`, `made_by`, `ScheduledResult` delivery (§3, §4) | an automation made in a conversation delivers its next result into that conversation, model-visible; a failure delivers its reason |
| **S2** | `triggers::describe` and the schedule picker's vocabulary (§3, §5) | every surface says the same words for a schedule; the picker makes each kind without cron |
| **S3** | One set of operations; the tool's `update`, `run_now`, by name (§5) | "move my morning line to 9" changes it; "run it now" runs it |
| **S4** | The automation card in chat (§5) | a turn that made or changed one shows the card; its buttons work |
| **S5** | The Automations page (§5) | create, change, pause, run now, delete and history work from the page |
