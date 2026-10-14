import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { allowPlaceLookups, placeAt, pointFor } from "../src/lib/placeHints";
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

test("told to wait, those queued meanwhile go one a second after it, not all at once", async () => {
  const at: number[] = [];
  const fetch = mock.method(globalThis, "fetch", async () => {
    at.push(Date.now());
    return at.length === 1 ? reply(503, {}, { "Retry-After": "3" }) : reply(200, krakow);
  });
  const asked = [placeAt(2, 2), placeAt(3, 3), placeAt(4, 4)];
  await Promise.all(asked);
  assert.equal(at.length, 3);
  assert.ok(at[1] - at[0] >= 2900, `waits out Retry-After, not ${at[1] - at[0]} ms`);
  assert.ok(at[2] - at[1] >= 1000, `a second apart, not ${at[2] - at[1]} ms`);
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

test("a place is looked up with all its names, then fewer, down to what is known", async () => {
  const urls: string[] = [];
  const fetch = mock.method(globalThis, "fetch", async (url: string) => {
    urls.push(decodeURIComponent(url));
    return reply(200, urls.length === 1 ? [] : [{ lat: "50.05", lon: "19.93" }]);
  });
  const place = { country: "Poland", country_code: "PL", state: "Malopolska", city: "Kraków", location: "Podwawelskie" };
  assert.deepEqual(await pointFor(place, "City"), null, "the location alone is new; not found, it says so");
  assert.equal(urls.length, 1);
  assert.match(urls[0], /q=Podwawelskie, Kraków, Malopolska, Poland&countrycodes=pl/);
  assert.deepEqual(await pointFor({ ...place, location: null }, "Country"), { lat: 50.05, lon: 19.93, level: "City" });
  assert.match(urls[1], /q=Kraków, Malopolska, Poland/);
  // Offline: not asked, and not to be kept as nothing found.
  allowPlaceLookups(false);
  assert.equal(await pointFor(place, null), undefined);
  allowPlaceLookups(true);
  assert.equal(urls.length, 2);
  fetch.mock.restore();
});

test("an answer that is no position is asked again later; a code cannot add to the query", async () => {
  const urls: string[] = [];
  let body: unknown = [{ lat: "not a number", lon: "19.9" }];
  const fetch = mock.method(globalThis, "fetch", async (url: string) => (urls.push(url), reply(200, body)));
  const place = { country: "Poland", country_code: "pl&x=1", state: null, city: "Kraków", location: null };
  assert.equal(await pointFor(place, null), undefined);
  assert.ok(!urls[0].includes("countrycodes"), urls[0]);
  body = { error: "html page" };
  assert.equal(await pointFor(place, null), undefined);
  body = [{ lat: "95", lon: "19.9" }];
  assert.equal(await pointFor(place, null), undefined);
  fetch.mock.restore();
});

test("dragging a pin moves the photos of it picked, two or more; else all", () => {
  const stack = [1, 2, 3, 4];
  assert.deepEqual(movingOf(stack, [2, 3, 9]), [2, 3]);
  assert.deepEqual(movingOf(stack, [2]), stack, "one picked is only looked at");
  assert.deepEqual(movingOf(stack, [1, 2, 3, 4, 5]), stack);
  assert.deepEqual(movingOf(stack, []), stack);
});
