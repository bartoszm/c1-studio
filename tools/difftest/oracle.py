"""The Python side of the differential test. Runs inside WSL.

Reads cases from stdin as JSON (`[{"name": ..., "args": [...]}]`), runs each through
the legacy c1cat selector against its own mirror of the catalog, and prints
`{name: [variant_pk, ...]}` (or `{name: {"error": ...}}`) to stdout.
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "legacy" / "c1cat-py"))

from c1cat import config, selector  # noqa: E402
from c1cat.__main__ import Context, build_parser  # noqa: E402


def main() -> int:
    catalog = config.find_catalog(sys.argv[1] if len(sys.argv) > 1 else None)
    parser = build_parser()
    cases = json.load(sys.stdin)
    ctx = None
    out = {}
    # Anything the legacy code prints (mirror rebuild progress) must not mix with the JSON.
    stdout, sys.stdout = sys.stdout, sys.stderr
    for case in cases:
        args = parser.parse_args(["find", *case["args"]])
        if ctx is None:
            ctx = Context(catalog=catalog, args=args)
        try:
            rows, _ = selector.run(args, ctx.mirror)
            out[case["name"]] = sorted(str(r["variant_pk"]) for r in rows)
        except selector.SelectorError as exc:
            out[case["name"]] = {"error": str(exc)}
    json.dump(out, stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
