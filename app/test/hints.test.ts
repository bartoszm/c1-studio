import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { allowPlaceLookups, placeAt } from "../src/lib/placeHints";
import { movingOf } from "../src/lib/pins";

const reply = (status: number, body: unknown = {}, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers });
const krakow = { address: { city: "Kraków", country: "Poland", country_code: "pl" } };

test("a spot is asked once, at most one a second; refused, it waits as told and asks again", async () => {
  const at: number[] = [];
  let answer = () => reply(200, krakow);
  const fetch = mock.method(globalThis, "fetch", async () => (at.push(Date.now()), answer()));
  assert.equal((await placeAt(50.06, 19.94))?.city, "Kraków");
  // The same spot again: no new request.
  assert.equal((await placeAt(50.06, 19.94))?.city, "Kraków");
  assert.equal(at.length, 1);
  // Another spot waits its second.
  await placeAt(50.07, 19.95);
  assert.equal(at.length, 2);
  assert.ok(at[1] - at[0] >= 1000, `a second apart, not ${at[1] - at[0]} ms`);

  // Refused: null, not kept, and the next ask waits as long as told.
  answer = () => reply(429, {}, { "Retry-After": "2" });
  assert.equal(await placeAt(1, 1), null);
  answer = () => reply(200, krakow);
  assert.equal((await placeAt(1, 1))?.city, "Kraków", "asked again");
  assert.equal(at.length, 4);
  assert.ok(at[3] - at[2] >= 1900, `waits out Retry-After, not ${at[3] - at[2]} ms`);
  fetch.mock.restore();
});

test("with online lookups off, nothing is asked", async () => {
  const fetch = mock.method(globalThis, "fetch", async () => reply(200, krakow));
  allowPlaceLookups(false);
  assert.equal(await placeAt(10, 10), null);
  assert.equal(fetch.mock.callCount(), 0);
  allowPlaceLookups(true);
  fetch.mock.restore();
});

test("dragging a pin moves the photos of it picked, two or more; else all", () => {
  const stack = [1, 2, 3, 4];
  assert.deepEqual(movingOf(stack, [2, 3, 9]), [2, 3]);
  assert.deepEqual(movingOf(stack, [2]), stack, "one picked is only looked at");
  assert.deepEqual(movingOf(stack, [1, 2, 3, 4, 5]), stack);
  assert.deepEqual(movingOf(stack, []), stack);
});
