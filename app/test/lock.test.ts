import { test } from "node:test";
import assert from "node:assert/strict";
import { isLockError, ownerUnknown, waitingText } from "../src/lib/lock";

test("the lock error of an open library is told from other errors", () => {
  assert.ok(
    isLockError(
      "opening store C:\\x\\library\\store.redb: Database already open. Cannot acquire lock.",
    ),
  );
  assert.ok(isLockError("Cannot acquire lock"));
  assert.ok(!isLockError("no such file"));
  assert.ok(!isLockError(null));
  assert.ok(!isLockError(""));
});

test("autosave says what it waits for, and when Capture One cannot be checked", () => {
  assert.equal(waitingText("CaptureOne.exe", "Capture One", "catalog"), "autosave waits for Capture One to close");
  assert.equal(waitingText("ImgCoreProcess.exe", null, "catalog"), "autosave waits for ImgCoreProcess.exe to close");
  assert.equal(
    waitingText("another program that has the catalog open", "Capture One", "catalog"),
    "autosave waits: the catalog is open in another program",
  );
  const unknown = "Capture One, which may be running (WSL cannot read Windows' process list: is Windows interop on?)";
  assert.ok(ownerUnknown(unknown));
  assert.ok(!ownerUnknown("CaptureOne.exe"));
  assert.equal(waitingText(unknown, "Capture One", "catalog"), `autosave waits for ${unknown}`);
});
