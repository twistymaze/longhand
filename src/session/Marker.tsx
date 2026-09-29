// A marker line in a transcript: a label with a line on each side. The saved
// transcript uses it for gaps and silences, and the live transcript uses it
// where the speaker labels restart.

export function Marker({ label }: { label: string }) {
  return (
    <div className="my-5 flex items-center gap-3 text-[0.85rem] text-label-tertiary">
      <span className="h-px flex-1 bg-separator" />
      {label}
      <span className="h-px flex-1 bg-separator" />
    </div>
  );
}
