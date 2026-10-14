# Installed 7.2.0: one conversation read back (2026-10-14)

Status: reference

The owner's Agent conversation on the installed 7.2.0 desktop app, hosted
route, Full access: three requests. Read back with `vak export` (through
`SessionLog`), and compared with the screen. What Vak's contract did at each
step, per AGENTS.md "Investigating a failure".

## 1. "How is the Indian stock market doing right now, make a detailed analysis"

What happened: a `plan` of two card deliverables; four webfetches of quote
data (two timed out at 15 s); three shell steps to read Unix timestamps
(`python` is not on the host, `date -d` is GNU-only); then cards: one
summary, two details cards, one research card.

- **Figures were written as sentences in details cards.** NIFTY, SENSEX,
  Bank and VIX went into `emit_details_card` fields such as "22,625.15, down
  96.80 points (−0.426%) from 22,721.95 prior close", drawn as one labelled
  box of paragraphs. The stats card (figures shown large) and the table card
  were admitted. Contract: the details card's description offers itself for
  "a set of readings", and its builder accepts `readings`, so the two cards
  compete for the same content.
- **No chart,** although the intraday series was fetched. The plan named
  only the form `card`, so no kind was asked of any step.

## 2. "Make a document with analysis, add good graph …"

What happened: a `plan` of one file; thirteen `recall` calls, eight of them
repeats (turn 2 three times, the same card twice, the same webfetch
results); a matplotlib attempt (not installed); PIL found; the chart drawn
as a 1500 × 650 PNG and printed as base64 (about 44,000 characters) on
stdout; `office_apply` refused it ("image data must be standard base64");
the chart redrawn as a 540 × 230, seven-colour PNG (3,708 characters) and
written. The draft was then reviewed and accepted.

- **A Word chart has to pass through the model's own output.** `add_image`
  takes the image only as inline base64 (`vak_ooxml::edit::SlideImage`),
  and Word has no chart op (`add_chart` is Excel's). The model copied tens
  of thousands of characters, broke them once, and then shrank the chart
  until it could copy it: the report's graph is small because of the
  argument's form, not the drawing.
- **Repeated recalls.** The same turn, card and results were recalled two
  or three times each within one turn. Not yet explained.
- **The host is unknown to the model.** It tried `python` and `date -d` as
  on Linux; the prompt says nothing of the platform.

## 3. "What is the current weather in Noida"

Two webfetches (geocoding, then current conditions) and a correct answer in
prose, no card. The figures (temperature, feels-like, humidity, wind) are
the stats card's shape; the prompt allows prose "when no card makes it
clearer", and nothing more specific was in reach.

## 4. On screen

- The accepted document appears twice: a draft card with "Review changes"
  and a file card, both "In your folder".
- The first request appears twice at the top of the export (entries 0 and
  1). Not yet explained.

## What follows

Tracked in `docs/plans/open-items.md` ("Installed 7.2.0 run").
