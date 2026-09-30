# Mail and calendar package

This is a native, skills-only Vakyartha plugin package. It helps an Agent
explain account setup and the current availability boundary. It does not add
provider access, commands, MCP servers, network access, or executable code.

From the repository root, inspect and install it with the existing plugin
lifecycle:

```sh
vak plugin inspect packages/mail-calendar
vak plugin install packages/mail-calendar --scope user
```

Installation does not enable the package or grant a permission. Review and
enable it through Settings → Capabilities if the person wants the account
setup guidance in their Agent. Account linking and cleanup remain in Settings
→ Email and calendar.

The owner can preview bounded Gmail and Microsoft inbox, event, and free/busy
results in Settings. Apple remains unverified and unavailable. Agents cannot
read provider content yet; the brokered Agent tools, working area, local
drafts, Review, provider effects, scheduled work, and continuous routines are
still being built against current 4.x storage. Before Agent content access is
enabled, the product must explain that disconnect removes the saved credential
and blocks future reads, while content recorded in append-only Agent session
history cannot currently be erased. This package does not claim
account-content crypto-shredding; that requires future data-architecture
lifecycle work.
