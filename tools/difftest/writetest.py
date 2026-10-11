"""Differential test of writes: the same edits through the Rust c1cat and the Python
oracle (legacy/c1cat-py, run in WSL), each on its own copy of the catalog, then a
row-by-row comparison of the two copies.

    cargo build --release -p c1cat
    python tools/difftest/writetest.py --work DIR [--catalog PATH] [--rust EXE]

Never touches the catalog itself: it is only read, once, to make the two copies. Needs
about three times the catalog's size free in DIR (two copies and Rust's backup), which
is removed afterwards unless --keep is given. Exits 1 if the copies differ.
"""

import argparse
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PYTHON_WSL = "/home/bartoszm/miniconda3/bin/python3"
LEGACY_WSL = "/mnt/c" + str(ROOT / "legacy" / "c1cat-py")[2:].replace("\\", "/")

# Changed on every write; when it was written, not what.
VOLATILE = {("ZDOCUMENTCONTENT", "ZDATEMODIFIED")}


def wsl_path(path: Path) -> str:
    p = str(path.resolve())
    return f"/mnt/{p[0].lower()}{p[2:].replace(os.sep, '/')}"


def pick(catalog: Path) -> dict[str, list[int]]:
    """Variants in each starting state the edits should meet."""
    con = sqlite3.connect(f"file:{catalog}?mode=ro", uri=True)
    groups = {
        "plain": "m.ZBASIC_RATING IS NULL AND m.ZCOLOR_TAG_INDEX IS NULL",
        "rated": "m.ZBASIC_RATING = 3",
        "colored": "m.ZCOLOR_TAG_INDEX = 2",
        "top": "m.ZBASIC_RATING = 5 AND m.ZCOLOR_TAG_INDEX IS NOT NULL",
        "placed": "m.ZIMAGE_CITY <> '' AND m.ZIMAGE_COUNTRY <> ''",
        "gps": "i.ZGPSLATITUDE LIKE '%.%' AND i.ZGPSLATITUDE NOT LIKE '%,%'",
        "gps_comma": "i.ZGPSLATITUDE LIKE '%,%'",
    }
    out = {}
    for name, where in groups.items():
        rows = con.execute(
            "SELECT v.Z_PK FROM ZVARIANT v JOIN ZVARIANTMETADATA m ON m.ZLAYER = v.ZADJUSTMENTLAYER"
            f" JOIN ZIMAGE i ON i.Z_PK = v.ZIMAGE WHERE i.ZISTRASHED = 0 AND {where} ORDER BY v.Z_PK LIMIT 4"
        ).fetchall()
        out[name] = [r[0] for r in rows]
    # The first keyword in the tree: a child under it shifts every interval after it.
    out["first_root"] = con.execute(
        "SELECT ZNAME FROM ZKEYWORD WHERE ZPARENT IS NULL ORDER BY ZLEFT LIMIT 1").fetchone()[0]
    con.close()
    return out


