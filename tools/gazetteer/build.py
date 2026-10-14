"""Builds crates/c1geo/data/gazetteer.tsv.gz from a GeoNames dump.

Run it once when the data should be refreshed:

    python tools/gazetteer/build.py <dir with the GeoNames files>

The directory holds, from https://download.geonames.org/export/dump/:
cities1000.zip, countryInfo.txt, admin1CodesASCII.txt and alternateNamesV2.zip.

GeoNames data is CC BY 4.0 (https://www.geonames.org/).

The output is gzipped text, one place per line, tab-separated:

    C  code             lat lon names...            a country
    A  code.admin1      lat lon names...            a state, province, region
    P  code admin1  pop lat lon names...            a city, town or village
    D  code admin1  pop lat lon names...            a part of a bigger city near it

A `D` is found by its name but never stands for a position: a point in Kraków is in
Kraków, not in Grzegórzki, a district GeoNames lists as a place of its own.

Names are the place's own name first, then the other names it goes by, in the Latin
script only (the catalog is typed on a Latin keyboard), each once after folding case and
diacritics. A country's and a state's point is the population-weighted median of its
cities: inland where people live, not the middle of a bounding box.
"""

import gzip
import io
import math
import sys
import unicodedata
import zipfile
from collections import defaultdict
from pathlib import Path

# Pseudo-languages GeoNames uses for codes and links, not names.
NOT_NAMES = {"link", "post", "iata", "icao", "faac", "abbr", "wkdt", "unlc", "fr_1793", "tcid", "uicn"}

# Cities smaller than this keep only their own name.
ALT_POPULATION = 15000


def fold(name: str) -> str:
    text = unicodedata.normalize("NFKD", name.casefold())
    text = "".join(c for c in text if not unicodedata.combining(c))
    return text.replace("ł", "l").replace("ø", "o").replace("đ", "d").replace("ı", "i").replace("æ", "ae")


def latin(name: str) -> bool:
    for c in name:
        if c.isalpha() and "LATIN" not in unicodedata.name(c, ""):
            return False
    return any(c.isalpha() for c in name) and not any(c.isdigit() for c in name)


def names(first: list[str], more: list[str], codes: bool = True) -> list[str]:
    """`codes`: keep short upper-case names (USA, UK); a city's are airport codes (KRK)."""
    seen, out = set(), []
    for n in first + more:
        n = n.strip()
        # Airport and other codes (KRK) are not names.
        if not n or "\t" in n or not latin(n) or (n.isupper() and len(n) <= 4):
            continue
        key = fold(n)
        if key not in seen:
            seen.add(key)
            out.append(n)
    return out


def radius_km(pop: int) -> float:
    """How far a place of this population reaches, roughly: Kraków (800k) about 24 km."""
    return 1 + math.sqrt(pop) / 40


def km(a: dict, b: dict) -> float:
    dlat = math.radians(b["lat"] - a["lat"])
    dlon = math.radians(b["lon"] - a["lon"]) * math.cos(math.radians((a["lat"] + b["lat"]) / 2))
    return 6371 * math.hypot(dlat, dlon)


def mark_parts(cities: list[dict]) -> None:
    """Marks the places that lie well inside a city ten times as big."""
    by_country = defaultdict(list)
    for c in cities:
        by_country[c["code"]].append(c)
    for members in by_country.values():
        big = [c for c in members if c["pop"] >= 100_000]
        for c in members:
            c["part"] = any(
                b["pop"] >= 10 * c["pop"] and b is not c and km(b, c) < radius_km(b["pop"]) / 2 for b in big
            )


def weighted_median(points: list[tuple[float, float, int]]) -> tuple[float, float]:
    def median(values: list[tuple[float, int]]) -> float:
        values.sort()
        half = sum(w for _, w in values) / 2
        acc = 0
        for v, w in values:
            acc += w
            if acc >= half:
                return v
        return values[-1][0]

    return median([(lat, max(pop, 1)) for lat, _, pop in points]), median([(lon, max(pop, 1)) for _, lon, pop in points])


