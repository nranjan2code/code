export interface DiffLine {
  type: "ctx" | "add" | "del" | "hunk";
  text: string;
  oldNo?: number;
  newNo?: number;
}

export interface DiffFile {
  path: string;
  lines: DiffLine[];
  adds: number;
  dels: number;
  binary?: boolean;
}

/** Split a unified diff into per-file structures. */
export function parseDiff(diff: string): DiffFile[] {
  const files: DiffFile[] = [];
  const chunks = diff.split(/^diff --git /m).filter((c) => c.trim());
  for (const chunk of chunks) {
    const header = chunk.split("\n")[0] ?? "";
    const m = /^a\/(.*) b\/(.*)$/.exec(header.trim());
    const path = m ? (m[2] || m[1]) : header;
    if (/^Binary files /.test(chunk)) {
      files.push({ path, lines: [], adds: 0, dels: 0, binary: true });
      continue;
    }
    const file: DiffFile = { path, lines: [], adds: 0, dels: 0 };
    let oldNo = 0;
    let newNo = 0;
    for (const line of chunk.split("\n").slice(1)) {
      if (line.startsWith("@@")) {
        const hm = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
        oldNo = hm ? parseInt(hm[1], 10) : 0;
        newNo = hm ? parseInt(hm[2], 10) : 0;
        file.lines.push({ type: "hunk", text: line });
      } else if (line.startsWith("+")) {
        file.lines.push({ type: "add", text: line.slice(1), newNo: newNo++ });
        file.adds++;
      } else if (line.startsWith("-")) {
        file.lines.push({ type: "del", text: line.slice(1), oldNo: oldNo++ });
        file.dels++;
      } else if (!line.startsWith("\\")) {
        file.lines.push({ type: "ctx", text: line.slice(1), oldNo: oldNo++, newNo: newNo++ });
      }
    }
    files.push(file);
  }
  return files;
}

export function parseStatus(status: string): { untracked: string[]; changed: string[] } {
  const untracked: string[] = [];
  const changed: string[] = [];
  for (const line of status.split("\n")) {
    const t = line.trimEnd();
    if (!t) continue;
    const code = t.slice(0, 2);
    const path = t.slice(3);
    if (code === "??") untracked.push(path);
    else changed.push(`${code} ${path}`);
  }
  return { untracked, changed };
}
