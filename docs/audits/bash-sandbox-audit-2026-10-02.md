# Bash and sandbox security audit — 2026-10-02

## Verdict

The broker architecture is in place, but restricted Bash is not a reliable
security boundary yet. I found one critical Linux filesystem-policy gap, and
multiple high-severity containment and authority escapes. These include a
write outside the workspace from the host-side file worker, commands continuing
after cancellation and timeout, and a glob allow rule authorizing hidden shell
substitution. The gaps affect promises in `AGENTS.md` and `docs/design/24-agent-security.md`.

Audited source at HEAD `e46c41783259c729b50e7877b8c24403c0d9353b`, plus the
working tree. I did not change runtime source. The report, probe, and its
reproduction script are the audit artifacts added here. Existing
working-tree changes were present and left intact.

## Findings

### P0 — Linux ReadOnly Landlock does not mediate `truncate(2)`

`Landlock::apply` builds the filesystem policy with `AccessFs::from_all(ABI::V1)`
([landlock.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-sandbox/src/landlock.rs:229)).
This policy handles the v1 filesystem rights but omits
`LANDLOCK_ACCESS_FS_TRUNCATE`, added in ABI v3. Unhandled operations are outside
the Landlock policy. In ReadOnly mode there is consequently no Landlock rule
denying `truncate(2)` on an existing file outside the workspace. A same-user
command can truncate files it owns, such as a key, config, or source file,
despite ReadOnly mode. The same omission leaves out-of-root truncate operations
uncontrolled in WorkspaceWrite mode.

