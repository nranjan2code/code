# Headless Vakyartha on one AWS EC2 host

This is an example deployment of the provider-neutral hosting contract in
[Hosting Vakyartha](../hosting.md). The repository contains the reusable
procedure and scripts. **The operator owns a private inventory file outside
the checkout** with their AWS profile, instance ID, SSH key path, and public
hostname. No provider key, gateway token, AWS credential, real account ID,
instance address, or deployment hostname belongs in this repository.

This path builds Linux ARM64 binaries locally using Docker and runs them on
Amazon Linux 2023. It does not build on EC2. The backend listens on
`127.0.0.1:8901`; Caddy is the only public web process on ports 80 and 443.
Vakyartha's owner passkey sign-in protects `/app` and `/admin` after one-time
bootstrap with the gateway token.
An AI agent operating this path should follow the
[agent runbook](AGENT_RUNBOOK.md) as well as this guide.

## 1. Local prerequisites and private inventory

Install Docker with Buildx, AWS CLI v2, OpenSSH, Python 3, `git`, and `curl`.
Sign in with an AWS CLI profile; use short-lived IAM Identity Center credentials
where possible. Confirm account and region before changing resources:

```bash
aws --profile YOUR_PROFILE sts get-caller-identity
aws --profile YOUR_PROFILE --region YOUR_REGION ec2 describe-instances --max-results 5
```

Create a private directory outside this checkout and copy
[`aws-ec2.example.env`](../../scripts/hosting/aws-ec2.example.env) there.
Fill the values, use an absolute key path, and run `chmod 600` on the file.
The script refuses to source an inventory inside the checkout. Never paste
the contents into an issue, PR, terminal transcript, or agent answer.

```bash
private_inventory="$HOME/Library/Application Support/Vakyartha/private-deployments/aws.env"
chmod 600 "$private_inventory"
scripts/hosting/aws-ec2.sh "$private_inventory" status
```

The example assumes a macOS build machine. `copy-token` uses `pbcopy`; the
other commands use standard CLI tools. `PUBLIC_HOST` is a normal DNS name,
not a secret; keeping the actual name in the private inventory prevents a
personal deployment from becoming a default in the public project.

## 2. Provision the instance

In the AWS console, choose the intended account and region before creating
anything. Under **EC2 → Instances → Launch instances**:

1. Select Amazon Linux 2023 **ARM64** and a compatible Graviton instance
   type. Size it for the actual workload; remote model inference keeps the
   host smaller than local inference would. Do not assume it is free. Review
   current compute, EBS, IPv4, transfer, and burst-credit prices and your
   account's credit or Free Tier eligibility in Billing.
2. Use a dedicated key pair, download its private key outside the repo,
   `chmod 600` it, and record only its path in the private inventory. An
   instance role and Session Manager can replace SSH in a future setup;
   never copy AWS access keys to the host.
3. Use an encrypted gp3 root EBS volume sized for the binary, workspace,
   ledgers and logs. Enable termination protection. Require IMDSv2. If you
   later run a container that needs the instance role, review the metadata
   hop limit for that network mode.
4. Give the security group inbound **22 from the operator's current /32**
   only, and **80 and 443 from the clients that must reach the public web**.
   Do not open 8901. The instance needs outbound DNS/HTTPS for updates,
   certificate issuance, and remote model providers.
5. Reserve one Elastic IP and associate it with this instance. Record the
   instance ID, region, and key path in the private inventory. Record the
   EIP allocation/association IDs in the private operations note. An EIP
   and its public IPv4 usage may be billed even when compute is stopped.
6. In the domain's existing DNS provider, add an A record for your chosen
   subdomain pointing to the Elastic IP. DNS management does not need to
   move to Route 53. Wait until public resolvers return the right address
   before starting certificate issuance.

For AWS CLI work, first inspect the chosen VPC, subnet, AMI, security group,
key pair and current account quotas; then create only missing resources.
Record every created resource ID in the private note. The repo script does
**not** create or delete cloud resources automatically: provisioning is a
deliberate account-level decision, while `deploy` operates an already chosen
instance. An AI agent must show the concrete account, region, instance size,
network rules and expected cost sources to the operator before a new launch.

## 3. Bootstrap Vakyartha

Confirm SSH host identity through the AWS console or a trusted channel
before accepting the first host key. Keep the verified entry in
`~/.ssh/known_hosts`; the script uses `StrictHostKeyChecking=yes` and will
refuse an unknown or changed host.

