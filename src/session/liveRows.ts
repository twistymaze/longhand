/**
 * The row builder of the live transcript (KTD6). It takes the live state as
 * plain data and returns the speaker rows, their paragraphs, and the
 * dividers where the speaker labels restart. It draws nothing.
 */

/** The paragraph rule of `crates/pipeline/src/join.rs` (R4, KTD7). */
export const PARAGRAPH_CHARS = 400;
export const PARAGRAPH_PAUSE_MS = 2_000;
const SAMPLES_PER_MS = 16;

/** One word in milliseconds from the start of its capture segment. */
export type TimedWord = { start_ms: number; end_ms: number; text: string };
/** A saved part: its words and the start offset of its capture segment. */
export type SavedPart = { segment: number; part: number; offset_ms: number; words: TimedWord[] };
/**
 * The open part: the text that shows, and the timed words of the model's
 * current hypothesis. `words` is null when the model gives no word times.
 */
export type OpenPart = {
  segment: number;
  part: number;
  offset_ms: number;
  words: TimedWord[] | null;
  committed: string;
  tentative: string;
};
/** A native row of the CombinedNemo route, in 16 kHz note samples. */
export type NativeRow = {
  generation: string;
  epoch: number;
  start_sample: number;
  end_sample: number;
  text: string;
  speaker: string | null;
  text_final: boolean;
  speaker_final: boolean;
};
/** A speaker turn in 16 kHz note samples, as in `LiveRecognitionRow`. */
export type SpeakerTurn = {
  start_sample: number;
  end_sample: number;
  speaker: string | null;
  speaker_final: boolean;
};
/** The speaker turns of one speaker analysis window. */
export type SpeakerBlock = { generation: string; epoch: number; turns: SpeakerTurn[] };
/**
 * The shown speaker numbers, keyed by speaker id
 * (`generation:epoch:input_start:local_id`). The builder adds a number when
 * a heading first shows and does not change a number (KTD12).
 */
export type NumberMap = Map<string, number>;
export type LiveRowsInput = {
  parts: SavedPart[];
  open: OpenPart[];
  native: NativeRow[];
  speakers: SpeakerBlock[];
  numbers: NumberMap;
  /** True in processing and loading mode, after Stop (KTD13). */
  afterStop?: boolean;
};

/** Text that shows. Each piece starts with its separator space, if any. */
export type LivePiece = { text: string; tentative: boolean };
/** `key` is the group (segment) and the time of the first word. */
export type LiveParagraph = { key: string; pieces: LivePiece[] };
export type LiveSpeakerRow = {
  kind: "speaker";
  key: string;
  /** The speaker id, or null when no speaker turn applies. */
  speaker: string | null;
  number: number | null;
  /** True when a speaker turn of the row is not final, or a word has no turn yet. */
  provisional: boolean;
  paragraphs: LiveParagraph[];
};
export type LiveDivider = { kind: "divider"; key: string };
export type LiveRowItem = LiveSpeakerRow | LiveDivider;

/** The text of a paragraph. */
export const paragraphText = (paragraph: LiveParagraph) =>
  paragraph.pieces.map((piece) => piece.text).join("");

// The scripts with no spaces between words, as in `asr::stream::unspaced`.
const UNSPACED = /[　-ヿㇰ-ㇿ㐀-䶿一-鿿豈-﫿＀-￯\u{20000}-\u{2ffff}]/u;
const lastChar = (text: string) => {
  const chars = Array.from(text);
  return chars[chars.length - 1] ?? "";
};
const firstChar = (text: string) => Array.from(text)[0] ?? "";

/** True when a space goes between two pieces, as in `join::build`. */
function spaceBetween(left: string, right: string): boolean {
  return left !== "" && !UNSPACED.test(lastChar(left)) && !UNSPACED.test(firstChar(right));
}

/**
 * True when a paragraph is past the length limit and ends a sentence, so
 * that the next piece starts a new paragraph (`join::paragraph_full`).
 */
