import { test } from "node:test";
import assert from "node:assert/strict";
import { binNote, deleteAction, removalText, withBin } from "../src/lib/bin";
import type { Wording } from "../src/lib/api";

const C1: Wording = { name: "Capture One", word: "Capture One", noun: "catalog", owner: "Capture One" };

test("the trash's photos are asked for only while it is shown", () => {
  assert.deepEqual(withBin({ hues: ["red"] }, false), { hues: ["red"] });
  assert.deepEqual(withBin({ hues: ["red"] }, true), { hues: ["red"], bin: "Only", hidden: "Include" });
});

test("Delete trashes, except in an album, where Ctrl+Delete does", () => {
  const at = (o: Partial<Parameters<typeof deleteAction>[0]>) =>
    deleteAction({ ctrl: false, album: false, inBin: false, removes: "Files", ...o });
  assert.equal(at({}), "bin");
  assert.equal(at({ album: true }), "album", "out of the album, as before");
  assert.equal(at({ album: true, ctrl: true }), "bin");
  assert.equal(at({ ctrl: true }), "bin");
  assert.equal(at({ inBin: true }), "none", "in the trash already");
  assert.equal(at({ removes: "None" }), "none", "a source that cannot delete");
  assert.equal(at({ removes: undefined }), "none", "no source yet");
  assert.equal(at({ removes: "None", album: true }), "album", "albums still work");
});

test("the notes count the photos and say what emptying does", () => {
  assert.equal(binNote(1, true), "1 photo moved to Trash");
  assert.equal(binNote(2, false), "2 photos restored");
  assert.match(removalText(3, "Files"), /files of 3 photos go to the Windows Recycle Bin/);
  assert.match(removalText(1, "MarksTrashed", C1), /^1 photo is marked deleted in Capture One and shows/);
  assert.match(removalText(2, "MarksTrashed", C1), /^2 photos are marked deleted in Capture One and show /);
});
