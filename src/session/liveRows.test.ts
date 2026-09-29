import { describe, expect, test } from "bun:test";
import fixture from "../../crates/pipeline/tests/fixtures/paragraph-rule.json";
import {
  buildLiveRows,
  paragraphText,
  type LiveRowsInput,
  type LiveSpeakerRow,
  type NativeRow,
  type SpeakerBlock,
  type TimedWord,
} from "./liveRows";

const MINUTE = 60_000;
/** Words 300 ms long with 100 ms between them, from `start_ms`. */
function spoken(text: string, start_ms = 0): TimedWord[] {
  return text.split(" ").map((word, i) => ({
    start_ms: start_ms + i * 400,
    end_ms: start_ms + i * 400 + 300,
    text: word,
  }));
}
/** A turn in note milliseconds. */
const turn = (start_ms: number, end_ms: number, speaker: string, speaker_final = true) => ({
  start_sample: start_ms * 16,
  end_sample: end_ms * 16,
  speaker,
  speaker_final,
});
const id = (epoch: number, local: number) => `g:${epoch}:0:${local}`;
const block = (epoch: number, turns: SpeakerBlock["turns"]): SpeakerBlock => ({
  generation: "g",
  epoch,
  turns,
});
function input(overrides: Partial<LiveRowsInput>): LiveRowsInput {
  return { parts: [], open: [], native: [], speakers: [], numbers: new Map(), ...overrides };
}
const rows = (items: ReturnType<typeof buildLiveRows>) =>
  items.filter((item): item is LiveSpeakerRow => item.kind === "speaker");
/** Each item as "divider" or [number, paragraph texts]. */
function shape(items: ReturnType<typeof buildLiveRows>) {
  return items.map((item) =>
    item.kind === "divider" ? "divider" : [item.number, item.paragraphs.map(paragraphText)],
  );
}

describe("the paragraph rule", () => {
  test("each fixture case gives the paragraphs that the Rust join gives", () => {
    expect(fixture.cases.length).toBeGreaterThan(0);
    for (const c of fixture.cases) {
      const words = c.words.map(([start_ms, end_ms, text]) => ({
        start_ms: start_ms as number,
        end_ms: end_ms as number,
        text: text as string,
      }));
      const built = buildLiveRows(input({ parts: [{ segment: 0, part: 0, offset_ms: 0, words }] }));
      expect(built).toHaveLength(1);
      const [row] = rows(built);
      expect(row.number).toBeNull();
      expect({ name: c.name, paragraphs: row.paragraphs.map((p) => [p.key, paragraphText(p)]) }).toEqual({
        name: c.name,
        paragraphs: c.paragraphs.map(([first, text]) => [`0/${words[first as number].start_ms}`, text as string]),
      });
    }
  });

  test("5 minutes of one speaker with no long pause breaks at the first sentence end after 400 characters (AE3)", () => {
    const sentence = "alpha alpha alpha alpha alpha alpha alpha alpha alpha alpha.";
    const words = spoken(Array(76).fill(sentence).join(" "));
    expect(words[words.length - 1].end_ms).toBeGreaterThan(5 * MINUTE);
    const built = buildLiveRows(input({
      parts: [{ segment: 0, part: 0, offset_ms: 0, words }],
      speakers: [block(0, [turn(0, 6 * MINUTE, id(0, 1))])],
    }));
    expect(built).toHaveLength(1);
    const [row] = rows(built);
    expect(row.number).toBe(1);
    // 7 sentences of 59 characters are the first to pass 400 characters.
    const paragraph = Array(7).fill(sentence).join(" ");
    expect(row.paragraphs.map(paragraphText)).toEqual([
      ...Array(10).fill(paragraph),
      Array(6).fill(sentence).join(" "),
    ]);
  });

  test("a pause longer than 2 s starts a new paragraph in the same speaker row", () => {
    const words = [...spoken("We met today."), ...spoken("Then we left.", 3_300)];
    const built = buildLiveRows(input({
      parts: [{ segment: 0, part: 0, offset_ms: 0, words }],
      speakers: [block(0, [turn(0, 10_000, id(0, 1))])],
    }));
    expect(shape(built)).toEqual([[1, ["We met today.", "Then we left."]]]);
  });

  test("with no turns, the text shows as paragraphs with no headings (R9)", () => {
    const words = [...spoken("We met today."), ...spoken("Then we left.", 3_300)];
    const built = buildLiveRows(input({ parts: [{ segment: 0, part: 0, offset_ms: 0, words }] }));
    expect(shape(built)).toEqual([[null, ["We met today.", "Then we left."]]]);
    expect(rows(built)[0].speaker).toBeNull();
  });
});

