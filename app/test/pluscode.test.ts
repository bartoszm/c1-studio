import { test } from "node:test";
import assert from "node:assert/strict";
import { parsePoint } from "../src/lib/coords";
import { decode, encode, kindOf, parsePlusCode, recoverNearest } from "../src/lib/pluscode";

const close = (got: [number, number] | string, lat: number, lon: number, eps = 1e-6) => {
  assert.ok(Array.isArray(got), String(got));
  assert.ok(Math.abs(got[0] - lat) < eps && Math.abs(got[1] - lon) < eps, `got ${got}, wanted ${lat}, ${lon}`);
};

// The examples of github.com/google/open-location-code.
test("a point's code, and a code's middle", () => {
  assert.equal(encode(47.36559, 8.524997), "8FVC9G8F+6X");
  close(decode("8FVC9G8F+6X"), 47.3655625, 8.5249375);
  close(decode("8fvc9g8f+6x"), 47.3655625, 8.5249375);
  // Padded: the whole 1° square, and 11 digits: a smaller cell inside the 10-digit one.
  close(decode("8FVC0000+"), 47.5, 8.5);
  close(decode("7FG49Q00+"), 20.375, 2.775);
  const fine = decode("8FVC9G8F+6XQ");
  assert.ok(Math.abs(fine[0] - 47.3655625) < 0.0000625 && Math.abs(fine[1] - 8.5249375) < 0.0000625);
  assert.equal(encode(-33.8688, 151.2093).length, 11);
  assert.equal(encode(-33.8688, 151.2093), encode(-33.8688, 151.2093 - 360));
});

test("a short code is taken near a point", () => {
  close(recoverNearest("9G8F+6X", [47.4, 8.6]), 47.3655625, 8.5249375);
  // Across the edge of the 1° square the reference is in.
  close(recoverNearest("9G8F+6X", [46.9, 8.6]), 47.3655625, 8.5249375);
  // Six digits gone: the same code every 0.05°, so the nearest is the one meant only close by.
  close(recoverNearest("8F+6X", [47.37, 8.52]), 47.3655625, 8.5249375);
  close(recoverNearest("8F+6X", [47.4, 8.6]), 47.4155625, 8.6249375);
});

test("what is a plus code and what is not", () => {
  assert.equal(kindOf("8FVC9G8F+6X"), "full");
  assert.equal(kindOf("8FVC0000+"), "full");
  assert.equal(kindOf("9G8F+6X"), "short");
  for (const bad of ["8FVC9G8F+6", "8FVC9G8F6X", "8F+VC+6X", "+6X", "8FVC9G8F+6A", "WC2345G6+", "8FV00000+", "8FVC0000+6X", "9G8F0+6X"])
    assert.equal(kindOf(bad), null, bad);
});

test("typed: alone, or a short code with its town", () => {
  assert.deepEqual(parsePlusCode("9G8F+6X Zürich, Switzerland"), { short: "9G8F+6X", locality: "Zürich, Switzerland" });
  assert.deepEqual(parsePlusCode(" 9g8f+6x "), { short: "9G8F+6X", locality: "" });
  const full = parsePlusCode("8FVC9G8F+6X Zürich");
  assert.ok(full && "full" in full);
  assert.equal(parsePlusCode("50.0614, 19.9366"), null);
  assert.equal(parsePlusCode("+50.0614 +19.9366"), null);
});

test("coordinates read plus codes too", () => {
  close(parsePoint("8FVC9G8F+6X"), 47.3655625, 8.5249375);
  close(parsePoint("9G8F+6X", [47.4, 8.6]), 47.3655625, 8.5249375);
  assert.equal(typeof parsePoint("9G8F+6X"), "string");
  assert.equal(typeof parsePoint("9G8F+6X Zürich", [47.4, 8.6]), "string", "a town is looked up by resolveGeo");
});
