import { startAuthentication, startRegistration } from "@simplewebauthn/browser";

type AuthMethod = { method: "bootstrap" | "passkey" };
type Ceremony = { challenge_id: string; options: Parameters<typeof startRegistration>[0]["optionsJSON"] };

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
  method: () => json<AuthMethod>("/auth/methods"),
  async enroll(token: string): Promise<string[]> {
    const start = await json<Ceremony>("/auth/enroll/start", { token });
    const credential = await startRegistration({ optionsJSON: start.options });
    const result = await json<{ recovery_codes: string[] }>("/auth/enroll/finish", {
      challenge_id: start.challenge_id, credential,
    });
    return result.recovery_codes;
  },
  async passkey(): Promise<void> {
    const start = await json<Ceremony>("/auth/passkey/start", {});
    const credential = await startAuthentication({ optionsJSON: start.options });
    await json("/auth/passkey/finish", { challenge_id: start.challenge_id, credential });
  },
  async recovery(code: string): Promise<void> {
    await json("/auth/recovery", { code });
  },
  async addPasskey(): Promise<void> {
    const start = await json<Ceremony>("/auth/passkey/add/start", {});
    const credential = await startRegistration({ optionsJSON: start.options });
    await json("/auth/passkey/add/finish", { challenge_id: start.challenge_id, credential });
  },
  async rotateRecovery(): Promise<string[]> {
    const start = await json<Ceremony>("/auth/recovery/rotate/start", {});
    const credential = await startAuthentication({ optionsJSON: start.options });
    const result = await json<{ recovery_codes: string[] }>("/auth/recovery/rotate/finish", {
      challenge_id: start.challenge_id, credential,
    });
    return result.recovery_codes;
  },
};
