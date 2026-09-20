# Agent character system

Status: In progress — built-in package and runtime identity shipped; installed user packs are the next slice.

## Purpose

Characters make long-lived Agents recognizable without making the product feel mechanical or demanding attention. They never communicate capability, trust, or permission. Those remain explicit interface state.

## Canonical Vak mascot

Vak is the indigo and saffron songbird. Its three-feather wave crest represents voice, listening, and ideas moving between people. This is the only character used as the product mascot in onboarding, empty states, marketing, and video. Its canonical id is `vak`; do not substitute another companion for brand use.

The mascot must remain legible as a 24 px portrait, keep its crest silhouette, indigo body, saffron throat, cream face, and calm expression. Marketing poses may change gesture and camera angle while preserving those anchors.

## Packaged companions

The product includes seven optional Agent companions: Mira (fox), Moss (forest friend), Nori (silver seal), Pip (navy penguin), Lumi (cloud friend), Tavi (tawny owl), and Beni (cream rabbit). Their palette and silhouette remain distinct at sidebar size.

Each runtime character provides:

- a stable id, display name, form, personality note, portrait, hue, and two-note interaction cue;
- quiet breathing while idle and a clearer attentive reaction while working or selected;
- no autoplay sound; cues play only after direct interaction and obey the Sound cues setting;
- a still presentation when either the OS or Vak requests reduced motion;
- identical identity in conversation, coworking, sidebar, settings, and Agent creation because the character id is frozen with the Agent identity.

Source portraits live in `crates/vak-client-ui/public/characters/`. Runtime metadata lives in `crates/vak-client-ui/src/agentGlyph.ts`; server validation lives in `crates/vak-server/src/agents.rs`. Additions must update both typed registries and the render harness. Portraits are transparent square PNGs, 512×512, framed to remain readable when cropped into the runtime container.

## Animation contract

The shipped portrait is source art, not the animation. `AgentMark` supplies the runtime motion layer. Motion is state based (`idle`, `working`, and selected), small in amplitude, never blocks interaction, and never carries information by itself. Future expression sheets may add blink, acknowledge, celebrate, and concern states behind this same component API.

Animated GIF is not the package format: it cannot follow reduced motion, state, theme, or interaction reliably and consumes resources while hidden. A future expression sheet or vector rig belongs in a versioned character manifest and is driven by the same runtime state machine.

## User supplied character packs

User supplied packs require an installed local pack boundary. Arbitrary remote image URLs and inline data are rejected because they create tracking, CSP, availability, and identity drift. The pack contract will use a directory under the Shared Vak data home with a versioned `character.json`, local assets, declared motion states, and optional local sound cues. Installation validates id uniqueness, MIME type, dimensions, decoded size, duration, and paths before publishing the character through the live capability registry. Removal revokes new selection immediately while frozen conversations keep their recorded id and show an explicit unavailable placeholder.

The intended manifest fields are `schema_version`, `id`, `name`, `kind`, `personality`, `portrait`, `palette`, `motion`, `sounds`, `author`, and `license`. This section defines the expansion boundary; it does not claim the pack installer is shipped yet.

## Review checklist

New characters need a distinct silhouette and palette, a calm adult-friendly expression, transparent framing at 24/40/64/128 px, light/dark/contrast theme checks, reduced-motion behavior, keyboard labels, local-only sound, license metadata, and screenshots in the presentation harness.
