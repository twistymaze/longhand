// Version order and version ranges, with no npm package.
//
// A version is plain `x.y.z`. A range is comparators (`<`, `<=`, `>`, `>=`,
// `=`, or a version with no operator) joined by spaces, and groups of
// comparators joined by `||`. The range `*` contains every version. Any other
// syntax, for example `^`, `~`, `x` placeholders or prerelease tags, is an
// error, because a guess can give the wrong answer.

type Parts = [number, number, number];

const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const COMPARATOR = /^(<=|>=|<|>|=)?(\d+\.\d+\.\d+)$/;

function parse(version: string): Parts {
  const match = VERSION.exec(version);
  if (!match) throw new Error(`version "${version}" is not a plain x.y.z version`);
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

/// -1 when a is before b, 0 when they are equal, 1 when a is after b.
/// Use it as the compare function of Array.prototype.sort.
export function compareVersions(a: string, b: string): -1 | 0 | 1 {
  const pa = parse(a);
  const pb = parse(b);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  }
  return 0;
}

function test(order: number, operator: string): boolean {
  switch (operator) {
    case "<":
      return order < 0;
    case "<=":
      return order <= 0;
    case ">":
      return order > 0;
    case ">=":
      return order >= 0;
    default:
      return order === 0;
  }
}

/// True when the version is in the range.
export function satisfies(version: string, range: string): boolean {
  parse(version);
  if (range.trim() === "*") return true;
  const groups = range.split("||").map((group) => group.trim().split(/\s+/));
  // Read the full range first, so that an error does not depend on the version.
  const parsed = groups.map((comparators) =>
    comparators.map((comparator) => {
      const match = COMPARATOR.exec(comparator);
      if (!match) throw new Error(`range "${range}": "${comparator}" is not a comparator with a plain x.y.z version`);
      parse(match[2]);
      return { operator: match[1] ?? "=", bound: match[2] };
    }),
  );
  return parsed.some((comparators) =>
    comparators.every(({ operator, bound }) => test(compareVersions(version, bound), operator)),
  );
}
