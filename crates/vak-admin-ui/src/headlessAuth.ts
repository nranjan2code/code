import { startAuthentication, startRegistration } from "@simplewebauthn/browser";

type Ceremony = { challenge_id: string; options: unknown };

async function json<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, body === undefined ? undefined : {
    method: "POST", headers: { "content-type": "application/json" },
    credentials: "same-origin", body: JSON.stringify(body),
  });
  const result = await response.json() as T & { error?: string };
  if (!response.ok) throw new Error(result.error ?? `Sign-in failed (${response.status})`);
  return result;
}

export const headlessAuth = {
  method: () => json<{ method: "bootstrap" | "passkey" }>("/auth/methods"),
  account: () => json<{ passkeys: number; recovery_codes_remaining: number }>("/auth/account"),
  revokeAll: () => json("/auth/sessions/revoke-all", {}),
  async enroll(token: string): Promise<string[]> {
    const start = await json<Ceremony>("/auth/enroll/start", { token });
    const credential = await startRegistration({ optionsJSON: start.options as Parameters<typeof startRegistration>[0]["optionsJSON"] });
    const result = await json<{ recovery_codes: string[] }>("/auth/enroll/finish", { challenge_id: start.challenge_id, credential });
    return result.recovery_codes;
  },
  async passkey(): Promise<void> {
    const start = await json<Ceremony>("/auth/passkey/start", {});
    const credential = await startAuthentication({ optionsJSON: start.options as Parameters<typeof startAuthentication>[0]["optionsJSON"] });
    await json("/auth/passkey/finish", { challenge_id: start.challenge_id, credential });
  },
  async recovery(code: string): Promise<void> { await json("/auth/recovery", { code }); },
  async addPasskey(): Promise<void> {
    const start = await json<Ceremony>("/auth/passkey/add/start", {});
    const credential = await startRegistration({ optionsJSON: start.options as Parameters<typeof startRegistration>[0]["optionsJSON"] });
    await json("/auth/passkey/add/finish", { challenge_id: start.challenge_id, credential });
  },
};
