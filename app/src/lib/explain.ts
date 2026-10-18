// Why a photo is in a class, and what each class asks: the details panel's Look section and
// the analysis charts say it in words, from the lines the library draws (Settings > Analysis).
import { HUE_NAMES, type ClassSettings, type LookView } from "./api";

type Stat = { share: number; patch: number; chroma: number };

/** A share as a percent: whole above 10%, one decimal below, "<0.1%" for a trace. */
export function pct(x: number): string {
  const p = x * 100;
  if (p > 0 && p < 0.1) return "<0.1%";
  return `${p >= 10 || p === 0 ? Math.round(p) : Math.round(p * 10) / 10}%`;
}
const num = (n: number) => Math.round(n).toLocaleString("en-US");

/** The colour name of an sRGB colour, as the analysis gives it (`c1analysis::name_of`). */
export function nameOf([r8, g8, b8]: [number, number, number]): string {
  const [r, g, b] = [r8 / 255, g8 / 255, b8 / 255];
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const [v, delta] = [max, max - min];
  const s = max > 0 ? delta / max : 0;
  if (v < 0.16) return "black";
  if (s < 0.16 || delta < 0.06) return v > 0.85 ? "white" : "grey";
  const mod = (a: number, n: number) => ((a % n) + n) % n;
  const hue =
    delta === 0 ? 0 : max === r ? 60 * mod((g - b) / delta, 6) : max === g ? 60 * ((b - r) / delta + 2) : 60 * ((r - g) / delta + 4);
  if (hue < 15 || hue >= 345) return v < 0.45 ? "brown" : "red";
  if (hue < 45) return v < 0.6 ? "brown" : "orange";
  if (hue < 70) return v < 0.5 ? "brown" : "yellow";
  if (hue < 165) return "green";
  if (hue < 200) return "teal";
  if (hue < 260) return "blue";
  if (hue < 295) return "purple";
  return "pink";
}

/** Black, white and grey: shown in the palette, never a colour of the photo. */
export const NEUTRAL = ["black", "white", "grey"];

const vivid = (st: Stat, l: ClassSettings) => st.patch >= l.patch_share && st.chroma >= l.patch_chroma;

/** The rule that makes a photo a colour, in words. */
export function colourRule(l: ClassSettings): string {
  return l.colour_mode === "Patch"
    ? `a colour counts with a vivid patch of ${pct(l.patch_share)} or more (chroma ${num(l.patch_chroma)}+)`
    : `a colour counts when it covers ${pct(l.colour_share)} of the photo or more`;
}

export type ColourLine = { name: string; counts: boolean; text: string };

/** The photo's colours with their measure, biggest first, each marked whether it counts:
 *  "red 19% · counts from 17%", "red patch 4% (chroma 62) · counts from 2%". */
export function colourLines(stats: Stat[], l: ClassSettings, classes: string[], top = 5): ColourLine[] {
  const patch = l.colour_mode === "Patch";
  const size = (st: Stat) => (patch ? st.patch : st.share);
  return HUE_NAMES.map((name, i) => ({ name, st: stats[i] ?? { share: 0, patch: 0, chroma: 0 } }))
    .filter(({ st }) => size(st) >= 0.001)
    .sort((a, b) => size(b.st) - size(a.st))
    .slice(0, top)
    .map(({ name, st }) => {
      const counts = classes.includes(name);
      const measure = patch ? `${name} patch ${pct(st.patch)} (chroma ${num(st.chroma)})` : `${name} ${pct(st.share)}`;
      const line = patch ? `${pct(l.patch_share)}, chroma ${num(l.patch_chroma)}` : pct(l.colour_share);
      return { name, counts, text: `${measure} · ${counts ? "counts" : "needs"} ${counts ? "from" : ""} ${line}`.replace("  ", " ") };
    });
}

/** Why the photo is (or is not) black and white. */
export function monoLine(chroma: number, stats: Stat[], l: ClassSettings): string {
  const patches = stats.some((st) => vivid(st, l));
  const mono = chroma < l.mono_chroma && !patches;
  const why = `mean chroma ${chroma.toFixed(1)} (black & white below ${l.mono_chroma})`;
  return mono ? why : chroma < l.mono_chroma && patches ? `${why}, but it has a vivid patch` : why;
}

