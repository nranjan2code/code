# Mail and calendar mock regression pack

Run the repeatable provider and integration regression pack from the repository
root:

```sh
./scripts/test-mail-calendar-mock-pack.sh
```

It uses Cargo offline mode and runs the `vak-mail-calendar` crate tests, Core
and server mail/calendar boundary tests, and isolated parser-worker tests. The
pack also explicitly runs the server restart/requeue test for an interrupted
continuous mail watch. Provider HTTP and IMAP tests bind loopback-only
ephemeral listeners and use temporary vaults. The tests do not connect to
Google, Microsoft, Apple, or any other external service. They do not read
credentials from the developer's home or data home.

## Generated stress fixtures

The large provider responses are generated in memory inside the tests; there
are no captured account exports or static personal-data fixture files.

| Provider path | Synthetic input | Expected bound checked |
| --- | ---: | --- |
| Gmail message listing and selected bodies | 5,000 listed messages; selected bodies exceed the projection budget | At most 20 returned/read messages and 16 KiB projected body text |
| Google Calendar | 1,000 events | At most 100 returned events |
| Microsoft Graph mail | 1,000 messages | At most 20 returned messages |
| Microsoft Graph calendar | 1,000 events | At most 100 returned events |
| Apple IMAP | 2,000-message mailbox | Fetch only newest 20 metadata records |

The surrounding suite also exercises provider-origin/page validation, account
capability and audience boundaries, encrypted Agent-local vault behavior,
credential isolation, effect commitment gates, and parser-worker isolation.
Add new provider regression cases to the relevant crate tests and update this
inventory when the synthetic input size or asserted bound changes.

This pack is regression coverage, not evidence of live-provider conformance or
the separate 24-hour service/recovery acceptance.
