---
name: social-youtube
description: Plan compliant YouTube research through the official Data API only; clearly report current connector availability.
serves: live-data
---

# YouTube add-on

Vakyartha currently supports a human-only YouTube API search preview in Settings. It is not an agent tool: never claim the agent can access its results. The owner may add a Google Cloud YouTube Data API key through the masked secure-key control, then search up to 10 results per request. Do not scrape pages, automate a browser, download video/audio, or use unofficial endpoints. Ordinary web search may be used separately when appropriate, but identify it as web search rather than YouTube API search.

A Google account or YouTube Premium subscription does not itself provide a developer API key or extra API quota. API projects have endpoint-specific quotas; `search.list` is particularly limited and each Vakyartha search consumes quota. YouTube data is subject to refresh and deletion requirements. Preview results are shown only in the Settings screen and Vakyartha does not save them or send them to an agent. Do not copy them into agent history or promise they have been refreshed after the preview closes.

Never ask for a password or API key in chat. The Settings key control stores the API key in Vakyartha's secure credential backend and never shows it again. Search remains owner-only, bounded, source-linked, and transient.

When helping the owner use the Settings preview, ask for a focused topic and a result cap of 1–10; treat title, channel, publish date, and URL as metadata only, never as watched or verified video content. A research brief should state that it is based on selected search metadata. A source review should list the video URL and channel/date context. A takeaway board can summarize only material the user separately supplies or explicitly reviews; do not infer video claims from titles. The current preview is not available to the agent and its results must not be copied into model context.