describe("speaker rows", () => {
  test("words with no turn continue Speaker 1, and a later turn for B adds Speaker 2 at its first word (AE1)", () => {
    const saved = { segment: 0, part: 0, offset_ms: 0, words: spoken("Are you coming?") };
    const open = {
      segment: 0, part: 1, offset_ms: 0,
      words: spoken("Yes I am", 2_000),
      committed: "Yes I", tentative: "am",
    };
    const numbers = new Map<string, number>();
    const before = buildLiveRows(input({
      parts: [saved], open: [open], numbers,
      speakers: [block(0, [turn(0, 1_200, id(0, 1))])],
    }));
    expect(shape(before)).toEqual([[1, ["Are you coming? Yes I am"]]]);
    const after = buildLiveRows(input({
      parts: [saved], open: [open], numbers,
      speakers: [block(0, [turn(0, 1_200, id(0, 1)), turn(1_900, 3_000, id(0, 2))])],
    }));
    expect(shape(after)).toEqual([
      [1, ["Are you coming?"]],
      [2, ["Yes I am"]],
    ]);
    const second = rows(after)[1];
    expect(second.paragraphs[0].key).toBe("0/2000");
    expect(second.paragraphs[0].pieces).toEqual([
      { text: "Yes I", tentative: false },
      { text: " am", tentative: true },
    ]);
  });

  test("a second epoch at minute 20 and a new segment at minute 25 each add one divider and restart at Speaker 1 (AE2)", () => {
    const parts = [
      { segment: 0, part: 0, offset_ms: 0, words: spoken("Hello there.") },
      { segment: 0, part: 1, offset_ms: 0, words: spoken("Hi.", 60_000) },
      { segment: 0, part: 2, offset_ms: 0, words: spoken("Twenty minutes now.", 20 * MINUTE + 1_000) },
      { segment: 1, part: 0, offset_ms: 25 * MINUTE, words: spoken("New microphone.", 1_000) },
    ];
    const built = buildLiveRows(input({
      parts,
      speakers: [
        block(0, [turn(0, 5_000, id(0, 1)), turn(59_000, 70_000, id(0, 2))]),
        // The person who was Speaker 2 speaks first in the new window.
        block(1, [turn(20 * MINUTE, 21 * MINUTE, id(1, 1))]),
        block(2, [turn(25 * MINUTE, 26 * MINUTE, id(2, 1))]),
      ],
    }));
    expect(shape(built)).toEqual([
      [1, ["Hello there."]],
      [2, ["Hi."]],
      "divider",
      [1, ["Twenty minutes now."]],
      "divider",
      [1, ["New microphone."]],
    ]);
    expect(built.filter((item) => item.kind === "divider").map((item) => item.key))
      .toEqual(["divider/g:1", "divider/g:2"]);
  });

  test("text of a new segment before its turns arrive continues the last row in a new paragraph", () => {
    const built = buildLiveRows(input({
      parts: [
        { segment: 0, part: 0, offset_ms: 0, words: spoken("Hello there.") },
        { segment: 1, part: 0, offset_ms: 25 * MINUTE, words: spoken("New microphone.", 1_000) },
      ],
      speakers: [block(0, [turn(0, 5_000, id(0, 1))])],
    }));
    expect(shape(built)).toEqual([[1, ["Hello there.", "New microphone."]]]);
    expect(rows(built)[0].paragraphs[1].key).toBe("1/1000");
  });

  test("a word that overlaps 2 turns takes the one with the larger overlap, and the earlier one on a tie", () => {
    const words = [{ start_ms: 1_000, end_ms: 2_000, text: "word" }];
    const part = { segment: 0, part: 0, offset_ms: 0, words };
    const larger = buildLiveRows(input({
      parts: [part],
      speakers: [block(0, [turn(1_600, 3_000, id(0, 2)), turn(0, 1_600, id(0, 1))])],
    }));
    expect(rows(larger)[0].speaker).toBe(id(0, 1));
    const later = buildLiveRows(input({
      parts: [part],
      speakers: [block(0, [turn(0, 1_300, id(0, 1)), turn(1_300, 3_000, id(0, 2))])],
    }));
    expect(rows(later)[0].speaker).toBe(id(0, 2));
    const tie = buildLiveRows(input({
      parts: [part],
      speakers: [block(0, [turn(1_500, 3_000, id(0, 2)), turn(0, 1_500, id(0, 1))])],
    }));
    expect(rows(tie)[0].speaker).toBe(id(0, 1));
  });

  test("a turn with no words makes no heading", () => {
    const words = [...spoken("One two three"), ...spoken("four five", 1_300)];
    const built = buildLiveRows(input({
      parts: [{ segment: 0, part: 0, offset_ms: 0, words }],
      speakers: [block(0, [
        turn(0, 1_000, id(0, 1)),
        turn(1_000, 1_250, id(0, 2)),
        turn(1_250, 5_000, id(0, 1)),
      ])],
    }));
    expect(shape(built)).toEqual([[1, ["One two three four five"]]]);
  });

  test("an epoch with no turns makes no divider", () => {
    const built = buildLiveRows(input({
      parts: [
        { segment: 0, part: 0, offset_ms: 0, words: spoken("Hello there.") },
        { segment: 0, part: 1, offset_ms: 0, words: spoken("Still here.", 20 * MINUTE + 1_000) },
      ],
      speakers: [block(0, [turn(0, 5_000, id(0, 1))]), block(1, [])],
    }));
    expect(shape(built)).toEqual([[1, ["Hello there.", "Still here."]]]);
  });

  test("a segment start and an epoch start at the same point make one divider", () => {
    const built = buildLiveRows(input({
      parts: [
        { segment: 0, part: 0, offset_ms: 0, words: spoken("Hello there.") },
        { segment: 1, part: 0, offset_ms: 60_000, words: spoken("New microphone.") },
      ],
      speakers: [
        block(0, [turn(0, 5_000, id(0, 1))]),
        block(1, [turn(60_000, 70_000, id(1, 1))]),
      ],
    }));
    expect(shape(built)).toEqual([[1, ["Hello there."]], "divider", [1, ["New microphone."]]]);
  });

  test("a number in the map stays the same after a revision puts a different speaker first in the window", () => {
    const part = { segment: 0, part: 0, offset_ms: 0, words: spoken("One two three four") };
    const numbers = new Map<string, number>();
    const first = buildLiveRows(input({
      parts: [part], numbers,
      speakers: [block(0, [turn(0, 800, id(0, 1)), turn(800, 2_000, id(0, 2))])],
    }));
    expect(shape(first)).toEqual([[1, ["One two"]], [2, ["three four"]]]);
    const revised = buildLiveRows(input({
      parts: [part], numbers,
      speakers: [block(0, [turn(0, 800, id(0, 2)), turn(800, 2_000, id(0, 1))])],
    }));
    expect(shape(revised)).toEqual([[2, ["One two"]], [1, ["three four"]]]);
    expect([...numbers]).toEqual([[id(0, 1), 1], [id(0, 2), 2]]);
  });

  test("a new speaker gets the next number of its window and not a number of another window", () => {
    const numbers = new Map([[id(0, 1), 1], [id(0, 2), 2], [id(1, 7), 1]]);
    const built = buildLiveRows(input({
      parts: [{ segment: 0, part: 0, offset_ms: 0, words: spoken("One two") }],
      numbers,
      speakers: [block(1, [turn(0, 2_000, id(1, 3))])],
    }));
    expect(rows(built)[0].number).toBe(2);
  });
});