def edits(g: dict) -> list[tuple[str, list[str], list[int]]]:
    """(command, value arguments, variant ids), applied in order on both sides."""
    return [
        ("rate", ["--set", "4"], g["plain"] + g["rated"]),
        ("color", ["--set", "green"], g["plain"] + g["colored"]),
        ("rate", ["--clear"], g["top"]),
        ("color", ["--set", "none"], g["colored"] + g["top"]),
        ("rate", ["--set", "0"], g["rated"]),
        ("rate", ["--set", "1"], g["plain"][:1]),
        ("keyword", ["add", "street"], g["plain"]),
        ("keyword", ["add", "difftest|alpha"], g["rated"]),
        ("keyword", ["add", f"{g['first_root']}|difftest child"], g["colored"][:2]),
        ("keyword", ["add", "b&w", "difftest|beta", "difftest|alpha"], g["top"] + g["rated"][:1]),
        ("keyword", ["remove", "street"], g["plain"][:2]),
        ("keyword", ["remove", "difftest|alpha", "b&w"], g["rated"][1:3]),
        # The keyword tree itself: every photo carrying the keyword is rewritten.
        ("keyword", ["rename", "--path", "difftest|alpha", "--to", "gamma"], []),
        ("keyword", ["move", "--path", "difftest|beta", "--to-root"], []),
        ("keyword", ["move", "--path", f"{g['first_root']}|difftest child", "--to-parent", "difftest"], []),
        ("keyword", ["move", "--path", "beta", "--to-parent", "difftest newgroup"], []),
        ("keyword", ["merge", "--from", "difftest|gamma", "--into", "street"], []),
        # Location and caption text: both metadata rows. A cleared field is NULL. (Rust
        # leaves a field alone when it holds the value already, Python writes it anyway:
        # the same unless it held '' and is cleared, which these cases avoid.)
        ("iptc", ["--set-city", "Kraków", "--set-country", "Poland", "--set-country-code", "PL"],
         g["plain"][:2] + g["placed"][:2]),
        ("iptc", ["--set-city", ""], g["placed"][2:4]),
        ("iptc", ["--set-location", "Rynek Główny", "--set-state", "Małopolska"], g["placed"][:1] + g["rated"][:1]),
        ("iptc", ["--set-copyright", "difftest copyright", "--set-creator", "difftest"], g["top"][:2]),
        ("describe", ["--description", "difftest caption", "--title", "difftest title"], g["plain"][:2]),
        ("describe", ["--description", ""], g["plain"][:1]),
        # GPS is on the image, in Capture One's decimal form whatever it held before.
        ("geo", ["--lat", "50.0614", "--lon", "19.9366"], g["plain"][:2] + g["gps"][:1]),
        ("geo", ["--lat", "-33.8688", "--lon", "151.2093"], g["gps_comma"][:1]),
        ("geo", ["--lat", "52", "--lon", "-1"], g["rated"][:1]),
        ("geo", ["--clear"], g["gps"][1:2] + g["gps_comma"][1:2]),
    ]


def ids(pks: list[int]) -> list[str]:
    return [a for pk in pks for a in ("--id", str(pk))]


def run(argv: list[str], env: dict | None = None) -> str:
    proc = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", env=env)
    if proc.returncode != 0:
        sys.exit(f"failed ({proc.returncode}): {' '.join(argv)}\n{proc.stdout}\n{proc.stderr}")
    return proc.stdout


def python_side(catalog: Path, steps) -> None:
    for n, (command, value, pks) in enumerate(steps):
        # Rating and colour use the mirror only to resolve --id, so it is built once. Keyword
        # edits start from the keywords the mirror holds, so it must be current for them.
        sync = [] if n == 0 or command == "keyword" else ["--no-sync"]
        argv = ["--catalog", wsl_path(catalog), "--quiet", *sync, command, *value, *ids(pks), "--apply", "--no-backup"]
        script = f"cd {LEGACY_WSL} && {PYTHON_WSL} -m c1cat " + " ".join(f"'{a}'" for a in argv)
        run(["wsl.exe", "-e", "bash", "-lc", script])


def forget_mirror(catalog: Path) -> None:
    """Python keeps a mirror per catalog path in WSL; the copy's is of no further use."""
    code = ("from pathlib import Path; import shutil; from c1cat import config; "
            f"shutil.rmtree(config.cache_dir(Path('{wsl_path(catalog)}')), ignore_errors=True)")
    subprocess.run(["wsl.exe", "-e", "bash", "-lc", f'cd {LEGACY_WSL} && {PYTHON_WSL} -c "{code}"'], capture_output=True)


def rust_side(exe: Path, catalog: Path, data: Path, steps) -> list[dict]:
    env = {**os.environ, "C1_DATA_DIR": str(data)}
    reports = []
    for command, value, pks in steps:
        out = run([str(exe), "--catalog", str(catalog), command, *value, *ids(pks), "--apply", "--json"], env)
        reports.append(json.loads(out))
    return reports


