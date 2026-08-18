"""Everything that stands between a command and the live catalog.

Capture One keeps the whole catalog in memory while it is open and rewrites it on
save, so any edit made underneath a running Capture One is silently lost. That is
not something we can engineer around; the only safe move is to refuse.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path

from . import config, source
from .paths import wsl_to_win

EXIT_OK = 0
EXIT_USAGE = 1
EXIT_REFUSED = 2
EXIT_INTEGRITY = 3
EXIT_VERIFY = 4
EXIT_EXTERNAL = 5


class Refused(Exception):
    """A guard said no. Carries the exit code the CLI should use."""

    def __init__(self, message: str, exit_code: int = EXIT_REFUSED, hint: str | None = None):
        super().__init__(message)
        self.exit_code = exit_code
        self.hint = hint


@dataclass
class GuardReport:
    checks: list[dict] = field(default_factory=list)

    def add(self, name: str, status: str, detail: str = "") -> None:
        self.checks.append({"check": name, "status": status, "detail": detail})

    @property
    def blocking(self) -> list[dict]:
        return [c for c in self.checks if c["status"] == "fail"]


def _run(argv: list[str], timeout: float) -> tuple[int, str]:
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return proc.returncode, (proc.stdout or "") + (proc.stderr or "")
    except (OSError, subprocess.SubprocessError) as exc:
        return -1, str(exc)


def detect_capture_one() -> dict:
    """Ask Windows whether Capture One is running. 'unknown' is not 'no'."""
    tasklist = config.WINDOWS_TOOLS["tasklist"]
    if not Path(tasklist).exists():
        return {"status": "unknown", "detail": "tasklist.exe not available"}
    running = []
    for name in config.C1_PROCESSES:
        code, out = _run([tasklist, "/NH", "/FO", "CSV", "/FI", f"IMAGENAME eq {name}"], timeout=25)
        if code != 0:
            return {"status": "unknown", "detail": f"tasklist failed: {out.strip()[:120]}"}
        if name.lower() in out.lower():
            running.append(name)
    if running:
        return {"status": "running", "detail": ", ".join(running)}
    return {"status": "stopped", "detail": "no Capture One process"}


def probe_windows_lock(catalog: Path) -> dict:
    """Try to open the catalog for exclusive read/write from the Windows side.

    A POSIX lock taken from WSL does not interlock with a Windows file lock on a
    DrvFs mount, so SQLite succeeding here proves nothing on its own.
    """
    powershell = config.WINDOWS_TOOLS["powershell"]
    win = wsl_to_win(catalog)
    if not Path(powershell).exists() or win is None:
        return {"status": "unknown", "detail": "no PowerShell or catalog is not on a Windows drive"}
    script = (
        f"try {{ $f=[System.IO.File]::Open('{win}','Open','ReadWrite','None'); $f.Close(); 'FREE' }} "
        f"catch {{ 'LOCKED' }}"
    )
    code, out = _run([powershell, "-NoProfile", "-NonInteractive", "-Command", script], timeout=40)
    text = out.strip().splitlines()[-1].strip() if out.strip() else ""
    if code != 0 or text not in ("FREE", "LOCKED"):
        return {"status": "unknown", "detail": f"probe inconclusive: {out.strip()[:120]}"}
    return {"status": "free" if text == "FREE" else "locked", "detail": text}


def file_holders(catalog: Path) -> str:
    """Which local processes have the catalog open, if lsof can tell us.

    A lingering read-only reader is enough to fail the Windows lock probe, and
    "locked by another process" is not actionable without a name.
    """
    code, out = _run(["lsof", "-F", "cp", "--", str(catalog)], timeout=10)
    if code not in (0, 1) or not out.strip():
        return ""
    seen: list[str] = []
    pid = ""
    for line in out.splitlines():
        if line.startswith("p"):
            pid = line[1:]
        elif line.startswith("c"):
            entry = f"{line[1:]} (pid {pid})"
            if entry not in seen:
                seen.append(entry)
    return ", ".join(seen)


def preflight(catalog: Path, *, write: bool, force: bool = False) -> GuardReport:
    """Run the guard chain. Raises Refused on the first blocking failure.

    Note ``write`` means "about to commit", not "opened read-write": a dry run holds
    a write transaction but never keeps it, so it only needs the structural checks.
    """
    report = GuardReport()

    con = source.read_only(catalog)
    try:
        info = source.schema_info(con)
    finally:
        # sqlite3's context manager commits, it does not close - and an open handle
        # of our own would make the Windows exclusive-open probe report LOCKED.
        con.close()
    if info["journal_mode"] != "delete":
        report.add("journal_mode", "fail", f"expected 'delete', found {info['journal_mode']!r}")
        raise Refused(
            f"catalog journal mode is {info['journal_mode']!r}, not 'delete' - refusing to write",
            EXIT_INTEGRITY)
    report.add("journal_mode", "ok", "delete")

    if info["version_supported"]:
        report.add("catalog_version", "ok", f"ZVERSION={info['zversion']}")
    else:
        detail = f"ZVERSION={info['zversion']}, expected {config.EXPECTED_ZVERSION}"
        if write and not force:
            report.add("catalog_version", "fail", detail)
            raise Refused(
                f"unexpected catalog version ({detail}); column meanings may have changed",
                EXIT_INTEGRITY, hint="re-run with --force if you know it is compatible")
        report.add("catalog_version", "warn", detail)

    if not write:
        return report

    journal = Path(str(catalog) + "-journal")
    if journal.exists() and journal.stat().st_size > 0:
        report.add("journal_file", "fail", f"{journal.name} is {journal.stat().st_size} bytes")
        raise Refused("a rollback journal is present - another process is mid-transaction",
                      EXIT_REFUSED)
    report.add("journal_file", "ok", "absent")

    process = detect_capture_one()
    if process["status"] == "running":
        report.add("capture_one", "fail" if not force else "warn", process["detail"])
        if not force:
            raise Refused(
                f"Capture One is running ({process['detail']}). It caches the catalog in memory "
                "and would overwrite these changes on its next save.",
                EXIT_REFUSED, hint="close Capture One, or re-run with --force if you are certain")
    else:
        report.add("capture_one", "ok" if process["status"] == "stopped" else "warn", process["detail"])

    lock = probe_windows_lock(catalog)
    if lock["status"] == "locked":
        holders = file_holders(catalog)
        detail = lock["detail"] + (f" (held by {holders})" if holders else "")
        report.add("windows_lock", "fail" if not force else "warn", detail)
        if not force:
            raise Refused(
                "the catalog file is open in another process"
                + (f": {holders}" if holders else ""),
                EXIT_REFUSED,
                hint="close Capture One (or that process), or re-run with --force. "
                     "Note that even a read-only sqlite3 session holds the file.")
    else:
        report.add("windows_lock", "ok" if lock["status"] == "free" else "warn", lock["detail"])

    # Advisory only. The process check and the lock probe are the real gates; a recent
    # mtime also happens after a plain file copy or a Capture One backup, so refusing
    # on it alone produces mostly false positives.
    age = time.time() - catalog.stat().st_mtime
    if age < 120:
        report.add("recent_write", "warn", f"modified {age:.0f}s ago")
    else:
        report.add("recent_write", "ok", f"modified {age / 60:.0f} min ago")

    return report


# --------------------------------------------------------------------------
# Backups
# --------------------------------------------------------------------------

def backup_dir() -> Path:
    config.BACKUP_ROOT.mkdir(parents=True, exist_ok=True)
    return config.BACKUP_ROOT


def list_backups(catalog: Path | None = None) -> list[dict]:
    out = []
    if not config.BACKUP_ROOT.is_dir():
        return out
    for meta in sorted(config.BACKUP_ROOT.glob("*.json"), reverse=True):
        try:
            record = json.loads(meta.read_text())
        except (OSError, ValueError):
            continue
        record["manifest"] = str(meta)
        payload = Path(record.get("backup", ""))
        record["exists"] = payload.is_file()
        record["size"] = payload.stat().st_size if record["exists"] else 0
        if catalog is None or record.get("source") == str(catalog):
            out.append(record)
    return out


def make_backup(catalog: Path, argv: list[str] | None = None, keep: int = 5) -> dict:
    """Online-copy the catalog aside. Reuses the newest backup if nothing changed."""
    stat = catalog.stat()
    for existing in list_backups(catalog):
        if (existing.get("source_mtime") == stat.st_mtime
                and existing.get("source_size") == stat.st_size and existing["exists"]):
            existing["reused"] = True
            return existing

    directory = backup_dir()
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    target = directory / f"{catalog.stem}.{stamp}{catalog.suffix}"
    started = time.time()
    src = source.read_only(catalog)
    try:
        dst = __import__("sqlite3").connect(str(target))
        try:
            src.backup(dst)
        finally:
            dst.close()
    except Exception:
        target.unlink(missing_ok=True)
        shutil.copy2(catalog, target)
    finally:
        src.close()

    record = {
        "backup": str(target),
        "source": str(catalog),
        "source_mtime": stat.st_mtime,
        "source_size": stat.st_size,
        "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "duration_s": round(time.time() - started, 2),
        "size": target.stat().st_size,
        "c1cat_version": config.VERSION,
        "argv": argv or [],
        "exists": True,
        "reused": False,
    }
    (directory / (target.name + ".json")).write_text(
        json.dumps(record, indent=2, ensure_ascii=False) + "\n")
    _prune(catalog, keep)
    return record


def _prune(catalog: Path, keep: int) -> None:
    records = list_backups(catalog)
    for stale in records[keep:]:
        Path(stale["backup"]).unlink(missing_ok=True)
        Path(stale["manifest"]).unlink(missing_ok=True)


# --------------------------------------------------------------------------
# The write transaction
# --------------------------------------------------------------------------

@contextmanager
def writer(catalog: Path, *, apply: bool, force: bool = False, backup: bool = True,
           argv: list[str] | None = None, keep: int = 5):
    """Run mutations in one transaction, committing only when ``apply`` is set.

    A dry run performs the real writes and then rolls back, so the preview is
    produced by the same code path that would commit - there is no second
    "what would happen" implementation to drift out of sync.
    """
    # A dry run rolls back, so it cannot clash with Capture One; only a commit needs
    # the full guard. Blocking the preview too would make the tool useless while
    # Capture One happens to be open, which is exactly when you want to plan a change.
    preflight(catalog, write=apply, force=force)

    backup_record = None
    if apply and backup:
        backup_record = make_backup(catalog, argv=argv, keep=keep)

    con = source.read_write(catalog)
    con.execute("BEGIN IMMEDIATE")
    ctx = {"backup": backup_record, "applied": False}
    try:
        yield con, ctx
    except Exception:
        con.execute("ROLLBACK")
        con.close()
        raise

    if apply:
        con.execute(
            "UPDATE ZDOCUMENTCONTENT SET ZDATEMODIFIED = ? WHERE Z_PK = "
            "(SELECT MIN(Z_PK) FROM ZDOCUMENTCONTENT)", (time.time(),))
        con.execute("COMMIT")
        ctx["applied"] = True
    else:
        con.execute("ROLLBACK")
    con.close()


def free_space(path: Path) -> int:
    usage = os.statvfs(path)
    return usage.f_bavail * usage.f_frsize
