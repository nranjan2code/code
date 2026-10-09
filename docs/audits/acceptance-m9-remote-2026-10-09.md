# Acceptance: M9, the remote and two machines taking turns (2026-10-09)

Status: record of the acceptance runs. Scope: data-architecture plan M9
as cut for one owner (plan §M9 "M9 design").

## How it was run

- Two Docker containers, "desk" and "away", each with its own data home,
  sharing one volume as the remote folder (`scripts/sync-lab`). The
  binaries were built for Linux from the working tree
  (`scripts/sync-lab/build.sh`) and each machine ran `vak serve` and the
  `vak` CLI as a person's two machines would.
- Turns were real, against the host's Ollama model `gemma4:e2b-mlx`. No
  paid provider was called. The operator's own data home was never
  opened: everything lived in the containers' volumes, which were
  removed afterwards.
- The lab was run three times end to end. The first two runs found the
  defects below; the third passed 45 of 45 checks.
- The admin Second copy screen and the client's Your data line were read
  in the in-app browser against a throwaway home on macOS.

## Scenarios (third run)

| Scenario | What was checked | Result |
|---|---|---|
| A first copy | A real turn on the desk; set up and first push over HTTP (16 files); the key file carried by hand; a pull on the away machine; the same conversation byte for byte on both; the away copy verifies | Pass |
| Standing by | On the away machine a turn is refused at once with the reason, a push is refused, and a take-over is refused before a hand-over | Pass |
| Automatic push | A finished turn is pushed with nobody asking, within the next scheduler ticks | Pass |
| Hand-over at a turn boundary | Hand-over is refused while a turn runs; once it ends the desk hands over and then refuses a turn itself | Pass |
| Fire and forget | An automation made on the desk; the desk handed over and its container stopped; the away machine took over, restarted, held the work, read everything the desk did, ran the automation unattended, did new work and handed back; the restarted desk took over and held the away machine's work and the automation's run records | Pass |
| A crash in the middle of a turn | `kill -9` on the server 2.5 seconds into a turn: nothing stored was damaged, the interrupted run was recorded abandoned, the desk worked again, its push caught up, and the away machine pulled the state | Pass |
| A push killed part-way | Ten pushes, each killed after a different delay (10 ms to 500 ms) following a key rotation: the away machine pulled a whole, readable, verified copy 10 of 10 times | Pass. The kill landed before the push finished in 2 of 10 (4 of 10 in each earlier run) |
| A pull killed part-way | Eight pulls, each killed after a different delay: the next pull left a whole, readable, verified copy 8 of 8 times | Pass. The kill landed in 2 of 8 (3 and 4 of 8 in the earlier runs) |
| Keys changed, key file not carried | The pull is refused with the reason and nothing on the puller changes; with the new key file it pulls | Pass |
| The remote unreachable | The desk still works; a push fails whole and says so; the status shows 14 files owed and why; when the folder is back the owed push goes through and the away machine gets what was written meanwhile | Pass |
| A lost machine | A plain take-over is refused; a forced one brings what was pushed and not what never was; the away machine works and pushes; the returning desk's push is refused and it is told 14 files were never pushed; it begins no turn; it will not drop that work unasked; told to, it pulls and stands by | Pass |
| Erasure across machines | A conversation erased on the away machine: the desk had it, pulled, and no longer has it; the signed receipt came too; no file in the remote mentions its text | Pass |
| Both take over at once | After a hand-over both machines ran take-over at the same moment and then both pushed: exactly one push was accepted and the other machine was told it does not hold the work | Pass |

## Found and fixed

1. **An interrupted push could leave the remote unusable.** Files were
   written in place while the old index still named them. The remote now
   keeps each file under its digest and never changes one; the index is
   replaced last and only then is anything removed.
2. **After a key rotation the other machine pulled a copy it could not
   read, silently.** The index now carries the key version; a pull is
   refused before it changes anything, and a key file made after the
   rotation is taken by a machine that already holds the earlier keys.
3. **A pull fenced itself half-way.** Importing the other machine's refs
   brought its newer writer epoch, which stopped the pulling process
   before it had applied erasures and rebuilt search. An import now keeps
   the puller's own epoch and moves it past both machines at the end.
4. **A machine holding only notes and settings could have its keys
   replaced.** "Holds nothing" counted conversation keys only; it now
   counts every stored object.
5. **`vak data cat` found no conversation** on a data home no server had
   run on. It reads the records in first.
6. **Forgetting the remote lost the machine's identity**, so setting the
   same folder up again locked it out of its own lease. Its id is kept.
7. **A standing-by machine was told it had unpushed work** (its own
   bookkeeping since the pull) and asked to discard it. A machine that
   stood by did no work, so it pulls without asking.
8. **A turn on a standing-by machine was accepted and failed later.** The
   run route refuses it at once with the reason.
9. **A project made on one machine had no way to be the same project in a
   different folder on the other.** `vak sync place` and Projects › Its
   folder here attach the folder to the existing project.

## Screens

| Screen | Seen with real data | Result |
|---|---|---|
| Admin › Data › Second copy: set the folder, Copy now, Hand over, Take over here, the facts, Make a key file | Yes, on a throwaway home | Pass. Found and fixed: after this machine handed over it said the other machine held the work; a missing full stop |
| Client › Settings › Your data: the Second copy line with Copy now | Yes | Pass |
| Admin › Projects: Its folder here | No | Tests only (`push_pull_roundtrip_identical_derive_messages`) |
| Admin: taking a key file; a forced take-over; the client's Take over here | No | The same routes were driven over HTTP and the CLI in the lab |

## Residual limits

- The remote is a folder. A folder kept in step by another tool (a cloud
  drive) has no atomic rename across machines: the lease is checked again
  just before a push counts, which narrows a race to moments and does not
  remove it. Two machines that both push within those moments could each
  believe it won; the lab's simultaneous take-over was run once per run
  (three times) and resolved correctly each time.
- Push and pull kills landed part-way in a minority of tries because both
  are quick on a small home; a large home was not tested.
- Both lab machines are Linux containers on one host. A macOS desk with a
  Linux remote, and a real network share, were not run.
- After a pull the server must be started again; nothing restarts it for
  a person outside a service manager.
- Handing over moves all the work. One conversation cannot move while
  others stay.
- Provider keys and bot tokens do not travel; the lab gave both machines
  the same local model by configuration.