describe("the open part", () => {
  test("open-part text with no words shows under the last speaker, and the saved part splits into speaker rows (AE4)", () => {
    const earlier = { segment: 0, part: 0, offset_ms: 0, words: spoken("Shall we start?") };
    const speakers = [block(0, [
      turn(0, 1_200, id(0, 1)),
      turn(1_900, 2_700, id(0, 2)),
      turn(2_700, 5_000, id(0, 1)),
    ])];
    const open = buildLiveRows(input({
      parts: [earlier], speakers,
      open: [{ segment: 0, part: 1, offset_ms: 0, words: null, committed: "Yes let's. Good", tentative: "then" }],
    }));
    expect(shape(open)).toEqual([[1, ["Shall we start? Yes let's. Good then"]]]);
    const pieces = rows(open)[0].paragraphs[0].pieces;
    expect(pieces[pieces.length - 1]).toEqual({ text: " then", tentative: true });
    const saved = buildLiveRows(input({
      parts: [earlier, { segment: 0, part: 1, offset_ms: 0, words: spoken("Yes let's. Good then", 2_000) }],
      speakers,
    }));
    expect(shape(saved)).toEqual([
      [1, ["Shall we start?"]],
      [2, ["Yes let's."]],
      [1, ["Good then"]],
    ]);
  });

  test("a displayed word that the timed words do not have goes under the speaker before it (R3a)", () => {
    const words = spoken("we should now", 0);
    const built = buildLiveRows(input({
      open: [{ segment: 0, part: 0, offset_ms: 0, words, committed: "We should go", tentative: "now." }],
      speakers: [block(0, [turn(0, 700, id(0, 1)), turn(700, 2_000, id(0, 2))])],
    }));
    expect(shape(built)).toEqual([[1, ["We should go"]], [2, ["now."]]]);
  });

  test("an open part that a saved part replaces shows only once", () => {
    const words = spoken("Hello there.");
    const built = buildLiveRows(input({
      parts: [{ segment: 0, part: 0, offset_ms: 0, words }],
      open: [{ segment: 0, part: 0, offset_ms: 0, words, committed: "Hello there.", tentative: "" }],
    }));
    expect(shape(built)).toEqual([[null, ["Hello there."]]]);
  });

  test("unspaced open-part text aligns by character", () => {
    const words = [
      { start_ms: 0, end_ms: 300, text: "今日" },
      { start_ms: 400, end_ms: 700, text: "は" },
      { start_ms: 800, end_ms: 1_100, text: "晴れ" },
    ];
    const built = buildLiveRows(input({
      open: [{ segment: 0, part: 0, offset_ms: 0, words, committed: "今日は", tentative: "晴れ" }],
      speakers: [block(0, [turn(0, 750, id(0, 1)), turn(750, 2_000, id(0, 2))])],
    }));
    expect(shape(built)).toEqual([[1, ["今日は"]], [2, ["晴れ"]]]);
  });
});

