import { test } from "node:test";
import assert from "node:assert/strict";
import { parsePoint } from "../src/lib/coords";

function near(text: string, lat: number, lon: number) {
  const got = parsePoint(text);
  assert.ok(Array.isArray(got), `${text}: ${got}`);
  assert.ok(Math.abs(got[0] - lat) < 1e-9 && Math.abs(got[1] - lon) < 1e-9, `${text} gave ${got}`);
}

test("decimal degrees, with or without hemispheres", () => {
  near("50.0614, 19.9366", 50.0614, 19.9366);
  near("50.0614 19.9366", 50.0614, 19.9366);
  near("-33.8688; 151.2093", -33.8688, 151.2093);
  near("33.8688S 151.2093E", -33.8688, 151.2093);
  near("33.8688 S, 151.2093 E", -33.8688, 151.2093);
});

test("degrees, minutes and seconds in the ways they are written", () => {
  const krk: [number, number] = [44 + 58 / 60, 14 + 45 / 60];
  for (const text of [
    "44°58′N 14°45′E",
    "44°58'N, 14°45'E",
    "N44°58′ E14°45′",
    "N 44° 58′, E 14° 45′",
    "44 58 N 14 45 E",
    "14°45′E 44°58′N",
    "44°58′ 14°45′",
    "44 58, 14 45",
  ]) near(text, ...krk);
  const sec = [50 + 3 / 60 + 41 / 3600, 19 + 56 / 60 + 12 / 3600];
  near(`50°3'41.0"N 19°56'12"E`, sec[0], sec[1]);
  near("50°3′41″N 19°56′12″W", sec[0], -sec[1]);
  near("50°3'41''N 19°56'12''E", sec[0], sec[1]);
  near("50°2.25′N 19°56.5′E", 50 + 2.25 / 60, 19 + 56.5 / 60);
});

test("what is not a position says so", () => {
  for (const bad of ["Kraków", "50.06", "91, 10", "10, 181", "44°58′E 14°45′E", "44°75′N 14°45′E", "1 2 3"]) {
    assert.equal(typeof parsePoint(bad), "string", bad);
  }
});
