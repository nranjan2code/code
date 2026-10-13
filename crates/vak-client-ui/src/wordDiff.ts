/** What changed between two texts, for showing them side by side: each side
 *  as runs of text, the runs the other side lacks marked. Lines are matched
 *  first, then the words of a changed stretch, so a one-word edit in a long
 *  file marks one word (docs/design/90-file-state.md §5, Compare with your
 *  folder). */
export type Run = { text: string; changed: boolean };

/** Myers' shortest edit script over two token lists: for each side, which
 *  tokens the other side does not have. */
function edits(a: string[], b: string[]): { keptA: boolean[]; keptB: boolean[] } {
  const n = a.length;
  const m = b.length;
  const max = n + m;
  const offset = max + 1;
  const v = new Array<number>(2 * max + 3).fill(0);
  const trace: number[][] = [];
  outer: for (let d = 0; d <= max; d++) {
    trace.push(v.slice());
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1]) ? v[offset + k + 1] : v[offset + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[offset + k] = x;
      if (x >= n && y >= m) {
        trace.push(v.slice());
        break outer;
      }
    }
  }
  const keptA = new Array<boolean>(n).fill(false);
  const keptB = new Array<boolean>(m).fill(false);
  let x = n;
  let y = m;
  for (let d = trace.length - 2; d >= 0 && (x > 0 || y > 0); d--) {
    const vd = trace[d];
    const k = x - y;
    const prevK = k === -d || (k !== d && vd[offset + k - 1] < vd[offset + k + 1]) ? k + 1 : k - 1;
    const prevX = vd[offset + prevK];
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      keptA[x - 1] = true;
      keptB[y - 1] = true;
      x--;
      y--;
    }
    if (d > 0) {
      x = prevX;
      y = prevY;
    }
  }
  return { keptA, keptB };
}

/** Joins tokens into runs of equal state. */
function runs(tokens: string[], kept: boolean[]): Run[] {
  const out: Run[] = [];
  tokens.forEach((text, index) => {
    const changed = !kept[index];
    const last = out[out.length - 1];
    if (last && last.changed === changed) last.text += text;
    else out.push({ text, changed });
  });
  return out;
}

const words = (text: string) => text.split(/(\s+)/).filter((token) => token.length > 0);

/** Each side as runs, what it has that the other lacks marked. */
export function compareTexts(before: string, after: string): { before: Run[]; after: Run[] } {
  const linesA = before.split(/(?<=\n)/);
  const linesB = after.split(/(?<=\n)/);
  const { keptA, keptB } = edits(linesA, linesB);
  const outA: Run[] = [];
  const outB: Run[] = [];
  let i = 0;
  let j = 0;
  while (i < linesA.length || j < linesB.length) {
    if (i < linesA.length && j < linesB.length && keptA[i] && keptB[j]) {
      outA.push({ text: linesA[i++], changed: false });
      outB.push({ text: linesB[j++], changed: false });
      continue;
    }
    // A changed stretch: the lines up to the next kept pair on each side,
    // compared word by word.
    const startA = i;
    const startB = j;
    while (i < linesA.length && !keptA[i]) i++;
    while (j < linesB.length && !keptB[j]) j++;
    const wordsA = words(linesA.slice(startA, i).join(""));
    const wordsB = words(linesB.slice(startB, j).join(""));
    const inner = edits(wordsA, wordsB);
    outA.push(...runs(wordsA, inner.keptA));
    outB.push(...runs(wordsB, inner.keptB));
  }
  return { before: outA, after: outB };
}
