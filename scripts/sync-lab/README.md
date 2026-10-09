# The two-machine lab

Two Docker containers stand in for two of one owner's machines, "desk" and
"away". Each has its own data home; they share one volume, which is the
remote folder (data-architecture plan M9). The lab drives them with real
Linux binaries and real model turns and prints PASS or FAIL for each check.

It needs Docker, and Ollama on the host with the model `gemma4:e2b-mlx`
(change the model in `up.sh`). No paid provider is called and the
operator's own data home is never touched: everything lives in the
containers' volumes.

```
sh scripts/sync-lab/build.sh      # from the repository root; a few minutes
cd scripts/sync-lab
sh up.sh                          # two fresh machines
python3 lab.py                    # about twenty minutes
sh down.sh
```

What it covers:

1. A first copy: a real turn, a push, the key file carried by hand, a pull,
   and the same conversation read on both machines.
2. Standing by: the second machine begins no turn, pushes nothing and
   cannot take over before a hand-over.
3. The automatic push after a turn.
4. Fire and forget: an automation made on the desk, the desk handed over
   and switched off, the automation running on the away machine, and the
   desk taking the work back with everything that happened.
5. The server killed with `kill -9` in the middle of a turn.
6. A push killed at ten different moments.
7. A pull killed at eight different moments.
8. The key changed on one machine and the key file not carried.
9. The remote folder unreachable, then back.
10. A lost machine: a forced take-over, and the lost machine returning.
11. An erasure made on one machine reaching the other.
12. Both machines taking over at the same moment.

The passphrase and tokens in `up.sh` are throwaway test values.
