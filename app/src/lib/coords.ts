// Coordinates as people type or paste them: `50.0614, 19.9366`, `33.8688S 151.2093E`,
// `44°58′N 14°45′E`, `N44°58' E14°45'`, `44 58 N 14 45 E`, `50°3'41.0"N 19°56'12"E`.
// The same forms `c1geo::parse_point` reads.

type Unit = "deg" | "min" | "sec" | "bare";
type Num = { value: number; unit: Unit; negative: boolean };
type Token = { num: Num } | { letter: string } | "sep";

const MINUTE_MARKS = "'′’ʹ‘";
const SECOND_MARKS = "\"″”“ʺ";
const DEGREE_MARKS = "°º˚";

function tokens(text: string): Token[] | null {
  const out: Token[] = [];
  const chars = [...text];
  let i = 0;
  while (i < chars.length) {
    const c = chars[i];
    if (/\s/.test(c)) {
      i++;
    } else if (c === "," || c === ";" || c === "/") {
      out.push("sep");
      i++;
    } else if ("NSEWnsew".includes(c)) {
      out.push({ letter: c.toUpperCase() });
      i++;
    } else if (/[0-9+\-.]/.test(c)) {
      const negative = c === "-";
      const start = c === "-" || c === "+" ? i + 1 : i;
      let end = start;
      while (end < chars.length && /[0-9.]/.test(chars[end])) end++;
      const digits = chars.slice(start, end).join("");
      if (!/^\d*\.?\d+$|^\d+\.$/.test(digits)) return null;
      i = end;
      while (i < chars.length && /\s/.test(chars[i])) i++;
      let unit: Unit = "bare";
      const m = chars[i];
      if (m !== undefined && DEGREE_MARKS.includes(m)) unit = "deg";
      else if (m !== undefined && MINUTE_MARKS.includes(m)) {
        if (chars[i + 1] === "'") {
          i++;
          unit = "sec";
        } else unit = "min";
      } else if (m !== undefined && SECOND_MARKS.includes(m)) unit = "sec";
      if (unit !== "bare") i++;
      out.push({ num: { value: +digits, unit, negative } });
    } else {
      return null;
    }
  }
  return out;
}

/** One coordinate from its numbers: degrees, then minutes and seconds if given. */
function value(nums: Num[]): number | null {
  let d: number, m = 0, s = 0;
  if (nums.length === 1) d = nums[0].value;
  else if (nums.length === 2 && nums[1].unit !== "sec") [d, m] = [nums[0].value, nums[1].value];
  else if (nums.length === 3) [d, m, s] = nums.map((n) => n.value);
  else return null;
  if (nums.length > 1 && (!Number.isInteger(d) || m >= 60 || s >= 60)) return null;
  if (nums.length > 2 && !Number.isInteger(m)) return null;
  const v = d + m / 60 + s / 3600;
  return nums[0].negative ? -v : v;
}

const numbers = (list: Token[]): Num[] => list.flatMap((t) => (typeof t === "object" && "num" in t ? [t.num] : []));

/** A typed position as [lat, lon], or what is wrong with it. */
export function parsePoint(text: string): [number, number] | string {
  const HOW = "Type latitude and longitude, e.g. 50.0614, 19.9366 or 44°58′N 14°45′E.";
  const list = tokens(text.trim());
  if (!list) return HOW;
  const letters = list.flatMap((t, i) => (typeof t === "object" && "letter" in t ? [[i, t.letter] as const] : []));
  let groups: [Num[], string | null][];
  if (letters.length === 0) {
    const plain: Num[][] = [[]];
    for (const t of list) {
      if (t === "sep") plain.push([]);
      else if ("num" in t) {
        const current = plain[plain.length - 1];
        // A new degrees value starts the next coordinate: `44°58' 14°45'`.
        if (t.num.unit === "deg" && current.length && plain.length === 1) plain.push([t.num]);
        else current.push(t.num);
      }
    }
    let kept = plain.filter((g) => g.length);
    if (kept.length === 1 && kept[0].every((n) => n.unit === "bare") && kept[0].length % 2 === 0) {
      // `44 58 14 45`: halves.
      const half = kept[0].length / 2;
      kept = [kept[0].slice(0, half), kept[0].slice(half)];
    }
    groups = kept.map((g) => [g, null]);
  } else if (letters.length === 2) {
    const [[a, la], [b, lb]] = letters;
    if (list.slice(0, a).every((t) => t === "sep")) {
      // Letters first: `N44°58' E14°45'`.
      groups = [[numbers(list.slice(a + 1, b)), la], [numbers(list.slice(b + 1)), lb]];
    } else {
      if (numbers(list.slice(b + 1)).length) return HOW;
      groups = [[numbers(list.slice(0, a)), la], [numbers(list.slice(a + 1, b)), lb]];
    }
  } else return HOW;
  if (groups.length !== 2) return HOW;
  const [[first, l1], [second, l2]] = groups;
  let lat = value(first);
  let lon = value(second);
  if (lat === null || lon === null) return HOW;
  const isLat = (l: string | null) => l === null || l === "N" || l === "S";
  const isLon = (l: string | null) => l === null || l === "E" || l === "W";
  let [latLetter, lonLetter] = [l1, l2];
  if (!(isLat(l1) && isLon(l2))) {
    // `14°45′E 44°58′N`: longitude first.
    if (l1 !== null && isLon(l1) && isLat(l2)) {
      [lat, lon] = [lon, lat];
      [latLetter, lonLetter] = [l2, l1];
    } else return HOW;
  }
  // A letter says the side: `-33.8688S` is south once, not twice.
  if (latLetter) lat = latLetter === "S" ? -Math.abs(lat) : Math.abs(lat);
  if (lonLetter) lon = lonLetter === "W" ? -Math.abs(lon) : Math.abs(lon);
  if (!Number.isFinite(lat) || !Number.isFinite(lon) || Math.abs(lat) > 90 || Math.abs(lon) > 180)
    return "Latitude is -90 to 90, longitude -180 to 180.";
  return [lat, lon];
}
