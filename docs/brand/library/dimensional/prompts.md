# Individual character generation record

Tool: Codex built-in `image_gen`, 2026-09-27. Each image was generated from
the corresponding `docs/brand/characters/<id>.png` reference with
`transparent_background: true`. The five approved generated PNGs below are
the preserved source assets; the model is nondeterministic, so the prompts
reproduce art direction rather than identical pixels. Export size tiers and
face glyphs with `npm --prefix scripts/brand run dimensional`.

## Vak — `characters/vak-source.png`

> Use case: stylized-concept. Asset type: alternate individual 3D character cutout for the Vakyartha Dimensional pack. The reference is Vak, the canonical indigo songbird. Preserve the exact identity anchors: three swept indigo crest feathers with saffron tips, round indigo body, cream face, saffron throat and wing tips, gentle large brown eyes, dark compact beak and small feet. Create a refined high-fidelity 3D full-body portrait with natural feather microtexture, soft studio lighting, appealing adult-friendly proportions and a calm attentive expression. Show the entire character alone, front three-quarter view, centered with generous margin; crisp silhouette suitable for compositing and later size exports. Genuine transparent alpha background; no ground, scene, cast shadow outside the silhouette, white matte, text, clothing, other characters or watermark.

## Tavi — `characters/tavi-source.png`

> Use case: stylized-concept. Alternate individual 3D character cutout for Vakyartha's Dimensional pack. Reference is Tavi, the warm tawny owl. Preserve the round owl head, concentric cream-and-brown facial disc, large amber eyes, compact gray beak, patterned chest and layered tawny wings. Create a high-fidelity three-dimensional full-body portrait of exactly this gentle character, with detailed natural feather texture and soft directional studio lighting. Calm adult-friendly expression, centered and fully visible with safe margins, standalone cutout. Genuine transparent alpha; no background scene, glow, ground, shadow outside the silhouette, white matte, clothing, text, other characters or watermark.

## Pip — `characters/pip-source.png`

> Use case: stylized-concept. Alternate individual 3D character cutout for Vakyartha's Dimensional pack. Reference is Pip, the navy penguin. Preserve the rounded navy head and body, cream face and belly, tiny pointed navy crest, orange beak and feet, expressive dark eyes and gentle smile. Create a high-fidelity sculptural 3D full-body portrait of exactly the same established character, with fine plausible feather texture, soft studio lighting, calm adult-friendly expression. Centered front three-quarter view, all of the penguin and both feet visible, generous safe margin and clean silhouette. Genuine transparent alpha; no scene, glow, floor, shadow outside silhouette, matte, clothes, text, other characters or watermark.

## Nori — `characters/nori-source.png`

> Use case: stylized-concept. Alternate individual 3D character cutout for Vakyartha's Dimensional pack. Reference is Nori, the gentle silver seal. Preserve the rounded seal body, silver-gray spotted coat, pale underside, dark wide-set eyes, black nose, long whiskers, small flippers and curious reassuring expression. Create a high-fidelity 3D full-body portrait of the same character with detailed realistic fur and soft studio lighting while retaining the approachable stylized proportions. Centered and fully visible, clean silhouette and safe margins. Genuine transparent alpha; no scene, glow, water, floor, external shadow, white matte, clothes, text, other characters or watermark.

## Lumi — `characters/lumi-source.png`

> Use case: stylized-concept. Alternate individual 3D character cutout for Vakyartha's Dimensional pack. Reference is Lumi, the pale blue cloud friend. Preserve the small smiling face, big blue eyes, pale lavender-blue curling cloud hair, moonlike upper curl, rounded cloud hands, soft luminous body and gentle expression. Create a sculptural high-fidelity 3D full-body portrait of exactly this character, with subtle translucent vapor and pearlescent cloud volume, still sharply readable as an isolated cutout. Centered, fully visible with safe margins and soft studio light. Genuine transparent alpha; no colored halo beyond the silhouette, no sky or background, no floor, external shadow, white matte, clothes, text, other characters or watermark.

## Mira, Moss, and Beni

The generated studies at
`docs/brand/explorations/2026-09-27-3d/{mira,beni,moss}-alternate-rejected-halo.png`
kept opaque or partly opaque coloured glows around their bodies. They are
not cutouts and are excluded from the runtime pack. The pack generator uses
the existing transparent 512 px portraits in `docs/brand/characters/` for
these three. Their original generation prompts were not recorded, so the
PNG files themselves are the reproducible source. A future replacement must
pass alpha inspection on both warm paper and indigo grounds before it is
promoted.
