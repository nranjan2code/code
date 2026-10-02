# Mail and calendar browser evidence

These captures show the real Settings work area mounted by
`/app/tests/mail-calendar-connected-review.html` against the fixture's
synthetic Google account (`owner@example.test`). The event, date, location and
description are fabricated. The visible local preview says it does not send
email, invite attendees, or change a calendar. The fixture refuses effect
routes and uses no credentials or provider traffic.

The editor and its local preview were captured at 1440 × 900 and 390 × 844 in
light and dark themes. The phone captures with `390-preview` in their names
scroll the preview into view so its lower fields can be inspected separately.
The exact email-send and event-create Review dialogs are also captured at both
sizes and themes. Their final action buttons were not activated; the fixture
rejects provider-effect routes.
The fixture mounts Settings directly, so the theme attribute was set for these
captures; they demonstrate component rendering at those dimensions, not theme
selection through the full application or native-device behavior.

The connected-review fixture passed 22 checks, including source-linked reply,
draft save/reopen, both concurrent-edit resolutions (load the latest version
or save local work as a separate draft), exact effect Review, keyboard focus
handling, event draft deletion, and paused routine preview/history/resume. Axe
reported zero violations (38 passes, 51 inapplicable, no incomplete rules) on
the rendered fixture; the open exact-email Review dialog also audited with zero
violations (40 passes, 49 inapplicable, no incomplete rules). These checks use
synthetic same-origin responses and do not establish provider conformance or
screen-reader testing.

The `daily-calendar-*` captures show the actual Today canvas with a synthetic
load of nine connected accounts, 72 recent messages, 300 timed events and 24
free/busy intervals. The calendar retains a minimum width per overlapping
event lane and scrolls inside its own frame. This keeps the phone page at 390px
wide while preserving usable event labels; selecting one calendar makes its
timeline roomier. The filter remounts the calendar layout so it recomputes
overlap lanes for the selected account. `daily-calendar-focused-390-light.png`
shows the focused view, and `daily-calendar-event-1440-light.png` shows the
selected event details and its local-draft next action. The daily browser
fixture passes 19 checks at both 1440 × 900 and 390 × 844. These captures
contain only fabricated fixture data and no provider requests. The browser
viewport was set to the named dimensions, and the fixture theme was set
directly for each capture.