def table_rows(con: sqlite3.Connection, table: str):
    columns = [r[1] for r in con.execute(f"PRAGMA table_info({table})")]
    keep = [c for c in columns if (table, c) not in VOLATILE]
    return keep, con.execute(f"SELECT {', '.join(keep)} FROM {table} ORDER BY rowid").fetchall()


def compare(a: Path, b: Path, original: Path) -> int:
    cons = [sqlite3.connect(f"file:{p}?mode=ro", uri=True) for p in (a, b, original)]
    tables = [r[0] for r in cons[0].execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")]
    differ = 0
    for table in tables:
        (cols, ra), (_, rb), (_, ro) = (table_rows(c, table) for c in cons)
        changed = sum(1 for x, y in zip(ra, ro) if x != y) + abs(len(ra) - len(ro))
        if ra == rb:
            if changed:
                print(f"  same     {table}: {changed} row(s) changed on both sides")
            continue
        differ += 1
        print(f"  DIFFERS  {table}: python {len(ra)} rows, rust {len(rb)} rows")
        shown = 0
        for x, y in zip(ra, rb):
            if x != y and shown < 5:
                shown += 1
                diffs = {c: (u, v) for c, u, v in zip(cols, x, y) if u != v}
                print(f"           row {x[cols.index('Z_PK')] if 'Z_PK' in cols else '?'}: {diffs}")
    for c in cons:
        c.close()
    return differ


def main() -> int:
    sys.stdout.reconfigure(encoding="utf-8")  # place names
    ap = argparse.ArgumentParser()
    ap.add_argument("--catalog", type=Path, help="defaults to %%APPDATA%%\\c1studio\\config.json")
    ap.add_argument("--work", type=Path, required=True, help="scratch directory, created and later removed")
    ap.add_argument("--rust", type=Path, default=ROOT / "target" / "release" / "c1cat.exe")
    ap.add_argument("--keep", action="store_true", help="leave the copies in --work")
    args = ap.parse_args()

    catalog = args.catalog or Path(json.loads((Path(os.environ["APPDATA"]) / "c1studio" / "config.json").read_text())["catalog"])
    if args.work.exists() and any(args.work.iterdir()):
        sys.exit(f"{args.work} is not empty")
    work = args.work
    try:
        original = work / "original" / catalog.name
        original.parent.mkdir(parents=True)
        # One read of the catalog; both sides start from this exact copy.
        print(f"copying {catalog} ...")
        src = sqlite3.connect(f"file:{catalog}?mode=ro", uri=True)
        dst = sqlite3.connect(original)
        src.backup(dst)
        dst.close()
        src.close()
        py, rs = work / "py" / catalog.name, work / "rs" / catalog.name
        for target in (py, rs):
            target.parent.mkdir()
            shutil.copyfile(original, target)

        steps = edits(pick(original))
        for command, value, pks in steps:
            print(f"  {command} {' '.join(value)} on {len(pks)}: {pks}")
        print("python ...")
        python_side(py, steps)
        print("rust ...")
        reports = rust_side(args.rust, rs, work / "rsdata", steps)
        conflicts = [r["conflicts"] for r in reports if r["conflicts"]]
        if conflicts:
            print(f"rust reported conflicts: {conflicts}")
        print("comparing ...")
        differ = compare(py, rs, original)
        backups = [r["backup"] for r in reports if r["backup"]]
        print(f"rust made {len(set(backups))} backup(s); wrote {sum(r['applied'] or 0 for r in reports)} change(s)")
        print("SAME" if not differ and not conflicts else f"{differ} table(s) DIFFER")
        return 1 if differ or conflicts else 0
    finally:
        if not args.keep:
            forget_mirror(work / "py" / catalog.name)
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