def main(src: Path, out: Path) -> None:
    countries = {}  # code -> (geonameid, name, capital)
    for line in (src / "countryInfo.txt").read_text(encoding="utf-8").splitlines():
        if line.startswith("#") or not line.strip():
            continue
        f = line.split("\t")
        countries[f[0]] = (f[16], f[4], f[5])
    admins = {}  # "PL.77" -> (geonameid, name, asciiname)
    for line in (src / "admin1CodesASCII.txt").read_text(encoding="utf-8").splitlines():
        f = line.split("\t")
        admins[f[0]] = (f[3], f[1], f[2])

    wanted = {gid for gid, _, _ in countries.values()} | {gid for gid, _, _ in admins.values()}
    alternate = defaultdict(list)
    with zipfile.ZipFile(src / "alternateNamesV2.zip") as z:
        with z.open("alternateNamesV2.txt") as raw:
            for line in io.TextIOWrapper(raw, encoding="utf-8"):
                f = line.rstrip("\n").split("\t")
                if f[1] in wanted and f[2] not in NOT_NAMES and f[7] != "1":
                    alternate[f[1]].append(f[3])

    cities = []
    with zipfile.ZipFile(src / "cities1000.zip") as z:
        with z.open("cities1000.txt") as raw:
            for line in io.TextIOWrapper(raw, encoding="utf-8"):
                f = line.rstrip("\n").split("\t")
                # Populated places only; PPLX (parts of a city) would compete with the city.
                if f[7] == "PPLX" or f[7] == "PPLH":
                    continue
                cities.append(
                    {
                        "name": f[1],
                        "ascii": f[2],
                        "alt": f[3].split(",") if f[3] else [],
                        "lat": float(f[4]),
                        "lon": float(f[5]),
                        "code": f[8],
                        "admin1": f[10],
                        "pop": int(f[14] or 0),
                    }
                )

    mark_parts(cities)
    by_country = defaultdict(list)
    by_admin = defaultdict(list)
    for c in cities:
        by_country[c["code"]].append((c["lat"], c["lon"], c["pop"]))
        by_admin[f"{c['code']}.{c['admin1']}"].append((c["lat"], c["lon"], c["pop"]))

    lines = []
    for code, (gid, name, _) in sorted(countries.items()):
        if not by_country[code]:
            continue
        lat, lon = weighted_median(by_country[code])
        lines.append("\t".join(["C", code, f"{lat:.4f}", f"{lon:.4f}", *names([name], alternate[gid])]))
    for key, (gid, name, ascii_name) in sorted(admins.items()):
        if not by_admin[key]:
            continue
        lat, lon = weighted_median(by_admin[key])
        lines.append("\t".join(["A", key, f"{lat:.4f}", f"{lon:.4f}", *names([name, ascii_name], alternate[gid])]))
    for c in sorted(cities, key=lambda c: (c["code"], c["admin1"], -c["pop"], c["name"])):
        # Other names only for bigger places: those are the ones known abroad by another
        # name (Cracow, Tokio); for a village they are mostly transliterations.
        n = names([c["name"], c["ascii"]], c["alt"] if c["pop"] >= ALT_POPULATION else [], codes=False)
        if not n:
            continue
        lines.append(
            "\t".join(["D" if c["part"] else "P", c["code"], c["admin1"], str(c["pop"]), f"{c['lat']:.4f}", f"{c['lon']:.4f}", *n])
        )

    data = ("\n".join(lines) + "\n").encode("utf-8")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(gzip.compress(data, 9, mtime=0))
    print(f"{len(lines)} places, {len(data) / 1e6:.1f} MB, {out.stat().st_size / 1e6:.1f} MB gzipped -> {out}")


if __name__ == "__main__":
    here = Path(__file__).resolve().parents[2]
    main(Path(sys.argv[1]), here / "crates" / "c1geo" / "data" / "gazetteer.tsv.gz")
