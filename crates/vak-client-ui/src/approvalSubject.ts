/** What an approval is about, in full. The command or target is the decision,
 *  so it is never shortened, hidden behind a disclosure, or left to the
 *  tool's name alone. Every approval card reads it from here. */
export interface ApprovalSubject {
  key: string;
  value: string;
}

const TARGET_KEYS = ["url", "path", "file_path", "file", "dir"] as const;

/** Where a command runs: the workspace when it names no folder, else that
 *  folder inside it. Without the workspace's own path nothing is claimed
 *  about the default, and a named folder is shown as given. */
function runsIn(cwd: string | null, workspace: string | undefined): string | null {
  const named = cwd && cwd !== "." ? cwd.replace(/^\.\//, "") : null;
  if (!workspace) return named;
  if (!named) return workspace;
  if (named.startsWith("/")) return named;
  return `${workspace.replace(/\/+$/, "")}/${named}`;
}

export function approvalSubjects(argsJson: string, workspace?: string): ApprovalSubject[] {
  let args: Record<string, unknown>;
  try {
    const parsed: unknown = JSON.parse(argsJson);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return [];
    args = parsed as Record<string, unknown>;
  } catch {
    return [];
  }
  const text = (key: string) => {
    const value = args[key];
    return typeof value === "string" && value.trim() ? value.trim() : null;
  };
  const command = text("command");
  if (command) {
    const folder = runsIn(text("cwd"), workspace);
    return folder ? [{ key: "command", value: command }, { key: "in", value: folder }] : [{ key: "command", value: command }];
  }
  for (const key of TARGET_KEYS) {
    const value = text(key);
    if (value) return [{ key, value }];
  }
  return [];
}

/** The sandbox that runs an approved `bash` command has no network. Whether
 *  the command wants one is never guessed from its words: every command run
 *  there is held to this, so the note is shown for every one. FullAccess and
 *  an unsandboxed install keep the network, and say nothing. */
export function approvalNetworkNote(tool: string, mode: string | undefined, sandbox: string | undefined): string | null {
  if (tool !== "bash" || !mode || mode === "FullAccess") return null;
  if (!sandbox || sandbox === "off") return null;
  return "It runs in a protected area with no internet access, so a command that downloads or contacts another computer will fail. Allowing it does not change that.";
}
