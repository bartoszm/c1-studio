// Countries and their ISO 3166-1 alpha-2 codes, so typing a country fills its code.
// Names come from the browser's own region names (ICU) in English, Polish and the
// user's language, plus the everyday names those miss (USA, UK, England, Hong Kong…).

const ALIASES: Record<string, string> = {
  usa: "US", "u.s.a.": "US", us: "US", america: "US", "united states of america": "US",
  uk: "GB", "u.k.": "GB", britain: "GB", "great britain": "GB", england: "GB", scotland: "GB",
  wales: "GB", "northern ireland": "GB", anglia: "GB", szkocja: "GB", walia: "GB",
  "hong kong": "HK", hongkong: "HK", macau: "MO", macao: "MO",
  holland: "NL", holandia: "NL", korea: "KR", "south korea": "KR", "north korea": "KP",
  russia: "RU", czechia: "CZ", "czech republic": "CZ", turkey: "TR", turkiye: "TR",
  vietnam: "VN", taiwan: "TW", burma: "MM", "ivory coast": "CI", uae: "AE", vatican: "VA",
};

// Groups and placeholders the browser also names, which are not countries, and codes
// ISO has retired (the browser still calls DD "Germany").
const NOT_COUNTRIES = new Set([
  "EU", "EZ", "UN", "QO", "ZZ",
  "AN", "BU", "CS", "CT", "DD", "DY", "FX", "HV", "JT", "MI", "NH", "NQ", "NT", "PC", "PU", "PZ", "RH", "SU",
  "TP", "VD", "WK", "YD", "YU", "ZR",
]);

/** Lower case, no accents, no punctuation runs: `Hong-Kong` and `hong kong` match, and
 *  so do `Kraków` and `krakow`. */
export function looseKey(name: string): string {
  return name
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase()
    .replace(/[-_]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

let byName: Map<string, string> | null = null;

function names(): Map<string, string> {
  if (byName) return byName;
  byName = new Map();
  const locales = [...new Set(["en", "pl", navigator.language])];
  const shown = locales.map((l) => {
    try {
      return new Intl.DisplayNames([l], { type: "region", fallback: "none" });
    } catch {
      return null;
    }
  });
  const A = "A".charCodeAt(0);
  for (let i = 0; i < 26; i++) {
    for (let j = 0; j < 26; j++) {
      const code = String.fromCharCode(A + i, A + j);
      if (NOT_COUNTRIES.has(code) || (code.startsWith("X") && code !== "XK")) continue;
      for (const dn of shown) {
        let name: string | undefined;
        try {
          name = dn?.of(code);
        } catch {
          name = undefined;
        }
        if (name && name !== code && !byName.has(looseKey(name))) byName.set(looseKey(name), code);
      }
    }
  }
  for (const [alias, code] of Object.entries(ALIASES)) byName.set(looseKey(alias), code);
  return byName;
}

/** The ISO code for a country name in English, Polish or the user's language, or null. */
export function codeFor(country: string): string | null {
  const k = looseKey(country);
  if (!k) return null;
  return names().get(k) ?? null;
}

/** The English name for an ISO code, or null if it is not one. */
export function nameFor(code: string): string | null {
  const c = code.trim().toUpperCase();
  if (!/^[A-Z]{2}$/.test(c) || NOT_COUNTRIES.has(c)) return null;
  if (c === "HK") return "Hong Kong";
  if (c === "MO") return "Macau";
  try {
    const name = new Intl.DisplayNames(["en"], { type: "region", fallback: "none" }).of(c);
    return name && name !== c ? name : null;
  } catch {
    return null;
  }
}
