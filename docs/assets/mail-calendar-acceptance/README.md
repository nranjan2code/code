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
The fixture mounts Settings directly, so the theme attribute was set for these
captures; they demonstrate component rendering at those dimensions, not theme
selection through the full application or native-device behavior.

The connected-review fixture passed 20 checks, including source-linked reply,
draft save/reopen, exact effect Review, keyboard focus handling, event draft
deletion, and paused routine preview/history/resume. Axe reported zero
violations (38 passes, 51 inapplicable, no incomplete rules) on the rendered
fixture. These checks use synthetic same-origin responses and do not establish
provider conformance or screen-reader testing.
