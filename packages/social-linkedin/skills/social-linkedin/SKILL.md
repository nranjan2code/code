---
name: social-linkedin
description: Explain LinkedIn API limits and its owner-visible OIDC profile connection without implying content access.
serves: live-data
---

# LinkedIn add-on

Vakyartha can connect an owner-visible LinkedIn profile name through OpenID Connect after the operator configures an app and LinkedIn enables native PKCE for it. This is a narrow account link, not identity verification. The requested scopes are `openid` and `profile`; the profile name remains in Settings and is not sent to the agent. A connection does not enable general post search, member-profile collection, or organization content access.

LinkedIn's documented APIs include member and organization actions, but access depends on the app's approved products, exact OAuth scopes, and (for organizations) the member's page role. Some read permissions are restricted to approved applications. Never present the existence of an endpoint as evidence that this app is approved to use it.

Never ask for a LinkedIn password, session cookie, client secret, or pasted token. The operator supplies only the public app Client ID in Settings. The local native PKCE flow uses a short-lived loopback callback and stores the access token through Vakyartha's Agent-scoped credential path. If PKCE is not enabled for the app, tell the operator to request it from LinkedIn; do not fall back to a client-secret flow. General feed and post search remain out of scope.

For a research brief, ask the user to provide or select the exact authorized organization posts and date range; summarize organization-level content without inferring personal traits. For source review, prioritize the canonical post URL, organization identity, date, and granted scope, and flag unavailable context. For a takeaway board, attach each concise claim to its post and distinguish the organization's statements from analysis. These formats do not imply LinkedIn feed, member, or profile search is available today.
