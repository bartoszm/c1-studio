import { test } from "node:test";
import assert from "node:assert/strict";
import type { Photo } from "../src/lib/api";
import { lensOf } from "../src/lib/format";

const photo = (lens: string | null, lens_name?: string | null) => ({ exif: { lens }, lens_name }) as unknown as Photo;

test("the lens is the index's name for it, or the file's when that is not known", () => {
  assert.equal(lensOf(photo("Fujifilm Fujinon XF23mmF2 R WR", "XF23mmF2 R WR")), "XF23mmF2 R WR");
  // An older app sends no lens_name: what the file says.
  assert.equal(lensOf(photo("XF23mmF2 R WR")), "XF23mmF2 R WR");
  assert.equal(lensOf(photo(null)), null);
  // null is no lens, even when the file says something: `-- mm f/--` hides the Lens row.
  assert.equal(lensOf(photo("-- mm f/--", null)), null);
});
