# M8 acceptance run: create, review, promote, share, comment, revise (2026-10-08)

Status: passed on `gpt-6-luna` after one client fix and three contract
fixes found by the run; create and review passed on local Ollama
(`gemma4:e2b-mlx`) on 5 of 8 create attempts. Erase is not part of this
run; it waits for M7a.

## Scope

The run is the exit test of plan M8 (`docs/plans/data-architecture-plan.md`,
"M8.4c design", step M8.4c-f). It used two disposable workspaces outside
the source checkout: `/tmp/vak-live/luna-ws` (provider `openai-responses`,
model `gpt-6-luna`, server on port 8937) and `/tmp/vak-live/workspace`
(provider `ollama`, model `gemma4:e2b-mlx`, server on port 8931), both a
dev build signed in with a throwaway token. Requests to the OpenAI
service went through a capture proxy that saves request bodies only.

The browser pane was not displayed during the run, so real clicks and
screenshots were not available. Every control named below as "clicked"
was activated in the running page by script (`element.click()`, or a
value set followed by an input event), and what the page then showed was
read back from the page. **Not done: any visual check, at 1440 × 900 or
390 × 844, light or dark.**

## The run on gpt-6-luna

| Step | What was done | Result |
|---|---|---|
| Create | Asked for `launch-brief.docx`, titled Launch brief, three short paragraphs | One `office_apply` call; a draft; nothing in the folder |
| Review | Clicked Review changes on the chat card | 10 change rows listed; Open in Library and Star present |
| Canvas | Clicked Open saved version in Canvas, then Comment to open its panel | The document's text shown; Discussion, Activity and Changes tabs |
| Open in Library, from the Canvas | Clicked it | The Library opened on the artifact, version 1 |
| Promote | Clicked Accept 1 selected file | "Accepted, version 1 · now in your folder"; the file exists |
| Undo | Clicked Undo acceptance in the Workbench | "Restored 1 file(s) to their pre-acceptance state"; the file is gone |
| Promote again | Review candidate, Open file review, Accept | Accepted; records are Candidate, Promotion, PromotionUndo, Candidate, Promotion; still one version (the same bytes proposed twice) |
| Invite | Invited a guest, Asha, to the conversation, allowed to comment | 2 bindings visible to her; document read 200; her comment 201 |
| Share | In the Library clicked Share, filled name Ravi, Can comment, From version 1 on, Create link | A link and a code shown once; the share is listed as Ravi · Can comment |
| Inheritance | Read as Asha again | Document 403, thread 403, 0 bindings: sharing by a link broke inheritance |
| Comment | Opened `?shared=artifact` with Ravi's code and commented | The page shows version 1 and one thread: Asha's comment, then Ravi's |
| Owner's view | Opened the artifact in the Library | Both comments, each "on version 1"; Starred |
| Revise | Clicked Continue working, sent "Revise it as the comments ask: …" | A tracked-change draft from `office_apply` in the Agent's own conversation |
| Review and promote the revision | Review changes, Accept | Version 2, made from version 1, accepted; the folder's file has the new paragraph as a tracked insertion |

Final state: two versions (`…eef83d`, then `…9c5801` with `…eef83d` as its
parent), both accepted; two comments on version 1; one active share; the
artifact starred.

## What the run found, and what was done

1. **Undo, Review candidate and the workspace checks were unreachable in
   the Workbench.** The Workbench decided that a run held a draft by
   looking for `.vak/scratch` in the run's directory. Drafts left the
   project tree at M3b, so every draft run was labelled "Written directly
   to the workspace" and the card was never drawn. It now tests whether
   the run's files are in the run's own directory. After the fix the card
   shows and Undo acceptance works (the Undo row above).
2. **Continue working did not tell the Agent what people had said.** The
   block the server writes for an attached artifact named its versions
   and not the comments on the current one. Asked to "revise it as the
   comments ask", the Agent called `recall` twice looking for them in the
   conversation that made the file (one call refused as invalid). The
   block now lists the comments on the current version. Measured after
   the fix, 5 turns: `doc_read` then `office_apply` in 5 of 5, `recall`
   in 0 of 5.
3. **A text tool's refusal of a document sent a new file to `doc_read`.**
   On Ollama, `write` to a `.docx` was answered "Read it with doc_read,
   then change it with office_apply". The file did not exist; the model
   read it, got "no such file", and stopped. The refusal now says how to
   create one as well as how to change one.
4. **An optional parameter sent as `null` was refused.** `"after": null`
   in an `add_paragraph` op was answered "must be string". An optional
   parameter sent as null is now treated as left out.

## Create and review on Ollama

The same create request, a new conversation each time:

| Build | Drafts made | Failures |
|---|---|---|
| Before fixes 3 and 4 | 2 of 6 (and the first attempt, 0 of 1) | 3 began with `"after": null` refused; 1 made no call |
| After | 5 of 8 | 2 put `path` and a `description` inside each op and left out `op`; 1 made no call |

Review on one of the drafts: Review changes listed the added paragraphs,
Open in Library and Star were present, and the Canvas showed the
document. The model wrote placeholders ("[Insert Launch Date Here]"),
since the request gave no content, and named the file `Launch brief.docx`
where the request said `launch-brief.docx`.

## Open

- **`office_apply`'s argument shape on the local model.** In 2 of 8 runs
  the model put `path` inside each op, with a `description`, and no `op`;
  in 1 of 7 earlier runs it wrote the op's schema as its value
  (`"op": {"enum": ["add_paragraph"], "type": "string"}`). The refusal
  names the missing fields and the model repeated the same call. What in
  the tool's schema leads there has not been measured.
- **One create run in each batch made no tool call and wrote nothing.**
  Not investigated.
- **`recall` with `turn_id` and a filler `query`** is refused as two
  targets. The model had filled every optional field of the call.
- **The Canvas names a draft by the draft that proposes it**, not by its
  artifact version (recorded at M8.4c-d as a decision).
- **After inheritance breaks, the shared-conversation page still lists
  the file**; opening it shows that it is not shared there.
- **No visual check** (see Scope).
