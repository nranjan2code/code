# Acceptance: M7b, the owner's control of what is kept (2026-10-09)

Status: record of one acceptance run. Scope: data-architecture plan M7b
as cut for one owner (plan §M7b "M7b design"), against
`docs/design/74-lifecycle-and-data-administration.md` §9.

## How it was run

- A dev build (`cargo build -p vak -p vak-server --bins`) served a
  disposable workspace under `/tmp` on port 8939, with its own throwaway
  data home (`VAK_HOME`), `[lifecycle] mode = "commit"`, and the local
  Ollama model `gemma4:e2b-mlx`. No paid provider was called, and the
  operator's real data home was never opened or acted on (plan M7a
  design, decision 8).
- Signed in with a throwaway gateway token. Actions were taken through
  the HTTP API and the screens in the in-app browser. A native prompt or
  confirmation cannot be typed into there, so the page's `prompt` and
  `confirm` were answered by script; the text each one showed was read
  back and is what the table quotes.
- The data home was deleted afterwards.

## Scenarios

| Scenario | How it was checked | Result |
|---|---|---|
| A shorter keep time previews what it removes and asks first | Admin › Data › Retention: the trash time set from 30 to 10 days. The confirmation named the kind, said nothing was past the new time yet, and saved on yes | Pass. A home made minutes earlier holds nothing old enough to be newly due; that half is `shortened_retention_previews_what_it_removes` |
| A hold blocks every erasure that would reach it | A conversation put on hold: listed by `/data/holds` and on the admin On hold list with its name; its own erasure preview said held; erasing everything was refused with `held`. Released, then erased | Pass. Found and fixed: erasing everything said "this conversation is on hold"; it now says something is on hold |
| An Agent's data is erased and nothing else | A test Agent given a conversation by a real turn, revoked, previewed (1 conversation), refused on a wrong name, erased with its name typed. The receipt said 1 conversation and 1 key; the two other conversations still answered search; the preview then offered nothing | Pass, after a fix (below) |
| A project's data is erased and its folder is left | A second folder given a conversation by a real turn (`vak exec`). Erased from Admin › Projects with its name typed. Its file and settings were unchanged, and the preview then offered nothing | Pass |
| One person is erased across chats | Not run live: the throwaway server runs without the gateway, so it has no channel sender. The Erase a person panel was read, and a made-up id answered "Nothing from that id is kept." Covered by `person_erasure_spans_agents_and_chats`, which runs real turns through the gateway | Tests only |
| Rotation keeps everything readable | Rotated from Admin › Data › Keys, from the client's Your data, and over HTTP (8 keys wrapped again). Afterwards a conversation read back, search answered, an erased conversation stayed gone, and the integrity report found nothing damaged with every receipt verifying | Pass |
| Erasing everything leaves a receipt and nothing else | With a real turn made, a key rotation done and an admin page open: 34 files before, the 1 receipt after, the server stopped. Run twice on one home: both receipts were kept. After a restart there were no conversations and the receipt was listed and verified | Pass, after a fix (below) |

## Found and fixed during the run

1. **An erased conversation was still served.** After an Agent's data was
   erased, `GET /sessions/{id}/transcript` still returned its text: the
   server had the conversation open, and an open conversation is held
   readable in memory. Trashing drops that handle; the Agent, project and
   person erasures did not. They now end by dropping the handle of every
   conversation that is hidden (`AppState::forget_hidden_sessions`).
2. **The same for a guest's words and an account's data**, where the
   conversation stays: an open conversation kept showing what was erased
   until the server restarted. A guest erasure now drops that
   conversation's handle and an account erasure every idle one, so each
   is read again from its records; both are refused while a turn that
   could hold the data is running.
3. **Files reappeared after erasing everything.** In the two seconds
   before the server stopped, background work wrote a search cache file
   and an empty credential key file. The process is now fenced the moment
   the erasure ends (`vak_session::fence::retire`) and clears again just
   before it exits (`Core::sweep_erased_install`).
4. **A second erasure of everything removed the first one's receipt.**
   The receipts directory is now never among what is cleared.

Each has a test: the transcript of an erased Agent's conversation is 404
(`revoke_cuts_endpoints_within_one_tick`, which fails without the fix),
an attached conversation stops showing a guest's words
(`a_guests_contributions_are_erased_and_the_conversation_stays`, which
failed before it), and stray files are cleared while the owner's folder
stays (`install_erasure_leaves_a_receipt_and_nothing_else`).

## Screens

| Screen | Seen with real data | Result |
|---|---|---|
| Admin › Data › Retention: the keep-time editor and its confirmation | Yes | Pass |
| Admin › Data › Retention: Erase everything, its counts, the receipt and Save the receipt | Yes | Pass |
| Admin › Data › Keys: where the key is kept, the key in use, the rotation history | Yes | Pass |
| Admin › Conversations: On hold list with Release hold | Yes | Pass |
| Admin › Conversations: Erase a person | Yes, with an id nothing is kept for | Pass for the panel; the erasure itself is tests only |
| Admin › Conversations: receipts for an Agent's and a project's data | Yes, each verified | Pass |
| Admin › Projects: Erase its data | Yes | Pass |
| Client › Settings › Your data: counts, keep times, Change the key, Erase everything | Yes, 1440 × 900 dark | Pass |
| Client › agent picker: Delete everything it holds; Library: Hold on a file | No: the Agent erasure was run over HTTP, and nothing in the home made a Library file | Tests only |

## Residual limits

- The rows marked "Tests only" have no browser evidence.
- Your data was not read at 390 × 844 or in the light theme.
- Retention's effect on old data was not seen live; a throwaway home has
  none. The thirty-day soak simulates it.
- A turn that is running when a guest or an account is erased is refused
  rather than interrupted; nothing checks a worker of another process.
- An Agent's conversation is not found by `/search` from the serving
  workspace, so "search no longer finds it" was checked for the other
  conversations only.
