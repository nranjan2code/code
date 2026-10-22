# Channel card rendering — installed investigation, 2026-10-22

Status: historical: read-only investigation of the installed 8.22.0 build and
its Telegram conversation, with source inspection for Slack and Discord.
No delivery implementation or installed configuration changed.

## Evidence and conclusion

The installed CLI resolves to `/Applications/Vakyartha.app/Contents/MacOS/vak`
and reports `8.22.0 (aa9822caa)`. Using `vak -C ~/vak-home data grep` and
`data cat` through the supported catalog/session readers located the screenshot's
weather and market turns. Both emitted `emit_summary_card` and
`emit_sources_card`; the market turn ends with successful card calls and no
ordinary assistant answer. This is a rendering gap, not missing card tools.
Private conversation identifiers and provider credentials are omitted here.

`crates/vak-delivery/src/telegram.rs::structured_card_chunks` converts each
card with `structured_markdown`, joins them with Markdown rules, then converts
the result to HTML. The bridge posts `sendMessage` with HTML parsing. Thus the
screenshot is formatted text derived from cards. Summary points and source
findings repeat the quotes; source label and title become the doubled heading
visible in the screenshot. Summary labels (including observation times) are
not rendered by the summary text projection.

`crates/vak-delivery/src/slack.rs::structured_card_blocks` uses native Block
Kit but iterates payload keys, stringifies nested arrays/objects, and exposes
field names. It does not draw the current forms by their block type.
`crates/vak-delivery/src/discord.rs::structured_card_embeds` uses native embeds
but includes only scalar payload fields when any are available; nested points
or sources are then omitted rather than added to a description. Its fallback
title is the semantic type when the payload has no title, as summary cards do.
These two conclusions are source inspection, not live Slack/Discord acceptance.
The inspected delivery projectors and Telegram bridge have no diff from the
installed revision. Existing bridge samples use legacy title/temperature/status
payloads, not the summary/sources forms that produced this real answer.

## Current provider contract

Telegram's current [Bot API](https://core.telegram.org/bots/api#sendrichmessage)
documents `sendRichMessage` and `InputRichMessage.blocks`, including section
headings, tables, expandable details, footers and buttons. Vak contains no
`sendRichMessage` implementation. The older no-heading/no-table assumptions in
docs/design/30-render-architecture.md describe its current basic-message path,
not the current provider's full capabilities. Client compatibility and actual
rich-message delivery have not been qualified by this investigation.

Slack [Block Kit](https://docs.slack.dev/block-kit/) and Discord
[embeds](https://docs.discord.com/developers/resources/message#embed-object)
provide native structure; Vak already uses these transports, with the generic
field rendering limitations described above.

## Follow-through

Render the existing typed forms directly for each surface, retaining provenance
and answer order, using rich Telegram messages with a tested basic-message
fallback, native Slack blocks, and Discord embeds. Preserve nested content and
account for every overflow instead of truncating or omitting it. Keep source
links concise in the main view with full findings available as details. Qualify
real summary/sources plus nested, long, escaped and multi-message examples in
the running channels. Do not infer a new card kind from a topic such as weather
or markets. This investigation authorizes no outbound test messages.
