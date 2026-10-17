import { test } from "node:test";
import assert from "node:assert/strict";
import { isLockError } from "../src/lib/lock";

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
