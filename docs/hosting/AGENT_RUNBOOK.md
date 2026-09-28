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
   browser owner enrollment and passkey sign-in. HTTP 200 for `/app` only proves the shell is served;
   it does not prove the model route or login works. Ask the operator to
   paste a bootstrap token copied locally with `copy-token` during first
   enrollment; never show it or the recovery codes in chat.
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
