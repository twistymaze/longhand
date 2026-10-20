import assert from "node:assert/strict";
import { test } from "node:test";
import { compareVersions, satisfies } from "./version.ts";

test("1.10.0 is after 1.9.3, and two equal versions compare as 0", () => {
  assert.equal(compareVersions("1.10.0", "1.9.3"), 1);
  assert.equal(compareVersions("1.9.3", "1.10.0"), -1);
  assert.equal(compareVersions("0.3.0", "0.3.0"), 0);
});

test("a sort with compareVersions gives numeric order", () => {
  assert.deepEqual(["0.10.0", "0.2.0", "0.9.1", "1.0.0"].sort(compareVersions), ["0.2.0", "0.9.1", "0.10.0", "1.0.0"]);
});

test("a version that is not plain x.y.z is an error", () => {
  for (const bad of ["1.2", "v1.2.3", "1.2.3-beta.1"]) {
    assert.throws(() => compareVersions(bad, "1.0.0"), (e: Error) => e.message.includes(`"${bad}"`));
  }
});

test("1.2.3 is in ranges of comparators joined by spaces and ||", () => {
  assert.equal(satisfies("1.2.3", ">=1.0.0 <1.2.4"), true);
  assert.equal(satisfies("1.2.3", "<0.2.1 || >=1.2.0 <1.3.0"), true);
  assert.equal(satisfies("1.2.3", ">=1.2.4"), false);
  assert.equal(satisfies("1.2.3", "<=1.2.3"), true);
  assert.equal(satisfies("1.2.3", ">1.2.3"), false);
  assert.equal(satisfies("1.2.3", "=1.2.3"), true);
  assert.equal(satisfies("1.2.3", "1.2.3"), true);
});

test("the range * contains every version", () => {
  assert.equal(satisfies("0.0.1", "*"), true);
});

test("a range with ^, ~ or an x placeholder is an error", () => {
  for (const bad of ["^1.2.0", "~1.2.0", "1.2.x", ">=1.x <2.0.0"]) {
    assert.throws(() => satisfies("1.2.3", bad), /range/);
  }
});
