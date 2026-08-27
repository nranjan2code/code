# 13 — Runtime HTTP/SSE adapter

`vak-server` is a stateless authenticated adapter over `vak-runtime`. It owns
HTTP parsing, authentication, response encoding, SSE fan-out, and embedded
admin assets. It does not own sessions, runs, configuration, or filesystem
state.

## Runtime endpoints

| Verb | Route | Runtime operation |
|---|---|---|
| GET | `/health`, `/version` | liveness and protocol handshake |
| GET/POST | `/projects` | project discovery and registration |
| GET/POST | `/sessions` | session catalog and creation |
| GET | `/sessions/:id/transcript` | authoritative transcript projection |
| POST | `/runs` | run admission and prompt submission |
| POST | `/runs/:id/cancel` | cancellation by live run ID |
| GET | `/events?run_id=:id` | ordered delta/snapshot SSE |
| GET/POST | `/approvals` | pending gates and verdicts |
| GET/PATCH | `/config` | effective config and revision-checked updates |
| GET/POST/PATCH | `/memory`, `/tasks`, `/skills` | Runtime CRUD |
| GET/POST | `/checkpoints`, `/backup` | checkpoint and backup operations |
| GET/POST | `/flows`, `/eval` | validated flow and deterministic evaluation |

Every mutating route is authenticated, validated, authorized, and audited by
Runtime. HTTP errors are typed responses; a route never silently edits local
client state.

## Events

The run stream carries each delta together with its complete snapshot and ends
with exactly one terminal outcome. A disconnect does not cancel the run. The
client reconnects and asks Runtime for the authoritative snapshot/transcript.

## Gateway

`serve --gateway` additionally owns the singleton lock and live gateway
receipt. Gateway duties remain in the same Runtime process so there is one
admission, permission, credential, and audit path.