/** "brightness 31% (dark below 20%, bright above 55%) · clipped 4% (from 3%) · …" */
export function lightLine(a: LookView["analysis"], l: ClassSettings): string {
  return [
    `brightness ${pct(a.brightness)} (dark below ${pct(l.dark_below)}, bright above ${pct(l.bright_above)})`,
    `clipped ${pct(a.highlights)} (from ${pct(l.clipped_share)})`,
    `blocked ${pct(a.shadows)} (from ${pct(l.blocked_share)})`,
    `contrast ${a.contrast.toFixed(2)} (low below ${l.flat_below})`,
  ].join(" · ");
}

/** "detail 1,240 (soft below 700, blurry below 300)" */
export const focusLine = (sharpness: number, l: ClassSettings) =>
  `detail ${num(sharpness)} (soft below ${num(l.soft_below)}, blurry below ${num(l.blurry_below)})`;

/** "width ÷ height 1.50 (square within 1.1, panorama from 2)" */
export const shapeLine = (w: number, h: number, l: ClassSettings) =>
  `width ÷ height ${(w / Math.max(1, h)).toFixed(2)} (square within ${l.square_ratio}, panorama from ${l.panorama_ratio})`;

/** The analysis charts. */
export type Card = "shape" | "hue" | "brightness" | "problems" | "focus";

/** What a card measures, under its title. */
export function cardHint(card: Card, l: ClassSettings): string {
  switch (card) {
    case "shape":
      return `Width ÷ height: square within ${l.square_ratio}, panorama from ${l.panorama_ratio}.`;
    case "hue":
      return `A photo can have several colours: ${colourRule(l)}. Black & white below chroma ${l.mono_chroma}.`;
    case "brightness":
      return `Mean brightness: dark below ${pct(l.dark_below)}, bright above ${pct(l.bright_above)}. One per photo.`;
    case "problems":
      return `Shares of pure white and pure black pixels, and the spread of brightness. A photo can have several.`;
    case "focus":
      return `Detail in the sharpest part: soft below ${num(l.soft_below)}, blurry below ${num(l.blurry_below)}.`;
  }
}

/** The rule behind one bar, for its tooltip. */
export function barRule(card: Card, value: string, l: ClassSettings): string {
  const rules: Record<string, string> = {
    landscape: `wider than ${l.square_ratio}:1, narrower than ${l.panorama_ratio}:1`,
    portrait: `taller than 1:${l.square_ratio}, less than 1:${l.panorama_ratio}`,
    square: `width ÷ height within ${l.square_ratio}`,
    panorama: `${l.panorama_ratio}:1 or longer, either way`,
    "black & white": `mean chroma below ${l.mono_chroma} and no vivid patch`,
    muted: `no colour reaches ${pct(l.colour_mode === "Patch" ? l.patch_share : l.colour_share)}, nor ${pct(l.colour_floor)} for the biggest`,
    dark: `mean brightness below ${pct(l.dark_below)}`,
    medium: `mean brightness ${pct(l.dark_below)} to ${pct(l.bright_above)}`,
    bright: `mean brightness above ${pct(l.bright_above)}`,
    "clipped highlights": `more than ${pct(l.clipped_share)} of the pixels pure white`,
    "blocked shadows": `more than ${pct(l.blocked_share)} of the pixels pure black`,
    "low contrast": `brightness spread below ${l.flat_below}`,
    sharp: `detail ${num(l.soft_below)} or more`,
    soft: `detail ${num(l.blurry_below)} to ${num(l.soft_below)}`,
    blurry: `detail below ${num(l.blurry_below)}`,
  };
  return rules[value] ?? (card === "hue" ? colourRule(l).replace("a colour", value) : value);
}

/** How picks combine, said once under the charts. */
export const COMBINE_NOTE =
  "A photo can count in several bars of a card. Picks in one card: any of them (switch a card to “all” to ask for every one); picks in different cards: all of them.";
