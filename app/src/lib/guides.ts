/** Composition guides drawn over the photo in the viewer. */
export type Guide = "off" | "thirds" | "golden" | "quarters" | "diagonals";

export const GUIDES: Guide[] = ["off", "thirds", "golden", "quarters", "diagonals"];

export const GUIDE_NAMES: Record<Guide, string> = {
  off: "No grid",
  thirds: "Thirds",
  golden: "Golden ratio",
  quarters: "Quarters",
  diagonals: "Diagonals",
};

/** The guide after (or, `back`, before) `g`, round the list. */
export function nextGuide(g: Guide, back = false): Guide {
  const i = GUIDES.indexOf(g);
  const n = GUIDES.length;
  return GUIDES[(((i < 0 ? 0 : i) + (back ? -1 : 1)) % n + n) % n];
}

/** A stored guide, or none for anything else. */
export function storedGuide(s: string | null): Guide {
  return GUIDES.includes(s as Guide) ? (s as Guide) : "off";
}

/** A line from (x1, y1) to (x2, y2). */
export type Line = [number, number, number, number];

const PHI = (1 + Math.sqrt(5)) / 2;

/** Lines across the frame at these fractions of its width and height. */
function split(w: number, h: number, at: number[]): Line[] {
  return [
    ...at.map((f): Line => [w * f, 0, w * f, h]),
    ...at.map((f): Line => [0, h * f, w, h * f]),
  ];
}

/** The lines of guide `g` over a `w` × `h` frame, in its pixels. Diagonals are the two
 *  corner-to-corner lines with a perpendicular to each from the other two corners, so
 *  they follow the frame's shape. */
export function guideLines(g: Guide, w: number, h: number): Line[] {
  switch (g) {
    case "thirds":
      return split(w, h, [1 / 3, 2 / 3]);
    case "golden":
      return split(w, h, [1 - 1 / PHI, 1 / PHI]);
    case "quarters":
      return split(w, h, [1 / 4, 1 / 2, 3 / 4]);
    case "diagonals": {
      const l2 = w * w + h * h;
      if (!l2) return [];
      // Where a perpendicular from (w, 0) and from (0, h) meets the diagonal (0, 0)–(w, h).
      const a: [number, number] = [(w * w * w) / l2, (w * w * h) / l2];
      const b: [number, number] = [(w * h * h) / l2, (h * h * h) / l2];
      return [
        [0, 0, w, h],
        [0, h, w, 0],
        [w, 0, a[0], a[1]],
        [0, h, b[0], b[1]],
        // The same for the other diagonal, mirrored.
        [0, 0, w - a[0], a[1]],
        [w, h, w - b[0], b[1]],
      ];
    }
    default:
      return [];
  }
}

/** Where an image `nw` × `nh` shows when fitted (object-fit: contain) into `w` × `h`. */
export function fitted(nw: number, nh: number, w: number, h: number) {
  if (!(nw > 0 && nh > 0 && w > 0 && h > 0)) return null;
  const s = Math.min(w / nw, h / nh);
  const fw = nw * s;
  const fh = nh * s;
  return { left: (w - fw) / 2, top: (h - fh) / 2, width: fw, height: fh };
}
