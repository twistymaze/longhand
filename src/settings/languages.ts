const names = new Intl.DisplayNames(["en"], { type: "language" });

export const languageName = (code: string): string => names.of(code) ?? code;

export function describeLanguages(codes: string[]): string {
  return codes.length > 2
    ? `${codes.length} languages`
    : codes.map(languageName).join(" and ");
}
