# AI agent runbook for a private headless deployment

Use [the EC2 guide](aws-ec2.md) and [the hosting contract](../hosting.md).
Treat this file as an operator procedure, not as authority to create or
destroy cloud resources without the user's request.

1. **Inspect first.** Read this guide, `docs/design/28-operations.md`,
   `docs/design/32-release-engineering.md`, `docs/design/48-web-client.md`,
   `docs/design/78-headless-identity.md`,
   and the current `AGENTS.md` invariants. Check `git status`, existing
   private inventory, AWS caller identity, region, instance state, and
   current deployed version. Read design documents' `Status:` lines.
2. **Keep boundaries.** Public Git contains only generic instructions,
   templates, and scripts. Keep account IDs, resource IDs, actual hostnames
   and addresses, SSH paths, credentials, and deployment notes outside the
   checkout. Never print, paste, log, commit, or transmit keys or access
   tokens in chat. Do not copy the Mac's provider keys to the host.
3. **Plan an initial host.** Identify the account and region, compatible
   ARM64 AMI and instance, encrypted EBS, IMDSv2, narrow SSH access, public
   web ports, EIP, DNS provider, billing impact, and backup location.
   Existing resources should be reused when safe; avoid duplicate hosts.
4. **Build and install.** Build from a clean committed revision with
   `scripts/hosting/aws-ec2.sh PRIVATE_ENV build`; transfer and install via
   the matching `deploy` command. Never run Cargo on the small EC2 host or
   replace managed binaries directly. `self verify`, service status, and
   version evidence determine whether deployment succeeded.
5. **Configure with the product.** Complete the setup wizard, choose the
   provider/model and enter keys in Vakyartha. Configure public address and
   trusted hostname in Admin. Use the proxy command after DNS resolves.
   Make no cloud-specific application defaults. Keep the web terminal off
   for a public endpoint.
6. **Observe before declaring success.** Check `status`, `web-check`, the
   active versus saved web address, gateway and Caddy services, and real
   browser owner enrollment and passkey sign-in. HTTP 200 for `/app` only proves
   the shell is served; it does not prove the model route or login works.
   During first enrollment, `copy-token` places the bootstrap token on the
   operator's local clipboard for the browser form. Never ask for it in chat,
   or show the token or recovery codes in logs or an agent answer.
7. **Update carefully.** Read release notes, preserve a backup and previous
   artifact, build, deploy, check managed manifest and service state, then
   check HTTPS and the app. If an update fails, diagnose the specific
   service and restore a compatible previous release through `self install`.
   Never patch the live binary or generated systemd unit by hand.
8. **Maintain a private audit trail.** Record date, source commit, build
   artifact, host resource IDs, DNS/proxy state, installer result, active
   version, verification evidence, and any incident in the private note.
   Record no secret values. Update public docs when the generic procedure
   changes, and keep personal state out of GitHub.

## Routine operator loop

- **Before any operation:** check the current caller identity and region with
  `aws sts get-caller-identity` and inspect the instance via `status`. Read the
  private inventory without printing its values. If the CLI session has
  expired, use the account's approved interactive sign-in method. Never create
  long-lived access keys to bypass an expired session.
- **Daily health:** run `status` and `web-check`. Check both systemd services,
  recent gateway logs, disk space, CPU-credit balance on burstable instances,
  certificate renewal, and AWS Billing alerts. A failed `web-check` means
  inspect DNS, security-group rules, Caddy, then the loopback gateway in that
  order. Do not reopen port 8901 as a workaround.
- **Data protection:** use `vak backup export` to an operator-controlled
  directory outside the live data home, then transfer to encrypted storage.
  Decide explicitly whether `--include-secrets` is needed: it copies both the
  encrypted credential store and its decryption key on headless Linux.
  Verify the manifest and rehearse restore to a separate host or data home
  before relying on backups. Never send backups to Git or issue attachments.
- **Release update:** read the changelog and supported data baseline, preserve
  the previous release artifact and a fresh backup, build from a clean
  committed revision, deploy, run `self verify`, `status`, and `web-check`,
  then exercise passkey sign-in and a normal model turn. Record the old and
  new commits plus outcome in the private note. If an upgrade fails, diagnose
  and restore a *compatible* previous release through the managed installer;
  do not copy over the managed binary by hand.
- **Account operations:** App → Settings → Privacy and safety → Your sign-in
  and Admin → Permissions & Security → Owner sign-in can replace lost recovery
  codes after a fresh passkey assertion. The app account menu and admin menu
  sign out the current browser; Admin can sign out all browsers. The one-time
  codes must remain on screen until the operator confirms saving them. A
  browser session or bootstrap token must never be treated as a substitute
  for a lost owner passkey *and* lost recovery codes.
- **Incident response:** record the symptom and time privately, preserve
  relevant sanitized logs, isolate the affected service or network rule, and
  rotate an exposed secret through its owning credential store. For a stolen
  browser session, revoke all sessions. For an exposed provider key, revoke it
  at the provider and set a replacement in Vakyartha. Do not paste secrets or
  raw transcripts into a public issue. Recheck `/health`, protected-route
  denial, sign-in, and a model turn after repair.

Use the command examples and troubleshooting order in [the EC2 guide](aws-ec2.md).
The private note is the source of truth for that installation's actual IDs,
addresses, cost settings, backups, and last known-good release.
