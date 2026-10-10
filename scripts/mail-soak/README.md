# The mail and calendar soak

The 24-hour service test the mail and calendar plan asks for (Stage 4's
exit): restarts, sleep and wake, network and provider outages, expired
tokens and cursors, duplicate triggers, queue limits and pause during a
run, with no duplicate run of a slot, nothing left running, and stored
data that still verifies.

One Docker container runs a `vak` built with `vak-server/test-support`,
which sends every provider call to the loopback origin in
`VAK_TEST_PROVIDER_BASE` (`crates/vak-mail-calendar/src/endpoints.rs`).
`fake.py` is that origin: a stand-in for Google's OAuth, Gmail and
Calendar that speaks only the requests the adapters make, signs id tokens
with its own key, keeps its state across a stop, and takes faults from the
soak. No real account, provider or paid model is used: turns go to the
host's Ollama (`gemma4:e2b-mlx`), and every client id and token is a
throwaway value.

```
sh scripts/mail-soak/build.sh          # from the repository root
cd scripts/mail-soak
sh up.sh
python3 soak.py --hours 1              # a short run first
python3 soak.py --hours 24 --report report-24h.json
sh down.sh
```

`soak.py` links an account through the real OAuth flow, makes a mail
watch (every minute), a digest (every 15 minutes) and a calendar-event
routine (every 2 minutes, 10 minutes before each meeting), and feeds the
fake new mail and meetings. Every 10 to 20 minutes it injects one fault,
drawn from a shuffled round of all of them:

| Fault | What it does |
|---|---|
| restart | the server killed (`-9`) or stopped (`-TERM`), then started |
| sleep | the whole container paused 5 to 15 minutes |
| error500, error429, hang | the provider fails, rate-limits or stops answering for 3 to 8 minutes |
| network | the provider unreachable for 3 to 10 minutes |
| tokens | every access token rejected |
| revoke | the refresh token revoked; the account is linked again |
| history | the mail history expired, so the watch's cursor resets |
| duplicate | a second server on the same data home for 5 to 10 minutes |
| pause | the watch paused while it runs, then resumed |
| burst | 150 meetings at once, past the routine's queue ceiling |

Every half hour, and at the end, it checks: `vak data verify` passes, the
server is healthy, each routine has at most one run per slot and none left
running past 30 minutes, and the stand-in answers. `report.json` has every
fault, every check, the run counts by status and the data home's size over
time.
