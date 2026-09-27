# Agent character system

Status: In progress — eight built-in expression atlases and runtime states shipped, with sized WebP copies since V4.2 of the visual refresh; an optional Dimensional pack adds alternate idle portraits and 3D face glyphs. Flat two-colour glyphs for 32 px and below and installed user packs are later slices.

## Purpose

Characters make long-lived Agents recognizable without making the product feel mechanical or demanding attention. They never communicate capability, trust, or permission. Those remain explicit interface state.

## Canonical Vak mascot

Vak is the indigo and saffron songbird. Its three-feather wave crest represents voice, listening, and ideas moving between people. This is the only character used as the product mascot in onboarding, empty states, marketing, and video. Its canonical id is `vak`; do not substitute another companion for brand use.

The mascot must remain legible as a 24 px portrait, keep its crest silhouette, indigo body, saffron throat, cream face, and calm expression. Marketing poses may change gesture and camera angle while preserving those anchors.

## Packaged companions

The product includes seven optional Agent companions: Mira (fox), Moss (forest friend), Nori (silver seal), Pip (navy penguin), Lumi (cloud friend), Tavi (tawny owl), and Beni (cream rabbit). Their palette and silhouette remain distinct at sidebar size.

Each runtime character provides:

- a stable id, display name, form, personality note, portrait, hue, and two-note interaction cue;
- a transparent 4×2 expression atlas with blink, attention, thought, work, wait, delight, and acknowledgement poses;
- no autoplay sound; cues play only after direct interaction and obey the Sound cues setting;
- a still presentation when either the OS or Vak requests reduced motion;
- identical identity in conversation, coworking, sidebar, settings, and Agent creation because the character id is frozen with the Agent identity.

Source portraits and expression atlases live in `docs/brand/characters/`: transparent square PNG portraits, 512×512, framed to remain readable when cropped into the runtime container, and transparent 4×2 PNG atlases, 1024×512. Each frame must remain legible at 24 px. The client never loads the sources. `scripts/brand/generate.mjs` writes WebP copies of each portrait and atlas with 64, 128 and 256 px frames into `crates/vak-client-ui/public/characters/` (`<id>-<size>.webp`, `<id>-atlas-<size>.webp`), and `AgentMark` loads the smallest copy that covers its size at the screen's pixel ratio (`characterTier`). Change a source, then run the generator and its `--check`. Runtime metadata lives in `crates/vak-client-ui/src/agentGlyph.ts`; server validation lives in `crates/vak-server/src/agents.rs`. Additions must update both typed registries, the generator's character list and the render harness.

## Animation contract

`AgentMark` selects atlas poses for `idle`, `listening`, `thinking`, `working`, `waiting`, `success`, `concern`, and `acknowledge`. Blink, work, and wait use brief pose changes; pointer interaction triggers a one-shot acknowledgement. Running work, voice capture, and approval waits supply their state from the relevant UI. An atlas load failure shows the still portrait. Motion is paused outside the viewport and disabled by OS or Vak reduced-motion settings. Motion never blocks interaction or carries information by itself. The individual poses are expression frames, not a full-body video sequence; a later rig may provide more natural gestures through this same state API.

Animated GIF is not the package format: it cannot follow reduced motion, state, theme, or interaction reliably and consumes resources while hidden. A future vector rig belongs in a versioned character manifest and is driven by the same runtime state machine.

## User supplied character packs

User supplied packs require an installed local pack boundary. Arbitrary remote image URLs and inline data are rejected because they create tracking, CSP, availability, and identity drift. The pack contract will use a directory under the Shared Vak data home with a versioned `character.json`, local assets, declared motion states, and optional local sound cues. Installation validates id uniqueness, MIME type, dimensions, decoded size, duration, and paths before publishing the character through the live capability registry. Removal revokes new selection immediately while frozen conversations keep their recorded id and show an explicit unavailable placeholder.

The intended manifest fields are `schema_version`, `id`, `name`, `kind`, `personality`, `portrait`, `palette`, `motion`, `sounds`, `author`, and `license`. This section defines the expansion boundary; it does not claim the pack installer is shipped yet.

## Review checklist

New characters need a distinct silhouette and palette, a calm adult-friendly expression, transparent framing at 24/40/64/128 px, light/dark/contrast theme checks, reduced-motion behavior, keyboard labels, local-only sound, license metadata, and screenshots in the presentation harness.
