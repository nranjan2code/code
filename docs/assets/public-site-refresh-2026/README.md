# Public site refresh, 2026-09-27

Status: implemented locally, reviewed in the browser, not published.

Owner direction: rebuild the public site from scratch for everyday people and
technical users, with very little text; reject the paper aesthetic, repeated
marketing cards and em dashes; make it modern, lively across generations, and
show all eight existing characters doing things.

The final composition uses three activity illustrations, varied placement,
large sans-serif type, white and charcoal backgrounds, and native expandable
requests. The homepage includes Vakyartha, Moss, Beni, Mira, Pip, Nori, Lumi and
Tavi. The illustrations are explicitly labelled examples, and the characters
are not presented as fixed capability roles.

## Evidence

`desktop-{light,dark}-*.png`: 1440 × 900 viewport captures.
`mobile-{light,dark}-*.png`: 390 × 844 viewport captures.
Every one of the seven routes was checked in all four combinations. No
horizontal page overflow or broken image was observed. Lower-page work and
learning scenes were also inspected directly. Viewport captures are used
because the browser's full-page capture produced stitching artifacts.

Verified interactions:

- Home request disclosure opens its corresponding example result.
- Native request disclosure still opens with JavaScript disabled.
- Walkthrough tabs select the everyday, work and code example; arrow keys move
  selection and focus together.
- Theme toggling updates the page and the control's accessible label.
- The review illustration explains both keeping and declining the change,
  explicitly stating that no real file was changed.
- Reduced-motion emulation leaves the result visible and uses automatic,
  non-animated scrolling. Test emulation was cleared afterwards.

Verified checks:

- `cargo test -p vak-server --lib site::tests`: four passed. Includes the new
  scene assets, public routes, trailing slashes and public-data fetch boundary.
- `python3 crates/vak-server/site/build.py --check`: passed.
- `node --check` on the shared script and every generated inline script: passed.
- Local public-page links and fragment targets: passed.
- Visible site source contains no em dash or em-dash entity.
- `git diff --check`: passed.

No live AI requests, real file mutations, external sign-in, production
publication or full Rust workspace suite were part of this static-site review.
The local preview serves generated static pages and brand assets, plus a fake
public build stamp. `/app` and `/admin` need the real Vakyartha server.

## Artwork

Original generated PNGs and the exact built-in imagegen prompts are in
`docs/brand/library/public-site-scenes/`. Website-ready WebP copies are in
`crates/vak-server/site/src/assets/` and are embedded under `/site/`.
The official logo and character master exports were not changed.
