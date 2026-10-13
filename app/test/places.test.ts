import { test } from "node:test";
import assert from "node:assert/strict";
import { fieldValues, fillFor, fromNominatim, parsePath, suggest, treeKeys, type Place } from "../src/lib/places";

const CATALOG: [string, number][] = [
  ["Poland", 30],
  ["Poland|Małopolskie", 20],
  ["Poland|Małopolskie|Kraków", 18],
  ["Poland|Małopolskie|Kraków|Kazimierz", 4],
  ["Poland|Mazowieckie", 8],
  ["Poland|Mazowieckie|Warszawa", 8],
  ["Poland|—", 2],
  ["Poland|—|Kraków", 2],
  ["Japan", 5],
  ["Japan|Kyoto", 5],
  ["Japan|Kyoto|Kyoto", 5],
];

const place = (p: Partial<Place>): Place => ({
  country: null, country_code: null, state: null, city: null, location: null, ...p,
});

test("a missing level reads as empty", () => {
  assert.deepEqual(parsePath("Poland|—|Kraków"), { country: "Poland", state: "", city: "Kraków", location: "" });
});

test("cities show the state and country they are in", () => {
  const [first] = suggest("city", "kra", {}, { catalog: CATALOG });
  assert.equal(first.value, "Kraków");
  assert.equal(first.parents, "Małopolskie › Poland");
  assert.equal(first.n, 18);
});

test("places in the photo's own country and state come first", () => {
  const all = suggest("city", "", { country: "Japan" }, { catalog: CATALOG });
  assert.equal(all[0].key, "Japan|Kyoto|Kyoto");
  const inState = suggest("city", "", { country: "Poland", state: "Mazowieckie" }, { catalog: CATALOG });
  assert.equal(inState[0].value, "Warszawa");
});

test("only the typed level is suggested, never an empty one", () => {
  const states = suggest("state", "", {}, { catalog: CATALOG });
  assert.deepEqual(states.map((s) => s.value).sort(), ["Kyoto", "Mazowieckie", "Małopolskie"]);
});

test("nearby places and the geocoded one come before the rest", () => {
  const nearby = suggest("city", "", { country: "Japan" }, { catalog: CATALOG, nearby: [["Poland|Mazowieckie|Warszawa", 3]] });
  assert.equal(nearby[0].value, "Warszawa");
  assert.equal(nearby[0].source, "nearby");
  assert.equal(nearby[0].n, 8, "the count stays the catalog's");
  // The geocoder says Lesser Poland; the catalog's Kraków in Poland stands for it.
  const here = { country: "Polska", state: "Lesser Poland Voivodeship", city: "Kraków", location: "Stare Miasto", code: "PL" };
  const cities = suggest("city", "", {}, { catalog: CATALOG, here });
  assert.deepEqual(cities.filter((s) => s.source === "here").map((s) => s.key), ["Poland|Małopolskie|Kraków", "Poland|—|Kraków"]);
  // One the catalog does not have comes as the geocoder names it.
  const [loc] = suggest("location", "", {}, { catalog: CATALOG, here });
  assert.equal(loc.source, "here");
  assert.equal(loc.key, "Polska|Lesser Poland Voivodeship|Kraków|Stare Miasto");
  // A country is matched by its code.
  assert.equal(suggest("country", "", {}, { catalog: CATALOG, here })[0].key, "Poland");
});

test("picking a city fills the state, country and code but not the location", () => {
  const fields = fillFor(parsePath("Poland|Małopolskie|Kraków"), "city", place({ location: "Rynek" }));
  assert.deepEqual(fields, { country: "Poland", state: "Małopolskie", city: "Kraków", country_code: "PL" });
  // A level the place leaves empty is not cleared on the photo.
  assert.deepEqual(fillFor(parsePath("—|—|—|Brooklyn Bridge"), "location", place({ country: "United States" })), {
    location: "Brooklyn Bridge",
  });
  assert.equal(fillFor(parsePath("Poland|—|Kraków"), "city", place({ state: "X" })).state, undefined);
});

test("a selection's places count as the index counts them", () => {
  const keys = new Map(
    treeKeys([
      [place({ country: "Poland", city: "Kraków" }), 3],
      [place({ country: "Poland" }), 2],
      [place({}), 4],
    ]),
  );
  assert.deepEqual(Object.fromEntries(keys), { Poland: 5, "Poland|—": 3, "Poland|—|Kraków": 3 });
});

test("a field across a selection: its values and how many leave it empty", () => {
  const v = fieldValues([[place({ city: "Kraków" }), 3], [place({ city: "Oslo" }), 1], [place({}), 2]], "city");
  assert.deepEqual(v, { values: [["Kraków", 3], ["Oslo", 1]], empty: 2 });
});

test("the reverse geocoder's address becomes a place", () => {
  const p = fromNominatim({
    address: { town: "Zakopane", state: "Lesser Poland Voivodeship", country: "Poland", country_code: "pl", suburb: "Krzeptówki" },
  });
  assert.deepEqual(p, { country: "Poland", state: "Lesser Poland Voivodeship", city: "Zakopane", location: "Krzeptówki", code: "PL" });
  assert.equal(fromNominatim({ error: "Unable to geocode" } as never), null);
});
