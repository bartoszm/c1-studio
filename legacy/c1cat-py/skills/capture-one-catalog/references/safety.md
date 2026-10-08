# The write protocol

## The rule that cannot be engineered around

Capture One loads the whole catalog into memory when it opens and rewrites it when it
saves. Anything written to the file underneath a running Capture One is silently
discarded on its next save. So the tool refuses to commit while Capture One is detected,
and tells the user to reopen it afterwards.

## The guard chain

Every commit runs these in order and stops at the first failure.

1. **Journal mode** must be `delete`. Anything else means a different Capture One version
   or an operation in progress. Never bypassable.
2. **Catalog version** must be `ZVERSION=160800`. A different value means column meanings
   may have shifted; `--force` overrides.
3. **No rollback journal.** A non-empty `<catalog>-journal` means another process is
   mid-transaction. Never bypassable.
4. **Capture One is not running.** `tasklist.exe /FI "IMAGENAME eq CaptureOne.exe"`,
   checked for `CaptureOne.exe` and `ImgCoreProcess.exe`.
5. **The file is not locked by Windows.** PowerShell opens it with
   `[System.IO.File]::Open(path, 'Open', 'ReadWrite', 'None')` and reports `FREE` or
   `LOCKED`. This check exists because a POSIX lock taken from WSL does **not** interlock
   with a Windows file lock on a DrvFs mount — SQLite's own `BEGIN IMMEDIATE` succeeding
   proves nothing about Capture One.
6. **Recent-write notice.** A catalog modified in the last two minutes gets a warning,
   not a refusal: a fresh mtime also follows a plain file copy or a Capture One backup.

`--force` bypasses 4 and 5 only. Steps 1 and 3 are absolute.

The lock probe is known to work in the positive direction: it correctly reported
`LOCKED` while a stray read-only SQLite handle from this tool was open, and `FREE` once
that handle was closed. Whether Capture One itself holds the file with
`FileShare.None` has not been confirmed on this machine — the process check in step 4 is
the primary defence, and the probe is the backstop.

## Dry runs

Every mutating command previews unless `--apply` is given. The preview is produced by
performing the real mutation inside a transaction and rolling it back, so there is one
code path rather than a separate simulation that can drift.

A dry run only needs the structural checks (1–3), because it never keeps its
transaction. That means the user can still plan a change while Capture One is open, and
only the commit is blocked.

## Backups

Before the first commit in a process, the catalog is copied aside with
`sqlite3.Connection.backup()` — a page-consistent online copy, with `shutil.copy2` as a
fallback.

Backups go to `~/.local/share/c1cat/backups/` on the **WSL side**, not next to the
catalog: `C:` has 32 GB free of 475 GB, each backup is ~715 MB, and Capture One already
keeps its own 540–667 MB rolling backups in the catalog folder. `--backup-dir`
overrides, `--keep` sets retention (5 by default), and a backup is reused rather than
rewritten when the catalog has not changed since.

```bash
c1cat backup --list
c1cat backup --now
c1cat backup --restore 0 --apply      # newest; takes a fresh backup of the current state first
```

## Inside the transaction

`BEGIN IMMEDIATE`, then the mutations, then invariant checks, then `COMMIT` or
`ROLLBACK`. Foreign keys stay off, matching how Capture One itself writes; the tool
verifies its own invariants explicitly instead:

- the keyword nested set is still exactly `1..2n`, with every child interval inside its
  parent's;
- collection links are no worse than they were before the operation (see
  `collections.md` — the catalog has pre-existing mismatches that Capture One tolerates);
- metadata went to both the adjustment and combined rows.

Any failure rolls back and exits non-zero.

## Exit codes

| code | meaning |
|---|---|
| 0 | fine |
| 1 | bad usage or an empty selection |
| 2 | refused — Capture One is running, or the file is locked |
| 3 | the catalog is not in a state we understand |
| 4 | an invariant check failed and the change was rolled back |
| 5 | an external tool failed |

## Recovering

If Capture One shows something unexpected after a write, close it, then
`c1cat backup --list` and `c1cat backup --restore N --apply`. Practise this once before
relying on it. Capture One's own `*.backup N` files in the catalog folder are a second
line of defence.
