---
name: social-linkedin
description: Explain LinkedIn API access and use only explicitly approved LinkedIn capabilities.
serves: live-data
---

# LinkedIn add-on

LinkedIn sign-in and API access are currently unavailable in this Vakyartha build. Do not claim a LinkedIn account is connected, do not search public posts or profiles, and do not scrape the feed, automate a browser, or collect member profiles. LinkedIn Premium or Sales Navigator does not grant Vakyartha API permissions.

LinkedIn's documented APIs include member and organization actions, but access depends on the app's approved products, exact OAuth scopes, and (for organizations) the member's page role. Some read permissions are restricted to approved applications. Never present the existence of an endpoint as evidence that this app is approved to use it.

Never ask for a LinkedIn password, session cookie, or pasted token. If an approved OAuth flow is added later, show its exact scopes and allow only the actions those granted scopes support. General feed and profile search remain out of scope.

For a research brief, ask the user to provide or select the exact authorized organization posts and date range; summarize organization-level content without inferring personal traits. For source review, prioritize the canonical post URL, organization identity, date, and granted scope, and flag unavailable context. For a takeaway board, attach each concise claim to its post and distinguish the organization's statements from analysis. These formats do not imply LinkedIn feed, member, or profile search is available today.
