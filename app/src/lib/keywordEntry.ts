// Typing keywords to add: several at once, separated by `;` or new lines (a comma cannot
// be in a Capture One keyword name, so it is not a separator). Levels are split by `|`.

/** The keywords in typed text, tidied (spaces around levels trimmed, repeats dropped,
 *  ignoring case). With parts that cannot be keywords: the first reason, and those parts
 *  as typed in `bad` (the good ones are still in `paths`). */
export function parseKeywords(text: string): { paths: string[]; error?: string; bad?: string[] } {
  const paths: string[] = [];
  const seen = new Set<string>();
  let error: string | undefined;
  const bad: string[] = [];
  for (const part of text.split(/[;\r\n]+/)) {
    if (!part.trim()) continue;
    const levels = part.split("|").map((l) => l.trim());
    const path = levels.join("|");
    const wrong = levels.some((l) => !l)
      ? "A keyword level is empty."
      : path.includes(",")
        ? "Keyword names cannot contain a comma."
        : undefined;
    if (wrong) {
      error ??= wrong;
      bad.push(part.trim());
      continue;
    }
    if (seen.has(path.toLowerCase())) continue;
    seen.add(path.toLowerCase());
    paths.push(path);
  }
  return error ? { paths, error, bad } : { paths };
}

/** The input's text with `pasted` put in place of its selection (or at the caret). */
export function pasteAt(value: string, start: number | null, end: number | null, pasted: string): string {
  const from = start ?? value.length;
  const to = end ?? from;
  return value.slice(0, from) + pasted + value.slice(to);
}

/** `adding` with `more` after it, leaving out those already there (ignoring case). */
export function withKeywords(adding: string[], more: string[]): string[] {
  const seen = new Set(adding.map((p) => p.toLowerCase()));
  const out = [...adding];
  for (const p of more) {
    if (seen.has(p.toLowerCase())) continue;
    seen.add(p.toLowerCase());
    out.push(p);
  }
  return out;
}