This is confirmed against a real Linux kernel in a disposable Docker
container: I applied a deny-by-default ruleset handling every ABI v1 filesystem
right, added no write grant, and `truncate(2)` still changed a synthetic
ungranted file from 24 bytes to zero. This matches the
[Linux Landlock documentation](https://docs.kernel.org/userspace-api/landlock.html),
which says truncation has a separate access right and can occur through
`truncate(2)` without `WRITE_FILE`. Linux is not the host OS, so this runtime
probe exercises the Linux kernel semantics and the exact v1 access mask, not
the compiled Rust runner end to end.

### P1 — Docker timeout/cancellation leaves command processes running

The broker starts its worker in a process group and kills that group when
cancelled ([broker.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-tools/src/broker.rs:390)).
Bash starts another process group for its shell ([bash.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-tools/src/bash.rs:132)); killing the
worker group does not kill that shell group. In Docker modes, the shell starts
`docker run` or `docker exec`, which starts the workload in the daemon-owned
container. Killing the local client does not stop the container command.

Live probes against both backends returned “cancelled” or “timed out” at about
0.5 or 1.0 seconds; in each case the command then wrote a file into the
workspace after the tool had returned. The retained task container was also
left executing after `docker exec` cancellation. This can defeat a permission
mode revocation and let old work race with the next run. The host can report
that work has stopped while it is still changing files.

### P1 — Docker Bash ignores its requested working directory

For command-scoped backends, the broker wraps only the Bash string before it
reaches the worker ([broker.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-tools/src/broker.rs:1536)).
The worker does not receive the sandbox, so `BashTool` cannot inject its
validated `cwd` into that command. Docker’s wrapper pins `-w` to the workspace
root ([docker.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-sandbox/src/docker.rs:275)).
A live broker call with `cwd: "subfolder"` ran `pwd` at the workspace root.
This breaks shell/file-tool path agreement and may run project commands against
the wrong files.

### P1 — A Docker-backed file write escaped the workspace

Docker contains Bash, while ordinary file tools continue to execute in the
host worker. `write` validates only the requested destination, then writes
through a predictable sibling path before renaming it
([write.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-tools/src/write.rs:143)).
I used brokered Docker Bash to create `allowed.txt.vak-tmp` as a symlink to a
synthetic file under the host home directory. The authorized `write` to
`allowed.txt` followed that link and overwrote the host file; the permission
engine allowed the requested in-workspace target. This confirms the
command-scoped Docker policy does not contain file-tool effects. The symlink
approach also highlights the no-follow and check-to-use gap already called out
in the security design.

### P1 — A command can continue after Bash reports success

Bash does not wait for or reap background jobs once the shell exits. A direct
`sleep 2 &` with a 1-second command timeout returned **success** after about 2.0
seconds because the background process kept captured output pipes open. A
detached child whose output was redirected let Bash return success in 12 ms;
it then wrote a workspace file two seconds later. Those commands retain the
worker’s authority after the call is complete and can race with subsequent
tools. The timeout only supervises the shell’s exit, not the execution group’s
lifetime.

### P1 — Linux Landlock does not deny UDP network access

The runner handles network rights from `ABI::V4`
([landlock.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-sandbox/src/landlock.rs:229)).
Landlock v4 covers TCP bind/connect; UDP restrictions were added in ABI v10.
No network namespace or firewall wraps the native Linux runner, so a
restricted Bash process can use UDP for outbound traffic, including DNS-style
exfiltration. The current implementation therefore does not meet its stated
“no network” contract on Linux. This is a source finding cross-checked against
the [Linux Landlock ABI documentation](https://docs.kernel.org/userspace-api/landlock.html);
I did not test UDP egress from the host Linux runner.

### P1 — macOS workspace-write Bash can read and change sibling temp files

Native Seatbelt grants read and write access to shared OS temp roots to keep
developer tools working ([backend.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-sandbox/src/backend.rs:135)).
The grant covers the entire `/private/var/folders`, `/private/var/tmp`, and
`/private/tmp` trees, rather than the current execution’s temp directory. A
real workspace-write Seatbelt run read a synthetic sibling temp file and
overwrote another. This crosses between simultaneous workspaces running under
the same account. Linux’s native backend similarly adds global temp roots to
its read/write allowances.

### P1 — Shell allow rules miss quoted command substitution

The permission parser intends to reject opaque command substitutions before
an allow pattern can approve them ([rules.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-permission/src/rules.rs:194)).
However, it stops scanning for shell syntax while inside double quotes, where
`$(...)` and backtick substitutions still execute. The tested rule
`+Bash(printf *)` classified
`printf '%s' "$(touch hidden-effect)"` as `Allow`; running it through the
Seatbelt Bash executor created `hidden-effect`. A similarly quoted backtick
form was also classified `Allow`. This bypasses pattern-scoped approval and
the “opaque commands cannot be learned or allow-covered” rule.

### P1 — The control-file guard is a string check, not a boundary

`BashTool` rejects only commands containing literal `.env`, `.vak/config.toml`,
or `.vak/config`, and only when an execution-event sink exists
([bash.rs](/Users/nisheethranjan/Projects/Vak/code/crates/vak-tools/src/bash.rs:53)).
Under Seatbelt, `cat .e?v` successfully read a synthetic `.env`. The same
command changed `.vak/permissions.local.toml`. The ordinary `write` tool has no
control-file refusal either, although WorkspaceWrite permits in-workspace
files. This contradicts the stated boundary that the worker cannot edit policy
or configuration; learned grants and trusted settings must remain broker-owned.

### P1 — Native Seatbelt and Landlock do not enforce process, memory, CPU, or disk ceilings

The native backends restrict filesystem/network operations but set no process
count, CPU, memory, or disk quota. A fork bomb or a large workspace/temp write
can exhaust the user’s machine or shared filesystem before the command
timeout. The output capture cap does not bound filesystem growth, and the
model-supplied timeout has no maximum. Docker does configure memory, CPU,
process, and tmpfs limits, but its writable container root has no size cap in
the one-shot backend. This conflicts with `docs/design/24-agent-security.md`
§Required invariants, which requires ceilings for process count, CPU, memory,
disk, output, and time.

## Verification and limits

- `cargo test -p vak-tools --test sandbox --test sandbox_stress -p vak-permission --tests` passed: 214 tests; 2 network-dependent tests were ignored. This validates existing coverage, not the gaps above.
- `docs/research/bash-sandbox-audit-2026-10-02/run.sh --docker` runs the real-worker repros; omit `--docker` to skip the daemon checks. The probe confirmed the allow-rule execution, control-file bypass, sibling temp access, worker-cancellation leak, background-process lifetime, Docker temporary-symlink overwrite, Docker `cwd` loss, one-shot Docker timeout leak, retained-task cancellation leak, and successful Docker task creation.
- A separate disposable Amazon Linux container confirmed the Landlock ABI v1 `truncate(2)` gap. It had no network, dropped capabilities, `no-new-privileges`, and touched only a synthetic container file. Docker containers created by the broker probe were removed afterward.
- The host was macOS arm64. I did not run the Rust Landlock runner on host Linux, probe its UDP egress, test a true OS-level fork bomb, inspect real credentials, or validate every supported kernel/filesystem combination. No real user file was read or modified by the probes.

The key residual risks already listed in `docs/design/24-agent-security.md`
§P0 remaining filesystem and credential hardening are directly relevant here:
make policy/config immutable to workers, use descriptor-relative no-follow
operations, and test cancellation against descendants and every backend.
