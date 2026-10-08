# Acceptance: M7a lifecycle, honest deletion (2026-10-08)

Status: record of one acceptance run. Scope: data-architecture plan M7a,
against `docs/design/74-lifecycle-and-data-administration.md` §9.

## How it was run

- A dev build (`cargo build -p vak -p vak-server --bins`) served a
  disposable workspace under `/tmp` on port 8939, with its own throwaway
  data home (`VAK_HOME`), `[lifecycle] mode = "commit"`, and the local
  Ollama model `gemma4:e2b-mlx`. No paid provider was called, and the
  operator's real data home was never opened or acted on (plan M7a
  design, decision 8).
- Signed in with a throwaway gateway token. Screens were read in the
  in-app browser at 1440 × 900; the client's Trash was also read at
  390 × 844 in the light theme.
- The data home and the backup were deleted afterwards.

## Scenarios

| Scenario (doc 74 §9) | How it was checked | Result |
|---|---|---|
| A person trashes, restores, then deletes permanently, and search never shows it again | Two conversations made by real turns. Search for a word only one of them holds: 1 hit; in the trash: 0; restored: 1; trashed and erased with its title typed: 0, and still 0 after a restore from backup. The other conversation's word: 1 throughout | Pass |
| A hold placed before erasure blocks it | `PUT /conversations/{id}/hold`, then erase: refused with `held`; released, then erased. Also from the admin Trash panel: On hold, Erase disabled | Pass |
| A restore re-applies erasures | Backup taken (40 files, manifest version 2, no erasures), conversation erased, backup restored: preview named 1 erasure, the report said 1 erasure re-applied and 1 key removed, `/conversations/{id}/gone` still answered with a verifying receipt, search still had 0 hits | Pass |
| After a restore the server is fenced | `/health` said `fenced`; a new session was refused with the restart message; the Integrity report said `fenced` | Pass |
| An Agent revoke cuts its endpoints at once while its data stays | A test Agent with a bot and a placeholder token: the Revoke sheet showed 1 bot token, the name was typed, the token was gone when the request answered, the state read `revoked`, Resume answered 409 | Pass |
| The owner erases a guest from a shared conversation | Not run live: a throwaway home has no guest. Covered by `guest_erasure_keeps_owner_conversation` and `a_guests_contributions_are_erased_and_the_conversation_stays` | Tests only |
| Erasing a conversation keeps a Saved artifact version and removes unsaved drafts | Not run live: nothing in the home made an artifact. Covered by `erasure_follows_lineage` | Tests only |
| A paused schedule records every skipped slot | Shipped and tested at M4.4 (`skipped_slot_is_a_record`); not repeated here | Earlier milestone |
| A steward erases a person across chats and Agents; label shortening previews its impact | Not in M7a: person scope, roles and labels are M7b. Vak has one owner | Out of scope |

## Screens

| Screen | Seen with real data | Result |
|---|---|---|
| Client › Settings › Archived tasks: Trash with days left, Restore, Delete for good with the title typed | Yes, 1440 × 900 dark and 390 × 844 light | Pass. Found and fixed during the run: the text to type was the start of the conversation's id, not its title |
| Client › conversation menu: Archive conversation, Move to trash | Yes | Pass |
| Client › a link to an erased conversation says when and why it is gone | Yes | Pass |
| Client › agent picker: Revoke sheet, Revoked badge and filter | Yes | Pass. After the run the sheet stopped listing zero counts and the picker refreshes the sidebar; those two changes were not seen again |
| Admin › Conversations: Trash panel, holds, Erase, receipts | Yes | Pass |
| Admin › Conversation detail › Lifecycle | Yes | Pass |
| Admin › Operate › Data › Integrity | Yes: 2 conversations and 7 other record logs, 71 records, nothing damaged, 1 key destroyed, 1 receipt verified, search up to date | Pass |
| Admin › Home › Data health | Yes: size and file count, retention acting, links | Pass |
| Client › Library draft expiry; Settings › Delete saved copies; the restore confirmation; admin guests list | No: nothing in the throwaway home made a draft, a connected account or a guest | Tests only |

## Residual limits

- The scenarios marked "Tests only" have no browser evidence.
- No test asserts that no screen renders a value without an API source
  (doc 74 §9's per-screen test); each number on the new screens was
  compared by hand with the route it reads.
- The soak (`thirty_day_soak_stays_within_budget`) simulates thirty days
  by setting file times and the pass's clock; it is not thirty days of
  real use.
