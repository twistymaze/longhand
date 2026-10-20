import { describe, expect, test } from "vitest";
import { OwnSaves } from "./ownSaves";

describe("OwnSaves", () => {
  test("a saved value that this window sent is its own save, once", () => {
    const own = new OwnSaves<string>();
    own.sent("A");
    own.settled();
    expect(own.has("A")).toBe(true);
    expect(own.confirm("A")).toBe(true);
    expect(own.has("A")).toBe(false);
    // Another computer sets the same text later.
    expect(own.confirm("A")).toBe(false);
  });

  test("a confirmed value removes the values sent before it", () => {
    const own = new OwnSaves<string>();
    own.sent("A");
    own.sent("B");
    own.settled();
    own.settled();
    expect(own.confirm("B")).toBe(true);
    expect(own.has("A")).toBe(false);
  });

  test("a value sent twice matches the oldest entry", () => {
    const own = new OwnSaves<string>();
    own.sent("A");
    own.sent("B");
    own.sent("A");
    expect(own.confirm("A")).toBe(true);
    expect(own.has("B")).toBe(true);
    expect(own.confirm("B")).toBe(true);
    expect(own.confirm("A")).toBe(true);
  });

  test("a value that this window did not send clears the list when no save is in flight", () => {
    const own = new OwnSaves<string>();
    own.sent("A");
    own.settled();
    expect(own.confirm("R")).toBe(false);
    expect(own.has("A")).toBe(false);
  });

  test("a value that this window did not send keeps the list while a save is in flight", () => {
    const own = new OwnSaves<string>();
    own.sent("A");
    expect(own.confirm("R")).toBe(false);
    expect(own.has("A")).toBe(true);
    own.settled();
    expect(own.confirm("A")).toBe(true);
  });

  test("a failed save keeps its value, because the backend can have written it", () => {
    const own = new OwnSaves<string>();
    own.sent("A");
    own.settled();
    expect(own.has("A")).toBe(true);
    expect(own.confirm("A")).toBe(true);
  });
});
