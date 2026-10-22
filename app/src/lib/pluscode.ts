// Google's Plus Codes (Open Location Code): `8FVC9G8F+6X` is a spot of about 14 × 14 m
// anywhere; `9G8F+6X` the same spot near a known place (the photo, the map, a town).
// The algorithm of github.com/google/open-location-code, as far as positions need it.

const ALPHABET = "23456789CFGHJMPQRVWX";
const SEPARATOR_AT = 8;
const PAIR_LENGTH = 10;
const PAIR_PRECISION = 8000;
const PAIR_FIRST_PLACE = 160_000;
const GRID_ROWS = 5;
const GRID_COLUMNS = 4;
const FINAL_LAT_PRECISION = PAIR_PRECISION * GRID_ROWS ** 5;
const FINAL_LON_PRECISION = PAIR_PRECISION * GRID_COLUMNS ** 5;

export type Kind = "full" | "short";

/** Whether `code` is a plus code, and which kind: a full one stands alone, a short one
 *  needs a place near it. */
export function kindOf(code: string): Kind | null {
  const c = code.toUpperCase();
  const sep = c.indexOf("+");
  if (sep < 0 || sep !== c.lastIndexOf("+") || sep > SEPARATOR_AT || sep % 2 === 1 || sep < 2) return null;
  const head = c.slice(0, sep);
  const tail = c.slice(sep + 1);
  if (tail.length === 1 || [...tail].some((ch) => !ALPHABET.includes(ch))) return null;
  const pad = head.indexOf("0");
  if (pad >= 0) {
    // Padding: only in a full code, from an even place to the separator, nothing after.
    if (sep !== SEPARATOR_AT || pad === 0 || pad % 2 === 1 || !/^0+$/.test(head.slice(pad)) || tail) return null;
  }
  if ([...head.slice(0, pad >= 0 ? pad : sep)].some((ch) => !ALPHABET.includes(ch))) return null;
  if (sep < SEPARATOR_AT) return "short";
  // The first pair is 20° steps: at most 180° of latitude and 360° of longitude.
  if (ALPHABET.indexOf(c[0]) * 20 >= 180 || ALPHABET.indexOf(c[1]) * 20 >= 360) return null;
  return "full";
}

/** The middle of a full code's area. */
export function decode(code: string): [number, number] {
  const c = code.toUpperCase().replace("+", "").replace(/0+$/, "");
  let lat = -90 * PAIR_PRECISION;
  let lon = -180 * PAIR_PRECISION;
  let place = PAIR_FIRST_PLACE;
  const pairs = Math.min(c.length, PAIR_LENGTH);
  for (let i = 0; i < pairs; i += 2) {
    lat += ALPHABET.indexOf(c[i]) * place;
    lon += ALPHABET.indexOf(c[i + 1]) * place;
    if (i < pairs - 2) place /= 20;
  }
  let latSize = place / PAIR_PRECISION;
  let lonSize = place / PAIR_PRECISION;
  let gridLat = 0;
  let gridLon = 0;
  if (c.length > PAIR_LENGTH) {
    let row = GRID_ROWS ** 4;
    let column = GRID_COLUMNS ** 4;
    const end = Math.min(c.length, 15);
    for (let i = PAIR_LENGTH; i < end; i++) {
      const d = ALPHABET.indexOf(c[i]);
      gridLat += Math.floor(d / GRID_COLUMNS) * row;
      gridLon += (d % GRID_COLUMNS) * column;
      if (i < end - 1) {
        row /= GRID_ROWS;
        column /= GRID_COLUMNS;
      }
    }
    latSize = row / FINAL_LAT_PRECISION;
    lonSize = column / FINAL_LON_PRECISION;
  }
  const south = lat / PAIR_PRECISION + gridLat / FINAL_LAT_PRECISION;
  const west = lon / PAIR_PRECISION + gridLon / FINAL_LON_PRECISION;
  return [Math.min(90, south + latSize / 2), Math.min(180, west + lonSize / 2)];
}

/** The 10-digit code of a point (`8FVC9G8F+6X`). */
export function encode(lat: number, lon: number): string {
  const span = 360 * FINAL_LON_PRECISION;
  let latValue = Math.floor(lat * FINAL_LAT_PRECISION) + 90 * FINAL_LAT_PRECISION;
  latValue = Math.max(0, Math.min(180 * FINAL_LAT_PRECISION - 1, latValue));
  let lonValue = Math.floor(lon * FINAL_LON_PRECISION) + 180 * FINAL_LON_PRECISION;
  lonValue = ((lonValue % span) + span) % span;
  latValue = Math.floor(latValue / GRID_ROWS ** 5);
  lonValue = Math.floor(lonValue / GRID_COLUMNS ** 5);
  let code = "";
  for (let i = 0; i < PAIR_LENGTH / 2; i++) {
    code = ALPHABET[latValue % 20] + ALPHABET[lonValue % 20] + code;
    latValue = Math.floor(latValue / 20);
    lonValue = Math.floor(lonValue / 20);
  }
  return `${code.slice(0, SEPARATOR_AT)}+${code.slice(SEPARATOR_AT)}`;
}

/** A short code's spot nearest `near`: the area its missing digits leave, centred there. */
export function recoverNearest(short: string, near: [number, number]): [number, number] {
  const [refLat, refLon] = [Math.max(-90, Math.min(90, near[0])), ((((near[1] + 180) % 360) + 360) % 360) - 180];
  const missing = SEPARATOR_AT - short.indexOf("+");
  const size = 20 ** (2 - missing / 2);
  let [lat, lon] = decode(encode(refLat, refLon).slice(0, missing) + short.toUpperCase());
  if (refLat + size / 2 < lat && lat - size >= -90) lat -= size;
  else if (refLat - size / 2 > lat && lat + size <= 90) lat += size;
  if (refLon + size / 2 < lon) lon -= size;
  else if (refLon - size / 2 > lon) lon += size;
  return [lat, ((((lon + 180) % 360) + 360) % 360) - 180];
}

export type Typed = { full: [number, number] } | { short: string; locality: string };

/** A typed plus code: `8FVC9G8F+6X`, `9G8F+6X`, `9G8F+6X Zürich, Switzerland` (as Google
 *  Maps gives them). Null: not a plus code. */
export function parsePlusCode(text: string): Typed | null {
  const m = /^\s*([0-9A-Za-z]+\+[0-9A-Za-z]*)(?:[\s,]+(.*))?$/.exec(text);
  if (!m) return null;
  const kind = kindOf(m[1]);
  if (kind === "full") return { full: decode(m[1]) };
  if (kind === "short") return { short: m[1].toUpperCase(), locality: (m[2] ?? "").trim() };
  return null;
}
