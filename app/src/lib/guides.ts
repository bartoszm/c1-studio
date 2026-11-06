/** Composition guides drawn over the photo in the viewer. */
export type Guide = "off" | "thirds" | "golden" | "quarters" | "diagonals" | "triangles" | "spiral";

export const GUIDES: Guide[] = ["off", "thirds", "golden", "quarters", "diagonals", "triangles", "spiral"];

export const GUIDE_NAMES: Record<Guide, string> = {
  off: "No grid",
  thirds: "Thirds",
  golden: "Golden ratio",
  quarters: "Quarters",
  diagonals: "Diagonals",
  triangles: "Golden triangles",
  spiral: "Fibonacci spiral",
};

/** How many ways a guide can be turned (`o`): the spiral starts in any of four corners,
 *  the triangles lie along either diagonal; the rest look the same every way. */
export const TURNS: Record<Guide, number> = {
  off: 1, thirds: 1, golden: 1, quarters: 1, diagonals: 1, triangles: 2, spiral: 4,
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

/** A stored turn (0–3), or 0 for anything else. */
export function storedTurn(s: string | null): number {
  const n = Number(s);
  return Number.isInteger(n) && n >= 0 && n < 4 ? n : 0;
}

/** Turn 0 and 1 are the triangles' two diagonals; the spiral goes round the corners in
 *  this order (by turn bit: none, mirrored left to right, both, top to bottom). */
const SPIRAL_TURNS = [0, 1, 3, 2];

/** The turn after `turn` for guide `g`. The triangles flip only the low bit, so the
 *  spiral's top-to-bottom mirror stays. */
export function nextTurn(g: Guide, turn: number): number {
  if (TURNS[g] === 2) return turn ^ 1;
  if (TURNS[g] === 4) return SPIRAL_TURNS[(SPIRAL_TURNS.indexOf(turn) + 1) % 4];
  return turn;
}

/** A line from (x1, y1) to (x2, y2). */
export type Line = [number, number, number, number];

/** A quarter of an ellipse from (x1, y1) to (x2, y2), radii rx and ry, turning clockwise
 *  on screen (`cw`) or not. The spiral's quarter circles, stretched with the frame. */
export type Arc = { x1: number; y1: number; x2: number; y2: number; rx: number; ry: number; cw: boolean };

export const PHI = (1 + Math.sqrt(5)) / 2;

/** Lines across the frame at these fractions of its width and height. */
function split(w: number, h: number, at: number[]): Line[] {
  return [
    ...at.map((f): Line => [w * f, 0, w * f, h]),
    ...at.map((f): Line => [0, h * f, w, h * f]),
  ];
}

/** Where a perpendicular from (w, 0) and from (0, h) meets the diagonal (0, 0)–(w, h). */
function feet(w: number, h: number): [[number, number], [number, number]] {
  const l2 = w * w + h * h;
  return [[(w * w * w) / l2, (w * w * h) / l2], [(w * h * h) / l2, (h * h * h) / l2]];
}

/** The straight lines of guide `g` over a `w` × `h` frame, in its pixels, turned `turn`
 *  ways (see `TURNS`). Diagonals are the two corner-to-corner lines with a perpendicular
 *  to each from the other two corners; golden triangles are one diagonal with its two. */
export function guideLines(g: Guide, w: number, h: number, turn = 0): Line[] {
  if (!(w > 0 && h > 0)) return [];
  switch (g) {
    case "thirds":
      return split(w, h, [1 / 3, 2 / 3]);
    case "golden":
      return split(w, h, [1 - 1 / PHI, 1 / PHI]);
    case "quarters":
      return split(w, h, [1 / 4, 1 / 2, 3 / 4]);
    case "diagonals":
    case "triangles": {
      const [a, b] = feet(w, h);
      const down: Line[] = [[0, 0, w, h], [w, 0, a[0], a[1]], [0, h, b[0], b[1]]];
      // The same for the other diagonal, mirrored.
      const up: Line[] = [[0, h, w, 0], [0, 0, w - a[0], a[1]], [w, h, w - b[0], b[1]]];
      if (g === "diagonals") return [...down, ...up];
      return turn % 2 ? up : down;
    }
    case "spiral":
      return spiral(w, h, turn).lines;
    default:
      return [];
  }
}

/** How many squares the spiral is cut into; the tenth is under 1% of the frame's side. */
const SQUARES = 10;

/** The Fibonacci (golden) spiral over a `w` × `h` frame: the golden rectangle cut into
 *  squares (`lines`) and a quarter circle in each (`arcs`), stretched to the frame as
 *  editors do. On a landscape frame turn 0 starts at the bottom-left corner and winds in
 *  towards the right; turn bit 1 mirrors it left to right, bit 2 top to bottom. A
 *  portrait frame gets it mirrored across its diagonal (x and y swapped), the first cut
 *  across: turn 0 starts top-right, bit 1 mirrors top to bottom, bit 2 left to right. */
export function spiral(w: number, h: number, turn = 0): { lines: Line[]; arcs: Arc[] } {
  if (!(w > 0 && h > 0)) return { lines: [], arcs: [] };
  const tall = h > w;
  const [L, S] = tall ? [h, w] : [w, h];
  const sx = L / PHI;
  const sy = S;
  const flipX = (turn & 1) === 1;
  const flipY = (turn & 2) === 2;
  // Each mirror turns the winding the other way.
  const cw = [flipX, flipY, tall].filter(Boolean).length % 2 === 0;
  // From the golden rectangle (φ × 1) to the frame's pixels.
  const at = (ux: number, uy: number): [number, number] => {
    let x = ux * sx;
    let y = uy * sy;
    if (flipX) x = L - x;
    if (flipY) y = S - y;
    return tall ? [y, x] : [x, y];
  };
  const line = (ax: number, ay: number, bx: number, by: number): Line => [...at(ax, ay), ...at(bx, by)];

  const lines: Line[] = [];
  const arcs: Arc[] = [];
  // The rectangle still to cut, and a square taken from its left, top, right, bottom in turn.
  let [x, y, W, H] = [0, 0, PHI, 1];
  for (let i = 0; i < SQUARES; i++) {
    let from: [number, number];
    let to: [number, number];
    switch (i % 4) {
      case 0: { // left: from its bottom-left to its top-right
        const s = H;
        lines.push(line(x + s, y, x + s, y + H));
        from = [x, y + s]; to = [x + s, y];
        x += s; W -= s;
        break;
      }
      case 1: { // top: from its top-left to its bottom-right
        const s = W;
        lines.push(line(x, y + s, x + W, y + s));
        from = [x, y]; to = [x + s, y + s];
        y += s; H -= s;
        break;
      }
      case 2: { // right: from its top-right to its bottom-left
        const s = H;
        lines.push(line(x + W - s, y, x + W - s, y + H));
        from = [x + W, y]; to = [x + W - s, y + s];
        W -= s;
        break;
      }
      default: { // bottom: from its bottom-right to its top-left
        const s = W;
        lines.push(line(x, y + H - s, x + W, y + H - s));
        from = [x + W, y + H]; to = [x, y + H - s];
        H -= s;
        break;
      }
    }
    const s = Math.abs(to[0] - from[0]);
    const [x1, y1] = at(...from);
    const [x2, y2] = at(...to);
    const [rx, ry] = tall ? [s * sy, s * sx] : [s * sx, s * sy];
    arcs.push({ x1, y1, x2, y2, rx, ry, cw });
  }
  return { lines, arcs };
}

const r = (n: number) => Math.round(n * 10) / 10;

/** Guide `g` as one SVG path: every line and arc in a single element to draw. */
export function guidePath(g: Guide, w: number, h: number, turn = 0): string {
  const parts = guideLines(g, w, h, turn).map(([x1, y1, x2, y2]) => `M${r(x1)} ${r(y1)}L${r(x2)} ${r(y2)}`);
  if (g === "spiral") {
    const { arcs } = spiral(w, h, turn);
    if (arcs.length) {
      parts.push(`M${r(arcs[0].x1)} ${r(arcs[0].y1)}`);
      for (const a of arcs) parts.push(`A${r(a.rx)} ${r(a.ry)} 0 0 ${a.cw ? 1 : 0} ${r(a.x2)} ${r(a.y2)}`);
    }
  }
  return parts.join("");
}

/** Where the guides go over an image of `size` shown fitted into a `w` × `h` stage
 *  `inset` pixels in from each side (`.stage img`), in the stage's pixels. */
export function frameFor(size: [number, number] | null, w: number, h: number, inset: number) {
  if (!size) return null;
  const box = fitted(size[0], size[1], w - 2 * inset, h - 2 * inset);
  return box && { ...box, left: box.left + inset, top: box.top + inset };
}

/** Where an image `nw` × `nh` shows when fitted (object-fit: contain) into `w` × `h`. */
export function fitted(nw: number, nh: number, w: number, h: number) {
  if (!(nw > 0 && nh > 0 && w > 0 && h > 0)) return null;
  const s = Math.min(w / nw, h / nh);
  const fw = nw * s;
  const fh = nh * s;
  return { left: (w - fw) / 2, top: (h - fh) / 2, width: fw, height: fh };
}
