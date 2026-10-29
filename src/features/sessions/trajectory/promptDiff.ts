/** One line of a prompt diff: a hunk header or separator (`meta`), an unchanged line, or an added or removed one. */
export interface PromptDiffLine {
  kind: "meta" | "context" | "added" | "removed";
  text: string;
}

type Op = { kind: "context" | "added" | "removed"; text: string };

/** Unchanged lines kept around each change, as in a unified diff. */
const CONTEXT_LINES = 3;
/** Above this many table cells the middle section is shown as a full replacement instead of an LCS. */
const MAX_TABLE_CELLS = 4_000_000;

function splitLines(text: string): string[] {
  return text === "" ? [] : text.split("\n");
}

/**
 * Line-level LCS diff of two texts. Shared prefix and suffix lines are trimmed first, so the table only covers the
 * part that actually changed.
 */
function diffOps(before: string[], after: string[]): Op[] {
  let start = 0;
  while (start < before.length && start < after.length && before[start] === after[start]) start++;
  let endBefore = before.length;
  let endAfter = after.length;
  while (endBefore > start && endAfter > start && before[endBefore - 1] === after[endAfter - 1]) {
    endBefore--;
    endAfter--;
  }
  const head: Op[] = before.slice(0, start).map((text) => ({ kind: "context", text }));
  const tail: Op[] = before.slice(endBefore).map((text) => ({ kind: "context", text }));
  const middleBefore = before.slice(start, endBefore);
  const middleAfter = after.slice(start, endAfter);
  const n = middleBefore.length;
  const m = middleAfter.length;

  if ((n + 1) * (m + 1) > MAX_TABLE_CELLS) {
    return [
      ...head,
      ...middleBefore.map((text): Op => ({ kind: "removed", text })),
      ...middleAfter.map((text): Op => ({ kind: "added", text })),
      ...tail,
    ];
  }

  const width = m + 1;
  const table = new Uint32Array((n + 1) * width);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      table[i * width + j] =
        middleBefore[i] === middleAfter[j]
          ? (table[(i + 1) * width + j + 1] ?? 0) + 1
          : Math.max(table[(i + 1) * width + j] ?? 0, table[i * width + j + 1] ?? 0);
    }
  }

  const middle: Op[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (middleBefore[i] === middleAfter[j]) {
      middle.push({ kind: "context", text: middleBefore[i] ?? "" });
      i++;
      j++;
    } else if ((table[(i + 1) * width + j] ?? 0) >= (table[i * width + j + 1] ?? 0)) {
      middle.push({ kind: "removed", text: middleBefore[i] ?? "" });
      i++;
    } else {
      middle.push({ kind: "added", text: middleAfter[j] ?? "" });
      j++;
    }
  }
  while (i < n) middle.push({ kind: "removed", text: middleBefore[i++] ?? "" });
  while (j < m) middle.push({ kind: "added", text: middleAfter[j++] ?? "" });
  return [...head, ...middle, ...tail];
}

/**
 * Unified-style diff lines between two texts, with `@@` headers per hunk. Mirrors the reference `promptDiffLines`:
 * three lines of context, a blank separator between hunks, and no output when the texts are equal.
 */
export function promptDiffLines(before: string, after: string): PromptDiffLine[] {
  const ops = diffOps(splitLines(before), splitLines(after));
  const changed = ops.flatMap((op, index) => (op.kind === "context" ? [] : [index]));
  if (changed.length === 0) return [];

  // Merge the context windows around each change into hunks.
  const ranges: [number, number][] = [];
  for (const index of changed) {
    const from = Math.max(0, index - CONTEXT_LINES);
    const to = Math.min(ops.length - 1, index + CONTEXT_LINES);
    const last = ranges.at(-1);
    if (last !== undefined && from <= last[1] + 1) last[1] = Math.max(last[1], to);
    else ranges.push([from, to]);
  }

  // Line counts before each op, so every hunk can state where it starts in both texts.
  let oldBefore = 0;
  let newBefore = 0;
  const countsBefore = ops.map((op) => {
    const counts = { old: oldBefore, new: newBefore };
    if (op.kind !== "added") oldBefore++;
    if (op.kind !== "removed") newBefore++;
    return counts;
  });

  return ranges.flatMap(([from, to], hunkIndex): PromptDiffLine[] => {
    const slice = ops.slice(from, to + 1);
    const oldLines = slice.filter((op) => op.kind !== "added").length;
    const newLines = slice.filter((op) => op.kind !== "removed").length;
    const start = countsBefore[from] ?? { old: 0, new: 0 };
    const oldStart = oldLines === 0 ? start.old : start.old + 1;
    const newStart = newLines === 0 ? start.new : start.new + 1;
    return [
      ...(hunkIndex === 0 ? [] : [{ kind: "meta" as const, text: "" }]),
      { kind: "meta" as const, text: `@@ -${oldStart},${oldLines} +${newStart},${newLines} @@` },
      ...slice.map((op): PromptDiffLine => {
        if (op.kind === "added") return { kind: "added", text: `+${op.text}` };
        if (op.kind === "removed") return { kind: "removed", text: `-${op.text}` };
        return { kind: "context", text: ` ${op.text}` };
      }),
    ];
  });
}
