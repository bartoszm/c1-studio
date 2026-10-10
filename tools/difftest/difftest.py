"""Differential test: the same selector flags through the Rust c1cat and the Python
oracle (legacy/c1cat-py, run in WSL), comparing the variant ids each returns.

    cargo build --release -p c1cat
    python tools/difftest/difftest.py [-k NAME] [--catalog PATH] [--rust EXE]

Both sides read the same catalog: Rust through its library (synced first), Python through
its mirror (rebuilt by the oracle if stale). Exits 1 if any case differs, apart from
the ones expected to differ by design.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ORACLE = "/mnt/c" + str(Path(__file__).resolve().with_name("oracle.py"))[2:].replace("\\", "/")
PYTHON_WSL = "/home/bartoszm/miniconda3/bin/python3"

# Cases expected to differ, and how. Python's GPS parser drops degrees-and-minutes
# coordinates (`50,2.2566833N`), which Rust reads, so Rust finds more with GPS.
EXPECTED = {
    "gps has": "rust-superset",
    "gps none": "rust-subset",
    "near krakow": "rust-superset",
}

# Python's comparison regex takes whole numbers only (`<=2.8` is "cannot parse"), so
# these cases go to Python in an equivalent spelling it accepts.
PYTHON_SPELLING = {
    "rating >2.5": ["--rating", "3-5"],
    "aperture <=2.8": ["--aperture", "0-2.8"],
}

CASES = {
    "everything": [],
    "rating >=4": ["--rating", ">=4"],
    "rating 3": ["--rating", "3"],
    "rating 2-4": ["--rating", "2-4"],
    "rating >2.5": ["--rating", ">2.5"],
    "rating none": ["--rating", "none"],
    "rating any": ["--rating", "any"],
    "color green": ["--color", "green"],
    "color red,blue": ["--color", "red,blue"],
    "color none": ["--color", "none"],
    "color any": ["--color", "any"],
    "keyword street": ["--keyword", "street"],
    "keyword taxi path": ["--keyword", "transport|taxi"],
    "keyword street+night": ["--keyword", "street", "--keyword", "night"],
    "keyword-any": ["--keyword-any", "night,private"],
    "keyword-under transport": ["--keyword-under", "transport"],
    "no-keyword street": ["--no-keyword", "street"],
    "unkeyworded": ["--unkeyworded"],
    "camera x100": ["--camera", "x100"],
    "camera GR": ["--camera", "GR"],
    "lens 23mm": ["--lens", "23mm"],
    "lens f/2": ["--lens", "f/2"],
    "iso 400": ["--iso", "400"],
    "iso >=3200": ["--iso", ">=3200"],
    "iso 100-800": ["--iso", "100-800"],
    "aperture <=2.8": ["--aperture", "<=2.8"],
    "aperture 1.4-2": ["--aperture", "1.4-2"],
    "focal 23": ["--focal", "23"],
    "focal 16-55": ["--focal", "16-55"],
    "date Q1 2026": ["--from", "2026-01-01", "--to", "2026-03-31"],
    "from 2025-09": ["--from", "2025-09"],
    "to 2018": ["--to", "2018"],
    "year 2025": ["--year", "2025"],
    "month 2026-07": ["--month", "2026-07"],
    "imported from 2026": ["--imported-from", "2026-01-01"],
    "imported to 2020": ["--imported-to", "2020-12-31"],
    "album NYC": ["--album", "NYC"],
    "album fujip2": ["--album", "fujip2"],
    "not-in-album fujip2": ["--not-in-album", "fujip2"],
    "city hong": ["--city", "hong"],
    "city krak": ["--city", "krak"],
    "city kraków": ["--city", "kraków"],
    "country japan": ["--country", "japan"],
    "state sicily": ["--state", "sicily"],
    "text street": ["--text", "street"],
    "text taxi": ["--text", "taxi"],
    "text hong kong": ["--text", "hong kong"],
    "text phrase": ["--text", '"new york"'],
    "text OR": ["--text", "tokyo OR kraków"],
    "text folded": ["--text", "krakow"],
    "edited": ["--edited"],
    "unedited": ["--unedited"],
    "style bnw": ["--style", "bnw"],
    "local layers": ["--has-local-layers"],
    "include trashed": ["--include-trashed"],
    "only trashed": ["--only-trashed"],
    "variant 0": ["--variant-index", "0"],
    "variant 1": ["--variant-index", "1"],
    "ext dng": ["--ext", "dng"],
    "ext raf,jpg": ["--ext", "raf,jpg"],
    "filename DSCF1*": ["--filename", "DSCF1*.RAF"],
    "filename R00*": ["--filename", "R00*"],
    "gps has": ["--gps", "has"],
    "gps none": ["--gps", "none"],
    "near krakow": ["--near", "50.06,19.94,30"],
    "combo street 2026 >=3": ["--year", "2026", "--rating", ">=3", "--keyword", "street"],
    "combo x100 japan edited": ["--camera", "x100", "--country", "japan", "--edited"],
    "combo text year color": ["--text", "street", "--year", "2025", "--color", "any"],
    "combo tree album rating": ["--keyword-under", "transport", "--not-in-album", "fujip2", "--rating", "none"],
}


def default_catalog() -> str:
    config = Path(os.environ["APPDATA"]) / "c1studio" / "config.json"
    return json.loads(config.read_text(encoding="utf-8"))["catalog"]


def to_wsl(path: str) -> str:
    m = re.match(r"^([A-Za-z]):[\\/](.*)$", path)
    if not m:
        raise SystemExit(f"cannot map {path} into WSL")
    return f"/mnt/{m.group(1).lower()}/" + m.group(2).replace("\\", "/")


def run_rust(exe: str, catalog: str, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run([exe, "--catalog", catalog, *args], capture_output=True, text=True, encoding="utf-8")


def rust_ids(exe: str, catalog: str, args: list) -> list | dict:
    done = run_rust(exe, catalog, "find", "--no-sync", "--ids", "--sort", "id", *args)
    if done.returncode != 0:
        return {"error": done.stderr.strip()}
    return sorted(done.stdout.split())


def anchored_cases(exe: str, catalog: str) -> dict:
    """Cases built from real photos: ids, image ids, folders and exact filenames."""
    done = run_rust(exe, catalog, "find", "--no-sync", "--json", "--rating", "5", "--limit", "2", "--sort", "id")
    photos = json.loads(done.stdout) if done.returncode == 0 else []
    if len(photos) < 2:
        print("warning: no 5-star photos to anchor id/folder cases on", file=sys.stderr)
        return {}
    a, b = photos
    folder = str(Path(a["file"]["path"]).parent)
    return {
        "id one": ["--id", a["id"]["ext"]],
        "id two": ["--id", a["id"]["ext"], "--id", b["id"]["ext"]],
        "image-id": ["--image-id", a["asset"]],
        "folder": ["--folder", folder],
        "folder parent": ["--folder", str(Path(folder).parent)],
        "filename exact": ["--filename", a["file"]["name"]],
    }


def compare(name: str, rust, python) -> tuple[str, str]:
    """Status and a detail line."""
    if isinstance(rust, dict) or isinstance(python, dict):
        detail = f"rust: {rust.get('error') if isinstance(rust, dict) else len(rust)}; python: {python.get('error') if isinstance(python, dict) else len(python)}"
        both_failed = isinstance(rust, dict) and isinstance(python, dict)
        return ("ok" if both_failed else "DIFF"), detail
    r, p = set(rust), set(python)
    if r == p:
        return "ok", ""
    only_r, only_p = sorted(r - p, key=int), sorted(p - r, key=int)
    detail = f"only rust {len(only_r)} {only_r[:5]}  only python {len(only_p)} {only_p[:5]}"
    shape = "rust-superset" if not only_p else "rust-subset" if not only_r else "mixed"
    if EXPECTED.get(name) == shape:
        return "expected", detail
    return "DIFF", detail


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--catalog", default=None)
    parser.add_argument("--rust", default=str(ROOT / "target" / "release" / "c1cat.exe"))
    parser.add_argument("-k", dest="only", help="run only cases whose name contains this")
    args = parser.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    catalog = args.catalog or default_catalog()

    synced = run_rust(args.rust, catalog, "sync")
    if synced.returncode != 0:
        print(synced.stderr, file=sys.stderr)
        return 2
    cases = {**CASES, **anchored_cases(args.rust, catalog)}
    if args.only:
        cases = {k: v for k, v in cases.items() if args.only in k}

    payload = json.dumps([{"name": k, "args": PYTHON_SPELLING.get(k, v)} for k, v in cases.items()])
    oracle = subprocess.run(
        ["wsl.exe", "-d", "Ubuntu", "-e", PYTHON_WSL, ORACLE, to_wsl(catalog)],
        input=payload, capture_output=True, text=True, encoding="utf-8",
    )
    if oracle.returncode != 0:
        print(oracle.stderr, file=sys.stderr)
        return 2
    try:
        python = json.loads(oracle.stdout)
    except json.JSONDecodeError:
        print("oracle printed something other than JSON:", oracle.stdout[:2000], oracle.stderr[-2000:], sep="\n", file=sys.stderr)
        return 2

    failed = 0
    width = max(map(len, cases))
    for name, case_args in cases.items():
        rust = rust_ids(args.rust, catalog, case_args)
        status, detail = compare(name, rust, python[name])
        if name in PYTHON_SPELLING and status == "ok":
            detail = f"python asked as {' '.join(PYTHON_SPELLING[name])}"
        failed += status == "DIFF"
        count = len(rust) if isinstance(rust, list) else "err"
        print(f"{status:<8} {name:<{width}} {count:>7}  {detail}")
    print(f"\n{len(cases)} cases, {failed} unexpected difference(s)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
