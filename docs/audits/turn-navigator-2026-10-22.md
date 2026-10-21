# Turn navigator and automatic Topics — 2026-10-22

Status: historical. Browser and live-model checks of the shared client and
main-model instruction change; installed native WebKit qualification remains open.

## Observed problem

Supported installed-service HTTP inspection of the main Agent conversation
found 47 turns, automatic Topics enabled, no Topics and no `topics` tool calls.
The screenshot's nine-turn navigator described its loaded history window.
Its labels and Agent subtitle used request/first-message text rather than
accepted plan names. Inspection did not open real-home ledgers or credentials.

## Changes

Wheel/trackpad rolling, captured pointer drag, arrows, Page Up/Down and Home/End
use the existing tick rail. Edge rolling requests one existing history page,
retaining the user's directive address across prepends. Existing history byte
limits and virtualised rendering remain. Partial counts say loaded. Accepted
plan names are joined by canonical request provenance, never page position.
Topic filtering retains its initial empty group. Keyboard focus emphasises the
active tick without a surrounding outline or new border decoration.

Agent subtitles use the latest work's accepted plan names, or the latest
request before planning. Long text truncates; hover exposes its full name and
working/last-worked-on state. Loaded recent history is reused; otherwise one
bounded recent snapshot supplies the preview without changing the reader's
history window. The original conversation title is not used as recent work.

The Topics sidebar refresh now listens to the change signal without depending
reactively on its own loading flag. Main-model instructions explicitly request
placement for ongoing/follow-up/multi-outcome work and preserve human controls.
This is guidance, not guaranteed model compliance or historical backfill.

## Checks

- Client typecheck, web/desktop bundle builds, turn-pairing tests and bounded
  history-page tests passed. Focused vak-core prompt-boundary test passed.
- Actual ChatPane fixture with 100 turns: keyboard Home selected 1, Page Down
  selected 11, wheel rolled forward and pointer drag scrubbed to 96. Preview
  labels used accepted section names. Light/dark at 1440×900 and 390×844 had
  no horizontal overflow. Existing focus outline was then replaced by active
  tick emphasis and checked visually.
- Existing history-scroll fixture with 10,000 synthetic turns: initial window
  held 40; at its first turn, upward navigation loaded one older page (80
  loaded) and selected the preceding addressed request, Question 9959.
- Sidebar fixture deliberately retains title `hi`; its subtitle instead shows
  the latest accepted name, with full-name hover text. No marquee or new box.
- Real model, default connected route, disposable `/tmp/vak-turn-dial-acceptance`
  workspace: an ordinary request for an ongoing reading-club checklist and
  welcome message created `Community reading club launch` with two placements.
  A follow-up for an agenda and feedback form listed Topics and assigned both
  outcomes to the same Topic. Supported CLI confirmed automatic=true, one Topic,
  four placements. No instruction in either request explicitly asked for Topics.

No historical conversation was backfilled; installed services were not replaced
or restarted. Full workspace gate and native-host qualification were not run.
