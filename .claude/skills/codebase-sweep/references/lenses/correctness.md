# Lens: correctness

Find logic that gives a wrong result for an input or state that can occur.

## Look for

- Wrong conditions: inverted checks, `<` for `<=`, a missing case in a `match` or `switch`, a fallback branch that hides a new state.
- Off-by-one errors in indexes, ranges, durations, sample counts, and offsets.
- Wrong units or conversions: samples and seconds, milliseconds and seconds, bytes and characters, UTF-8 and UTF-16 offsets, local time and UTC.
- Values that can be empty, zero, negative, or very large where the code assumes otherwise.
- Swapped arguments, a wrong field or variable with a similar name, and copy-paste errors.
- State machines with a transition that is not handled, or that leaves state inconsistent.
- Parsing and serialization that do not round-trip: a value written in one form and read in another, or an old file format that the reader no longer accepts.
- Text processing that breaks on Unicode, line ends (CRLF), or empty input.

## Leave to other lenses

Races (`concurrency`), lost data (`data-loss`), failure paths (`error-recovery`), lifetimes (`resources`).