export function paragraphFull(text: string): boolean {
  if (Array.from(text).length <= PARAGRAPH_CHARS) return false;
  const end = text.replace(/["'”’)\]]+$/u, "");
  return /[.?!…。？！]$/u.test(end);
}

type Assigned = { scope: string; speaker: string | null; final: boolean };
/** One piece of text in the order that it shows. Times are note milliseconds. */
type Unit = {
  group: string;
  text: string;
  tentative: boolean;
  start: number | null;
  end: number | null;
  fixed: Assigned | null;
  /** The paragraph key when a paragraph starts at this piece. */
  key: string;
};
type Turn = SpeakerTurn & { scope: string; speaker: string };

const scopeOf = (generation: string, epoch: number) => `${generation}:${epoch}`;

/** Split text into words, and each character of an unspaced script into its own word. */
function tokens(text: string): string[] {
  const out: string[] = [];
  for (const chunk of text.split(/\s+/)) {
    let run = "";
    for (const char of Array.from(chunk)) {
      if (!UNSPACED.test(char)) {
        run += char;
        continue;
      }
      if (run) out.push(run);
      run = "";
      out.push(char);
    }
    if (run) out.push(run);
  }
  return out;
}
const normal = (token: string) => token.toLowerCase().replace(/\p{P}/gu, "");

/** The index in `timed` that each word of `shown` matches, by a word-level diff (KTD8). */
function align(shown: string[], timed: string[]): (number | null)[] {
  const match: (number | null)[] = shown.map(() => null);
  let head = 0;
  while (head < shown.length && head < timed.length && shown[head] === timed[head]) {
    match[head] = head;
    head++;
  }
  let tail = 0;
  while (
    tail < shown.length - head &&
    tail < timed.length - head &&
    shown[shown.length - 1 - tail] === timed[timed.length - 1 - tail]
  ) {
    match[shown.length - 1 - tail] = timed.length - 1 - tail;
    tail++;
  }
  const a = shown.slice(head, shown.length - tail);
  const b = timed.slice(head, timed.length - tail);
  if (!a.length || !b.length || a.length * b.length > 1_000_000) return match;
  // Longest common subsequence of the words between the equal head and tail.
  const width = b.length + 1;
  const table = new Uint32Array((a.length + 1) * width);
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      table[i * width + j] =
        a[i] === b[j]
          ? table[(i + 1) * width + j + 1] + 1
          : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
  for (let i = 0, j = 0; i < a.length && j < b.length; ) {
    if (a[i] === b[j]) {
      match[head + i] = head + j;
      i++;
      j++;
    } else if (table[(i + 1) * width + j] >= table[i * width + j + 1]) i++;
    else j++;
  }
  return match;
}

function savedUnits(part: SavedPart): Unit[] {
  const group = String(part.segment);
  return [...part.words]
    .sort((a, b) => a.start_ms - b.start_ms)
    .filter((word) => word.text.trim())
    .map((word) => ({
      group,
      text: word.text.trim(),
      tentative: false,
      start: part.offset_ms + word.start_ms,
      end: part.offset_ms + Math.max(word.end_ms, word.start_ms),
      fixed: null,
      key: `${group}/${word.start_ms}`,
    }));
}

function openUnits(part: OpenPart): Unit[] {
  const group = String(part.segment);
  const shown = [
    ...tokens(part.committed).map((text) => ({ text, tentative: false })),
    ...tokens(part.tentative).map((text) => ({ text, tentative: true })),
  ];
  const timed = (part.words ?? []).flatMap((word) => tokens(word.text).map(() => word));
  const timedTokens = (part.words ?? []).flatMap((word) => tokens(word.text));
  const match = align(shown.map((s) => normal(s.text)), timedTokens.map(normal));
  return shown.map((word, i) => {
    const time = match[i] === null ? null : timed[match[i]!];
    return {
      group,
      text: word.text,
      tentative: word.tentative,
      start: time ? part.offset_ms + time.start_ms : null,
      end: time ? part.offset_ms + Math.max(time.end_ms, time.start_ms) : null,
      fixed: null,
      // A word with no time has no time for its key.
      key: time ? `${group}/${time.start_ms}` : `${group}/p${part.part}/${i}`,
    };
  });
}

function nativeUnits(rows: NativeRow[]): Unit[] {
  return [...rows]
    .sort((a, b) => a.start_sample - b.start_sample || a.end_sample - b.end_sample)
    .filter((row) => row.text.trim())
    .map((row) => ({
      group: "native",
      text: row.text.trim(),
      tentative: !row.text_final,
      start: row.start_sample / SAMPLES_PER_MS,
      end: Math.max(row.end_sample, row.start_sample) / SAMPLES_PER_MS,
      fixed: {
        scope: scopeOf(row.generation, row.epoch),
        speaker: row.speaker,
        final: row.speaker_final,
      },
      key: `native/${row.start_sample / SAMPLES_PER_MS}`,
    }));
}

const overlaps = (a: SpeakerTurn, b: SpeakerTurn) =>
  a.start_sample < b.end_sample && b.start_sample < a.end_sample;

/**
 * The turns that the rows use. After Stop, an epoch whose turns overlap the
 * turns of a lower-numbered epoch is a result of the post-Stop pass, and the
 * rows ignore it (KTD13).
 */
function usedTurns(blocks: SpeakerBlock[], afterStop: boolean): Turn[] {
  const ordered = [...blocks].sort((a, b) => a.epoch - b.epoch);
  const turns: Turn[] = [];
  ordered.forEach((block, i) => {
    if (
      afterStop &&
      block.turns.some((turn) =>
        ordered.slice(0, i).some((lower) => lower.epoch < block.epoch && lower.turns.some((t) => overlaps(turn, t))),
      )
    )
      return;
    const scope = scopeOf(block.generation, block.epoch);
    for (const turn of block.turns)
      if (turn.speaker !== null && turn.end_sample > turn.start_sample)
        turns.push({ ...turn, scope, speaker: turn.speaker });
  });
  return turns.sort((a, b) => a.start_sample - b.start_sample || a.end_sample - b.end_sample);
}

/** The turn with the largest sample overlap; on a tie, the earlier turn (KTD10). */
function turnFor(unit: Unit, turns: Turn[]): Turn | null {
  if (unit.start === null || unit.end === null) return null;
  const start = unit.start * SAMPLES_PER_MS;
  // A word with no length counts as one sample.
  const end = Math.max(unit.end * SAMPLES_PER_MS, start + 1);
  let best: Turn | null = null;
  let bestOverlap = 0;
  for (const turn of turns) {
    if (turn.start_sample >= end) break;
    const overlap = Math.min(end, turn.end_sample) - Math.max(start, turn.start_sample);
    if (overlap > bestOverlap) {
      best = turn;
      bestOverlap = overlap;
    }
  }
  return best;
}

/** The shown number of a speaker: the kept one, or the next free one in its window (KTD12). */
function numberFor(numbers: NumberMap, scope: string, speaker: string): number {
  const kept = numbers.get(speaker);
  if (kept !== undefined) return kept;
  let highest = 0;
  for (const [id, number] of numbers)
    if (id.startsWith(`${scope}:`)) highest = Math.max(highest, number);
  numbers.set(speaker, highest + 1);
  return highest + 1;
}

/** Build the speaker rows, paragraphs and dividers of the live transcript. */
export function buildLiveRows(input: LiveRowsInput): LiveRowItem[] {
  const saved = new Set(input.parts.map((part) => `${part.segment}/${part.part}`));
  const parts = [
    ...input.parts.map((part) => ({ part, units: () => savedUnits(part) })),
    ...input.open
      .filter((part) => !saved.has(`${part.segment}/${part.part}`))
      .map((part) => ({ part, units: () => openUnits(part) })),
  ].sort((a, b) => a.part.segment - b.part.segment || a.part.part - b.part.part);
  let units = parts.flatMap((entry) => entry.units());
  const native = nativeUnits(input.native);
  if (native.length && units.length) {
    // Both routes at once: order by time, and keep a word with no time after the word before it.
    let carried = 0;
    const keyed = [...units, ...native].map((unit) => {
      if (unit.start !== null) carried = unit.start;
      return { unit, at: carried };
    });
    units = keyed.sort((a, b) => a.at - b.at).map((entry) => entry.unit);
  } else if (native.length) units = native;

  const turns = usedTurns(input.speakers, input.afterStop ?? false);
  const items: LiveRowItem[] = [];
  let row: LiveSpeakerRow | null = null;
  let paragraph: LiveParagraph | null = null;
  let paragraphChars = "";
  let previous: Assigned | null = null;
  let group: string | null = null;
  let lastEnd: number | null = null;

  for (const unit of units) {
    const turn = unit.fixed ? null : turnFor(unit, turns);
    const assigned: Assigned | null =
      unit.fixed ?? (turn ? { scope: turn.scope, speaker: turn.speaker, final: turn.speaker_final } : previous);
    // A word with no turn continues the speaker before it (R6), and that speaker can still change.
    const inherited = !unit.fixed && !turn && assigned !== null;
    const scope = assigned?.scope ?? null;
    const speaker = assigned?.speaker ?? null;

    // The divider goes before the first word of the first turn of a new epoch (KTD11).
    const divider = scope !== null && previous !== null && previous.scope !== scope;
    if (divider) items.push({ kind: "divider", key: `divider/${scope}` });
    const newGroup = group !== unit.group;
    if (newGroup) lastEnd = null;
    group = unit.group;
    const paused =
      unit.start !== null && lastEnd !== null && unit.start - lastEnd > PARAGRAPH_PAUSE_MS;
    if (unit.start !== null) lastEnd = Math.max(lastEnd ?? unit.end!, unit.end!);
    const newRow = !row || divider || row.speaker !== speaker;
    const newParagraph = newRow || newGroup || paused || paragraphFull(paragraphChars);

    if (newParagraph) {
      paragraph = { key: unit.key, pieces: [{ text: unit.text, tentative: unit.tentative }] };
      paragraphChars = unit.text;
    } else {
      const space = spaceBetween(paragraphChars, unit.text) ? " " : "";
      paragraphChars += space + unit.text;
      const last = paragraph!.pieces[paragraph!.pieces.length - 1];
      if (last.tentative === unit.tentative) last.text += space + unit.text;
      else paragraph!.pieces.push({ text: space + unit.text, tentative: unit.tentative });
    }
    if (newRow) {
      row = {
        kind: "speaker",
        key: `row/${paragraph!.key}`,
        speaker,
        number: speaker !== null && scope !== null ? numberFor(input.numbers, scope, speaker) : null,
        provisional: false,
        paragraphs: [],
      };
      items.push(row);
    }
    if (newParagraph) row!.paragraphs.push(paragraph!);
    if (assigned && (inherited || !assigned.final)) row!.provisional = true;
    previous = assigned;
  }
  return items;
}
