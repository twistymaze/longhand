// A marker line in a transcript: a label with a line on each side. The saved
// transcript uses it for gaps, pauses and silences, and the live transcript
// uses it where the speaker labels restart. A pause marker has dashed lines,
// so that it reads differently from a "Resumed …" marker: a pause is a break
// inside one recording, not a new recording.

export function Marker({ label, dashed = false }: { label: string; dashed?: boolean }) {
  const line = dashed ? "h-0 flex-1 border-t border-dashed border-separator" : "h-px flex-1 bg-separator";
  return (
    <div className="my-5 flex items-center gap-3 text-[0.85rem] text-label-tertiary">
      <span className={line} />
      {label}
      <span className={line} />
    </div>
  );
}
