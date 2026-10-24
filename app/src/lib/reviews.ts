// Suggestions reviewed: how the user corrected each (a place renamed, keywords added or
// taken off, a caption rewritten, a rating changed, or the whole rejected), shown in the
// AI tab's Reviewed list and sent, a line each, when lessons are refined.

import type { Review, Suggestion } from "./api";
import { LEVELS, type Level } from "./aiPlace";

export type Change =
  | { kind: "place"; level: Level; from: string; to: string }
  | { kind: "added"; keyword: string }
  | { kind: "dropped"; keyword: string }
  | { kind: "caption"; from: string; to: string }
  | { kind: "headline"; from: string; to: string }
  | { kind: "rating"; from: number; to: number }
  | { kind: "rejected" };

const trim = (v: string | null | undefined) => v?.trim() ?? "";
const leaf = (path: string) => path.split("|").at(-1) ?? path;

/** What the user changed of a suggestion. Nothing for one accepted as it was; fields left
 *  untaken are no correction (the photo had its own, or the level was unsure). */
export function changes(r: Review): Change[] {
  if (r.status === "Rejected") return [{ kind: "rejected" }];
  const t = r.taken;
  if (!t) return [];
  const s = r.suggestion;
  const out: Change[] = [];
  for (const level of LEVELS) {
    const from = trim(s.place?.[level]?.name);
    const to = trim(t.place[level]);
    if (to && to !== from) out.push({ kind: "place", level, from, to });
  }
  const suggested = new Set([...s.keywords, ...s.keywords_new].map((k) => k.toLowerCase()));
  const taken = new Set(t.keywords.map((k) => k.toLowerCase()));
  for (const k of t.keywords) if (!suggested.has(k.toLowerCase())) out.push({ kind: "added", keyword: k });
  for (const k of s.keywords) if (!taken.has(k.toLowerCase())) out.push({ kind: "dropped", keyword: k });
  const caption = trim(t.caption);
  const captions = [trim(s.caption_en), trim(s.caption_pl)].filter(Boolean);
  if (caption && captions.length && !captions.includes(caption)) out.push({ kind: "caption", from: captions[0], to: caption });
  const headline = trim(t.headline);
  if (headline && trim(s.headline) && headline !== trim(s.headline)) {
    out.push({ kind: "headline", from: trim(s.headline), to: headline });
  }
  if (t.rating && s.rating && t.rating !== s.rating) out.push({ kind: "rating", from: s.rating, to: t.rating });
  return out;
}

/** A change as a chip in the list. */
export function chipText(c: Change): string {
  switch (c.kind) {
    case "place": return `${c.level}: ${c.from || "—"} → ${c.to}`;
    case "added": return `+ ${leaf(c.keyword)}`;
    case "dropped": return `− ${leaf(c.keyword)}`;
    case "caption": return "caption rewritten";
    case "headline": return `headline: ${c.to}`;
    case "rating": return `★${c.from} → ★${c.to}`;
    case "rejected": return "rejected";
  }
}

const cut = (text: string, n = 300) => (text.length > n ? `${text.slice(0, n - 1)}…` : text);
const quote = (text: string) => `"${cut(text)}"`;

/** What the model suggested, briefly: for a rejected suggestion, what was turned down. */
function suggested(s: Suggestion): string {
  const place = s.place && LEVELS.map((l) => s.place?.[l]?.name).filter(Boolean).join(" › ");
  return [
    s.headline && `headline ${quote(s.headline)}`,
    (s.caption_en || s.caption_pl) && `caption ${quote((s.caption_en || s.caption_pl)!)}`,
    s.keywords.length + s.keywords_new.length ? `keywords ${[...s.keywords, ...s.keywords_new].map(leaf).join(", ")}` : "",
    place && `place ${place}`,
    s.rating && `rating ${s.rating}`,
  ].filter(Boolean).join("; ");
}

/** One line for the model refining lessons. */
export function digest(r: Review): string | null {
  const list = changes(r);
  if (!list.length) return null;
  if (list[0].kind === "rejected") return `- Rejected as a whole: ${suggested(r.suggestion) || "(empty)"}`;
  const parts = list.map((c) => {
    switch (c.kind) {
      case "place": return c.from ? `${c.level} ${quote(c.from)} changed to ${quote(c.to)}` : `${c.level} ${quote(c.to)} added`;
      case "added": return `keyword ${quote(c.keyword)} added`;
      case "dropped": return `keyword ${quote(c.keyword)} taken off`;
      case "caption": return `caption ${quote(c.from)} rewritten as ${quote(c.to)}`;
      case "headline": return `headline ${quote(c.from)} changed to ${quote(c.to)}`;
      case "rating": return `rating ${c.from} changed to ${c.to}`;
      default: return "";
    }
  });
  return `- ${parts.join("; ")}`;
}

export type Shown = "all" | "edited" | "rejected" | "unchanged";
export function shows(r: Review, which: Shown): boolean {
  if (which === "all") return true;
  if (which === "rejected") return r.status === "Rejected";
  const edited = r.status === "Accepted" && changes(r).length > 0;
  return which === "edited" ? edited : r.status === "Accepted" && !edited;
}
