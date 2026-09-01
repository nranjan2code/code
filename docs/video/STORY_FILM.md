# Story film direction

## Creative premise

The first video is a reference explainer: it teaches the architecture layer by layer. This second
asset is the film: one request is the protagonist, and the architecture is revealed only when the
request needs it. The emotional movement is uncertainty → trust → agency → proof → confidence.

## Visual language

- Dark “Auditor’s Desk” palette from the VAK product design system.
- Large, quiet typography for the human idea; tutor diagrams appear as evidence, not slide backgrounds.
- Slow parallax/push-in on each diagram, with the important claim in the foreground.
- Burnt terracotta marks the active story beat; green, yellow, and blue retain their VAK state meanings.
- A restrained film-grain layer gives the image a physical texture without using 3D or sci-fi effects.
- Progress rail makes the request’s journey legible even with audio muted.

## Beat map

The 90-second cut uses 18 five-second beats: hook, 15 tutor diagrams, handoff, and closing title.
Every tutor PNG is used exactly once through `video/vak-story/src/assetManifest.ts`.

## Audio direction

Use a warm, close-mic narrator with deliberate pauses after each claim. Music should be a restrained
low-frequency bed that supports the reveal without competing with technical words. Captions should
be generated from the final audio rather than manually guessed, so timing follows the delivered
voice performance.

## Editing rules

Let the claim land before the diagram moves. Cut on conceptual transitions: entry → trust → action →
durability → evidence → delivery. Never fill a quiet moment with random motion. If a diagram is too
dense, crop to its relevant region or hold it as a complete map for orientation.