Build locally from a **committed, clean source tree**, then install using
Vakyartha's managed installer. The builder stages tracked source only into
Docker, with no local `.env`, `.vak`, Git metadata, or AWS credentials. The
Amazon Linux base image and repository release are pinned, and Rust is pinned
to 1.98.1.
`Cargo.lock`, target triple, Docker Buildx version, complete builder
package/toolchain inventory, and SHA-256 digests are recorded beside each
release. Artifacts are retained under
the full source commit and builder-definition digest, so a later release or a
changed builder cannot overwrite the last one:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" build
scripts/hosting/aws-ec2.sh "$private_inventory" deploy
scripts/hosting/aws-ec2.sh "$private_inventory" status
scripts/hosting/aws-ec2.sh "$private_inventory" web-check
```

`deploy` checks the manifest's source commit, lockfile digest, pinned
builder/toolchain, builder environment evidence, and both binary digests
before transfer. It copies
`vak` and `vak-delivery-worker` into a temporary host directory, calls
`vak self install --force`, and checks `self verify`. If a gateway service was
already registered it also calls `self services-sync` and `self status`. A
first install is left inactive until setup. Never overwrite binaries in the
managed prefix by hand: that breaks its manifest. A clean release build can
take several minutes at the final Rust link step; do not interrupt it merely
because that step is quiet. Check `docker buildx history ls` before deciding
that a build is stalled.
The canonical host workspace is `~/vak-home`. On a fresh host complete the
terminal setup wizard before expecting a ready model route:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" setup
```

The wizard activates the durable gateway as part of setup. Systemd user
services stop when the account's last SSH session ends unless user
lingering is on, and then Caddy answers 502: enable it with
`sudo loginctl enable-linger ec2-user`. Setup, `vak self status`,
`services-sync` and `vak doctor` report it when it is off. Check from
outside with no SSH session open (`web-check`) before declaring a deploy
done.

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" tunnel
# In a browser on the build machine: http://127.0.0.1:18901/app
```

Choose a provider and model in the Vakyartha UI and enter its key there.
Provider keys go to Vakyartha's secure credential store on the host, not to
the inventory, shell history, service unit, build context, or Git. Use a
remote model provider on a small instance.

The built-in `webfetch` can retrieve a known public URL without a search
provider key. A turn that reads a public news RSS feed and follows its links
may produce cited news without Tavily being configured. Tavily is a separate
MCP search integration and needs its own key. To know which route a turn
actually used, inspect its session ledger's tool calls; a citation card or
the absence of a key alone does not establish that route.

## 4. Publish the HTTPS address

After DNS points at the instance and ports 80/443 are open, install Caddy:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" proxy
```

`proxy` downloads the pinned ARM64 release and official checksum list,
checks SHA-512, then installs the Caddy binary, a generated Caddyfile for
`PUBLIC_HOST`, and a systemd service. Caddy terminates TLS and proxies to
the loopback backend. It redirects `/` to `/app`. The public hostname is
generated from the private inventory; no real hostname is tracked in Git.
The generated proxy overwrites `X-Real-IP` with the client address it sees.
To rate-limit individual visitors behind this loopback proxy, set
`gateway.rate_limit.trusted_proxy_ips = ["127.0.0.1"]` in the host's private
configuration. Trust only the actual proxy socket address; direct public
access to the backend must remain closed.

In **Admin → Model & providers → Infrastructure → Web address**, set:

- HTTPS public address: `https://` followed by `PUBLIC_HOST`.
- Accepted hostname: exactly `PUBLIC_HOST`.
- Sign-in lifetime: the desired number of hours, such as 24.

Save and restart the Vakyartha gateway from Operations. Keep the backend
bind on loopback and leave the web terminal disabled for public access.
The screen shows both saved and active values so you can see when a restart
is still needed. Check TLS and both browser shells:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" web-check
```

First-owner enrollment requires the pinned gateway token. On the operator's
Mac, run the following and paste from the clipboard into the initial setup
form. Register a passkey on the public HTTPS address and store the recovery
codes in a password manager. Later visits use the passkey, not the token:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" copy-token
```

If the recovery-code screen is closed before saving, sign in with the
registered passkey. In **Admin → Permissions & Security → Owner sign-in**, or
**App → Settings → Privacy and safety → Your sign-in**, choose **Generate new
recovery codes**. Confirm with the passkey and save the new set; generating it
invalidates the previous codes.

To end the current browser session, use **App → account menu → Sign out**,
**App → Settings → Privacy and safety → Your sign-in → Sign out**, or the
**Admin → Sign out** menu. **Admin → Permissions & Security → Owner sign-in →
Sign out all browsers** invalidates every active browser session. A service
restart also invalidates the in-memory sessions; the owner can sign in again
with a passkey or an unused recovery code.

The script does not print the token or a token-bearing URL. Never send that
token through chat, logs, screenshots, GitHub, or a public URL query string.
If a token is exposed, revoke or rotate it through the secure credential
store and restart affected services; do not merely remove the leaked text.

## 5. Update and operate

