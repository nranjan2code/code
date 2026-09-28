# 78 — Headless identity

Status: **in progress**. This document is the implementation contract for
human sign-in on a network-exposed Vakyartha server. The current bearer-token
browser form is a bootstrap path until the owner enrolls.

## Boundary

Vakyartha has three distinct credentials:

1. The gateway bearer is for CLI, channel bridges, and first-owner bootstrap.
   It remains in the shared secret scope and never becomes a browser cookie.
2. A passkey proves the human owner's identity. Its relying-party origin is
   the exact `[server].public_url` origin. A hostname change requires a new
   passkey enrollment while the old origin is still reachable, or recovery.
3. An opaque browser session is a short-lived, revocable server-side record.
   Its cookie is `HttpOnly`, `SameSite=Strict`, `Secure` on HTTPS, and
   `Path=/`, shared by `/app` and `/admin`. Only a digest is stored server-side.

The desktop's loopback auto-login is local-machine convenience, separate from
headless identity. It cannot grant a session through a public hostname.

## Install and recovery

An empty installation has no owner. A holder of the gateway bootstrap token
may begin the one-time owner enrollment. The server stores a registration
challenge with a short deadline and consumes it exactly once. Enrollment
verifies the authenticator response and atomically records the first owner;
concurrent enrollments cannot create a second owner. It issues recovery codes
once. Only hashes are retained; an operator must save the codes outside the
server. Once the owner exists, the browser token login is disabled.

The owner signs in with a passkey or spends a recovery code. A spent code is
removed atomically. A recovery session can enroll a replacement passkey.
Neither a public form nor an unauthenticated API can reset the owner. The CLI
bootstrap token does not become a backdoor after enrollment; disaster recovery
requires an authenticated owner or deliberate local data-home maintenance.

Additional passkeys can be enrolled from an authenticated owner session. The
owner can sign out one browser session or revoke all sessions. Passkey removal
and recovery-code rotation are follow-up account-management work. A restart invalidates in-memory browser
sessions and pending challenges. Session expiry is enforced server-side.

## Storage and request rules

The owner record lives under the canonical data home and is registered in
`vak_core::state::REGISTRY` for backup and purge. It contains a stable owner
id, serialized public-key credentials, and hashes of recovery codes. Writes
are atomic, mode 0600 on Unix, and protected by an OS file lock. Concurrent
processes serving the same data home serialize enrollment and recovery writes.

The browser sends no gateway bearer after enrollment. State-changing cookie
requests require an exact first-party `Origin` matching the configured public
origin including scheme and port. Originless clients must use bearer auth.
The server rejects untrusted `Host` values before routing and never trusts an
arbitrary `X-Forwarded-For` as a security principal. Rate limiting uses the
socket peer unless an operator explicitly trusts that proxy IP and the proxy
overwrites `X-Real-IP` with its observed client address. Public health reports
readiness only. Authentication failures disclose neither a credential prefix
nor whether a passkey id exists. Login, challenge, and recovery endpoints
have rate limits. Security events carry a decision and opaque correlation id,
never the secret, assertion, recovery code, or page content.

## Scope

One owner controls the existing single-user data home, workspace picker,
agent execution, and admin console. This does **not** pretend to be
multi-tenant: adding invited users requires account-specific authorization
on every route, workspace, session, secret, and event stream. OIDC can later
be an optional sign-in method bound to this owner identity, with issuer,
audience, nonce, state, and PKCE validated. It is not required for a fresh
self-hosted installation. Provider keys and gateway tokens remain separate
from human identity.

## Acceptance

- A fresh headless install can enroll and use a passkey without an external
  identity provider or a cloud-specific service.
- `/app` and `/admin` use the same authenticated browser session; logout
  revokes it server-side and a restart invalidates it.
- The gateway bearer cannot sign in to a browser after owner enrollment.
- Passkey challenges are single-use, short-lived, origin-bound, and verified
  server-side; a lost passkey can be replaced with one recovery code.
- Mutations with an absent or wrong `Origin` cannot use a cookie; exact
  scheme, host, and port are checked.
- Public unauthenticated routes reveal no workspace, model, permission, or
  credential state; no secret appears in Git, logs, or an API response.
