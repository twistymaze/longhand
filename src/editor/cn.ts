// Join class names and drop the false and empty ones. Longhand's local helper
// for the copied editor components, which used a shared `cn`.
export function cn(classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(" ");
}