Read the release notes and supported data baseline before an upgrade. Check
the AWS caller profile, host status, deployed source commit, service state,
disk capacity and backup freshness first. Confirm the existing host key is
trusted in `known_hosts`; never disable strict host key checking. Keep the
previous source commit and its full-commit artifact directory locally. Commit
the new source first because the builder refuses a dirty tree. Build and
install the exact full commit:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" build
scripts/hosting/aws-ec2.sh "$private_inventory" deploy
scripts/hosting/aws-ec2.sh "$private_inventory" status
scripts/hosting/aws-ec2.sh "$private_inventory" web-check
```

The build command writes
`target/amazonlinux-arm64/<full-commit>/<builder-definition-sha256>/`. Review
its `build-manifest.json` and `build-environment.txt`; the deploy command
refuses an artifact for another commit, a changed Cargo lockfile, a different
builder image/toolchain, or modified binaries. Keep that directory until the new
release has passed its checks and the rollback window has ended.

If an update fails, inspect `status` and sanitized gateway logs before
retrying. To reinstall a retained, previously validated artifact without
checking out another branch, pass its full 40-character commit:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" deploy PREVIOUS_FULL_COMMIT
scripts/hosting/aws-ec2.sh "$private_inventory" status
scripts/hosting/aws-ec2.sh "$private_inventory" web-check
```

If no artifact for that source and current builder exists, first run
`scripts/hosting/aws-ec2.sh "$private_inventory" build PREVIOUS_FULL_COMMIT`.
This materializes the exact old application source from Git objects, while
using the current pinned builder. The deploy command requires that commit to
exist in the local Git object database and verifies its `Cargo.lock` and
builder definition against the retained manifest. After
rollback, record the failed and restored commits and diagnose before another
attempt. The command output intentionally omits public IPs and AWS resource
identifiers; consult the private operations note or AWS console when those
identifiers are needed.

The post-deploy checks prove binary integrity, service state and HTTPS shell
availability. For a provider-backed voice smoke test, sign in to the deployed
Admin UI, confirm the intended agent voice route and provider key state, then
exercise a short synthesis in the owning channel. Do not put provider keys in
CLI arguments or logs. Without an authorized provider key, report voice
runtime verification as pending rather than inferring it from `/app` or
`/admin` returning 200.

For day-to-day investigation:

```bash
scripts/hosting/aws-ec2.sh "$private_inventory" status
scripts/hosting/aws-ec2.sh "$private_inventory" logs
aws --profile YOUR_PROFILE --region YOUR_REGION ec2 describe-instance-status --instance-ids YOUR_INSTANCE_ID
```

`logs` prints the latest gateway journal entries; review before sharing
them. For Caddy, use `sudo journalctl -u caddy.service` over SSH. An agent
must avoid dumping secrets, raw session content, or the private inventory
into its transcript when investigating an incident.

| Cadence | Check | If it fails |
|---|---|---|
| Each use or deployment | `status`, `web-check`, and a real passkey sign-in | Check EC2 state, DNS and TLS, Caddy, then gateway logs; keep port 8901 closed to the internet. |
| Daily | Instance status checks, gateway and Caddy service state, EBS free space, and AWS billing alerts | Investigate the affected layer before restarting or resizing; record the incident privately. |
| Weekly | Certificate renewal status, security-group rules, SSH source range, backup freshness, and CPU credits on burstable instances | Correct drift in the owning console or private configuration; document the change. |
| Each release | Changelog, data baseline, backup, clean local build, managed install, `self verify`, app and model turn | Restore a compatible prior managed release only after diagnosing the failure. |

For a backup, run `vak backup export DESTINATION` on the host from the
canonical workspace; choose an operator-controlled destination outside the
live data home. The export includes the registry's durable state, but omits
the credential store by default. `--include-secrets` copies the encrypted
credential store **and its local decryption key** on a headless Linux host;
protect that backup as a live secret. Copy the backup to encrypted storage,
inspect `manifest.json`, and rehearse `vak backup import SOURCE` in a separate
environment before relying on it. Never commit either backup to Git. See
`vak backup --help` for the version's exact options.

Keep an operator note outside the checkout with the current commit, artifact
hashes, managed prefix, instance and network resource IDs, DNS state, cost
alarms, last backup and restore check, and rollback revision. Record **paths
or references**, not secret values. The example inventory contains the
minimum targeting fields for the script; the note records operational
history. AWS account ownership and current pricing are verified in the AWS
console, not inferred from a Free Tier label.

Stopping the instance stops the web service but does not remove EBS or
public IPv4 charges. An EIP/DNS change requires updating the A record; a
hostname change also requires Caddy and Admin Web address changes. After
either change, run `proxy`, restart the gateway when its saved web address
changes, then `web-check` and sign in again. A hostname change can require
enrolling a passkey for the new relying-party ID, so preserve access to the
old address and recovery material until the new origin works.

To retire a host, first take and retain a verified backup, record the
resource inventory, then explicitly remove the instance, EBS volumes,
Elastic IP, security group, key pair if unused, and DNS record. There is no
automated `destroy` command because those actions can erase the only copy
of sessions and agent state.

## Boundary with other clouds

The application configuration is the same on AWS, GCP, Azure, or a local
Linux host: loopback gateway, HTTPS reverse proxy, exact trusted hostname,
secure credential store, managed install and service supervision. AWS-specific
code here only resolves an EC2 host and transfers artifacts. Another cloud
adapter should provide host discovery and SSH/SSM transport while using
the same build and Vakyartha lifecycle commands.