describe("native rows and the post-Stop pass", () => {
  const native = (start_ms: number, end_ms: number, text: string, speaker: string, epoch = 0): NativeRow => ({
    generation: "g", epoch,
    start_sample: start_ms * 16, end_sample: end_ms * 16,
    text, speaker, text_final: true, speaker_final: true,
  });

  test("native CombinedNemo rows get the same paragraph rule, numbers and dividers (KTD9)", () => {
    const built = buildLiveRows(input({
      native: [
        native(0, 1_000, "Hello there.", id(0, 5)),
        native(3_100, 4_000, "Are you there?", id(0, 5)),
        native(4_100, 5_000, "Yes.", id(0, 6)),
        native(20 * MINUTE, 20 * MINUTE + 900, "Next window.", id(1, 6), 1),
      ],
    }));
    expect(shape(built)).toEqual([
      [1, ["Hello there.", "Are you there?"]],
      [2, ["Yes."]],
      "divider",
      [1, ["Next window."]],
    ]);
  });

  test("native rows with a tentative text show it as tentative", () => {
    const built = buildLiveRows(input({
      native: [native(0, 1_000, "Hello", id(0, 1)), { ...native(1_100, 1_500, "there", id(0, 1)), text_final: false }],
    }));
    expect(rows(built)[0].paragraphs[0].pieces).toEqual([
      { text: "Hello", tentative: false },
      { text: " there", tentative: true },
    ]);
  });

  test("native rows put no space before closing punctuation or after an opening bracket", () => {
    const built = buildLiveRows(input({
      native: [
        native(0, 500, "Hello", id(0, 1)),
        native(500, 600, ",", id(0, 1)),
        native(700, 900, "(yes", id(0, 1)),
        native(900, 1_000, ")", id(0, 1)),
      ],
    }));
    expect(paragraphText(rows(built)[0].paragraphs[0])).toBe("Hello, (yes)");
  });

  test("after Stop, a result epoch of the post-Stop pass changes no row and adds no divider (KTD13)", () => {
    const parts = [
      { segment: 0, part: 0, offset_ms: 0, words: spoken("Hello there.") },
      { segment: 0, part: 1, offset_ms: 0, words: spoken("Next window.", 20 * MINUTE + 1_000) },
    ];
    const live = [
      block(0, [turn(0, 5_000, id(0, 1))]),
      block(1, [turn(20 * MINUTE, 21 * MINUTE, id(1, 1), false)]),
    ];
    const expected = buildLiveRows(input({ parts, speakers: live, afterStop: true }));
    expect(shape(expected)).toEqual([[1, ["Hello there."]], "divider", [1, ["Next window."]]]);
    const withResult = buildLiveRows(input({
      parts, afterStop: true,
      speakers: [...live, block(2, [turn(0, 30 * MINUTE, id(2, 1))])],
    }));
    expect(withResult).toEqual(expected);
  });

  test("a row whose turn is provisional is marked provisional", () => {
    const built = buildLiveRows(input({
      parts: [{ segment: 0, part: 0, offset_ms: 0, words: spoken("One two three four") }],
      speakers: [block(0, [turn(0, 800, id(0, 1)), turn(800, 2_000, id(0, 2), false)])],
    }));
    expect(rows(built).map((row) => row.provisional)).toEqual([false, true]);
  });
});
