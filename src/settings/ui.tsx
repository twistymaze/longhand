// Primitives after macOS's grouped settings form: inset groups of rows, a
// title and a description on the left and the control on the right, and
// footnotes under a group.

import {
  type MouseEvent,
  type ReactNode,
  type Ref,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { registerCloseSave } from "./closeSaves";

export function Pane({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mx-auto settings-pane w-full max-w-[56rem] px-[clamp(0.5rem,3vw,2rem)] pt-7 pb-10">
      <h1 className="mb-5 text-[1.6rem] leading-tight font-bold tracking-[-0.025em]">{title}</h1>
      {/* minmax(0, 1fr): a group never grows past the pane to fit a line that does not wrap. */}
      <div className="grid grid-cols-[minmax(0,1fr)] gap-6">{children}</div>
    </div>
  );
}

export function Group({
  header,
  footer,
  children,
}: {
  header?: string;
  footer?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section>
      {header && <h2 className="mb-2 px-1 text-[0.85rem] font-semibold text-label-secondary">{header}</h2>}
      <div className="overflow-hidden rounded-[12px] bg-group shadow-[0_0_0_0.5px_var(--separator),0_1px_2px_rgb(0_0_0/0.04)]">
        {children}
      </div>
      {footer && (
        <div className="mt-2 px-1 text-[0.8rem] leading-[1.45] text-label-secondary">{footer}</div>
      )}
    </section>
  );
}

// Rows after the first have a separator inset to the text, as in macOS.
export function Row({
  label,
  detail,
  children,
}: {
  label: ReactNode;
  detail?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="settings-row px-4">
      <div className="settings-row-content flex min-h-[3.25rem] flex-wrap items-center gap-4 py-2.5">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{label}</div>
          {detail && <div className="mt-0.5 text-[0.85rem] leading-[1.4] text-label-secondary">{detail}</div>}
        </div>
        {children && <div className="settings-row-controls flex min-w-0 max-w-full flex-wrap items-center gap-2">{children}</div>}
      </div>
    </div>
  );
}

type ButtonProps = {
  children: ReactNode;
  onClick?: (e: MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
  prominent?: boolean;
  /// A red button, for the action that deletes.
  destructive?: boolean;
  ref?: Ref<HTMLButtonElement>;
  /// The button is one part of a split button: the main part ("start") or
  /// the menu part ("end"). The two parts touch and share one outline.
  joined?: "start" | "end";
  /// The accessible name, for a button that shows only an icon.
  label?: string;
  /// The button opens a menu, which is open when this is true.
  expanded?: boolean;
};

const shapes = {
  none: "rounded-[7px] px-3",
  start: "rounded-l-[7px] pr-2.5 pl-3",
  end: "rounded-r-[7px] px-1.5",
};

// Pressed state is instant: feedback belongs to pointer-down, not release.
export function Button({ children, onClick, disabled, prominent, destructive, ref, joined, label, expanded }: ButtonProps) {
  const tone = destructive
    ? "bg-negative text-white shadow-[0_1px_1.5px_rgb(0_0_0/0.14)] active:opacity-80"
    : prominent
      ? "bg-accent text-white shadow-[0_1px_1.5px_rgb(0_0_0/0.14)] active:bg-accent-pressed"
      : "bg-fill text-label shadow-[0_0_0_0.5px_var(--separator)] active:bg-fill-pressed";
  return (
    <button
      ref={ref}
      type="button"
      aria-label={label}
      aria-haspopup={expanded === undefined ? undefined : "menu"}
      aria-expanded={expanded}
      className={`${shapes[joined ?? "none"]} py-[3px] font-medium whitespace-nowrap select-none ${tone} outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40`}
      onClick={onClick}
      disabled={disabled}
    >
      {children}
    </button>
  );
}

export const fieldClass =
  "rounded-md border border-control-border bg-control px-2 py-[3px] text-label placeholder:text-label-tertiary outline-offset-0 focus:outline-3 focus:outline-accent/50";

export type Tone = "positive" | "caution" | "negative" | "neutral";

const toneClass: Record<Tone, string> = {
  positive: "text-positive",
  caution: "text-caution",
  negative: "text-negative",
  neutral: "text-label-secondary",
};

const dotClass: Record<Tone, string> = {
  positive: "bg-positive",
  caution: "bg-caution",
  negative: "bg-negative",
  neutral: "bg-label-tertiary",
};

export function Status({ tone, children }: { tone: Tone; children: ReactNode }) {
  return (
    // The dot is centred on the first line of text, also when the text wraps.
    <span className={`inline-flex items-start gap-1.5 ${toneClass[tone]}`}>
      <span aria-hidden className="flex h-[1lh] shrink-0 items-center">
        <span className={`size-2 rounded-full ${dotClass[tone]}`} />
      </span>
      <span className="min-w-0">{children}</span>
    </span>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  return <p className="px-3 pb-2 text-[0.85rem] text-negative">{children}</p>;
}

/// The notice that a shared text changed on another computer. `name` is the
/// field name in the notice. `suffix` adds text to each button name, for
/// example a field name for screen readers.
export function ChangedElsewhereNotice({
  name,
  suffix,
  onKeep,
  onReload,
}: {
  name: string;
  suffix?: ReactNode;
  onKeep: () => void;
  onReload: () => void;
}) {
  return (
    <div className="settings-row px-4">
      <div className="flex min-h-[3.25rem] flex-wrap items-center gap-2 py-2.5">
        <div role="status" className="min-w-0 flex-1 text-[0.85rem] leading-[1.4]">
          <p className="font-medium text-caution">{`The ${name} changed on another computer.`}</p>
          <p className="text-label-secondary">Reload shows the new text. Keep My Text saves your text.</p>
        </div>
        <Button onClick={onKeep}>
          Keep My Text{suffix}
        </Button>
        <Button prominent onClick={onReload}>
          Reload{suffix}
        </Button>
      </div>
    </div>
  );
}

/// A bar that fills. With `active`, a quiet sweep says that work continues
/// that the fill does not show; `fraction` null has no fill yet.
export function ProgressBar({
  fraction,
  label,
  valueText,
  active = false,
}: {
  fraction: number | null;
  label?: string;
  valueText?: string;
  active?: boolean;
}) {
  const percent = fraction === null ? null : Math.round(Math.min(1, Math.max(0, fraction)) * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuenow={percent ?? undefined}
      aria-valuetext={valueText}
      aria-valuemin={0}
      aria-valuemax={100}
      className={`relative h-1.5 w-28 overflow-hidden rounded-full bg-fill-pressed${active ? " progress-active" : ""}`}
    >
      <div className="progress-fill h-full rounded-full bg-accent" style={{ width: `${percent ?? 0}%` }} />
    </div>
  );
}

// An on/off switch after the macOS switch. The knob moves to its new side
// with no overshoot, and without motion when the user asks for reduced
// motion.
export function Switch({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative h-[18px] w-[32px] shrink-0 rounded-full transition-colors duration-150 select-none ${
        checked ? "bg-accent" : "bg-fill-pressed"
      } outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40`}
    >
      <span
        aria-hidden
        className={`absolute top-[2px] left-[2px] size-[14px] rounded-full bg-white shadow-[0_0.5px_1.5px_rgb(0_0_0/0.3)] transition-transform duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none ${
          checked ? "translate-x-[14px]" : ""
        }`}
      />
    </button>
  );
}

/// A small label beside a row title, for example "Recommended".
export function Tag({ children, accent }: { children: ReactNode; accent?: boolean }) {
  return (
    <span
      className={`rounded-[4px] px-[5px] py-px text-[0.72rem] leading-[1.3] font-medium whitespace-nowrap ${
        accent ? "bg-accent/12 text-accent" : "bg-fill text-label-secondary"
      }`}
    >
      {children}
    </span>
  );
}

/// A count of notes in words, such as "1 note" or "3 notes".
export const notes = (n: number) => `${n} ${n === 1 ? "note" : "notes"}`;

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} KB`;
  return `${bytes} bytes`;
}

/// The last component of a path, for a row title; the full path goes beneath.
export function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export type Question = { title: string; message: string; action: string };

/// `ask` shows a question and resolves to true when the user confirms.
/// `dialog` is the question to render, or null. A new question cancels the
/// question that is open.
export function useConfirm() {
  type Pending = Question & { answer: (yes: boolean) => void };
  const [pending, setPending] = useState<Pending | null>(null);
  const open = useRef<Pending | null>(null);
  const ask = useCallback(
    (question: Question) =>
      new Promise<boolean>((resolve) => {
        open.current?.answer(false);
        const next: Pending = {
          ...question,
          answer: (yes) => {
            if (open.current === next) {
              open.current = null;
              setPending(null);
            }
            resolve(yes);
          },
        };
        open.current = next;
        setPending(next);
      }),
    [],
  );
  const dialog = pending && (
    <ConfirmDialog
      title={pending.title}
      message={pending.message}
      action={pending.action}
      onConfirm={() => pending.answer(true)}
      onCancel={() => pending.answer(false)}
    />
  );
  return { ask, dialog };
}

/// A name field that opens for one edit, for a rename or a new name (R15,
/// R16, KTD7). It takes the focus and selects its text when it opens.
/// Return gives the text to `onApply`; the owner closes the field when the
/// name applies. When `onApply` gives false, the field stays open with the
/// text and the focus. Escape and a click elsewhere call `onCancel`, so a
/// name changes only on Return. `byKey` is true for Escape: the owner then
/// gives the focus back to the control that opened the field.
export function NameField({
  value,
  label,
  placeholder,
  onApply,
  onCancel,
}: {
  value: string;
  label: string;
  placeholder?: string;
  onApply: (v: string) => Promise<boolean>;
  onCancel: (byKey: boolean) => void;
}) {
  const [draft, setDraft] = useState(value);
  const field = useRef<HTMLInputElement>(null);
  // A blur while the command runs does not cancel the edit.
  const applying = useRef(false);
  useLayoutEffect(() => {
    field.current?.focus();
    field.current?.select();
  }, []);
  const apply = async () => {
    if (applying.current) return;
    applying.current = true;
    const done = await onApply(draft);
    applying.current = false;
    if (!done) field.current?.focus();
  };
  return (
    <input
      ref={field}
      aria-label={label}
      className={`${fieldClass} w-full`}
      value={draft}
      placeholder={placeholder}
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => {
        if (!applying.current) onCancel(false);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          void apply();
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onCancel(true);
        }
      }}
    />
  );
}

/// Why a CommitField commits: the user pressed Return or left the field, or
/// the field saves with no user action (a window close, a pane or prompt
/// change). An unattended commit must not ask the user anything.
export type CommitHow = "user" | "unattended";

/// A text field that commits on Return or when focus leaves, the way macOS
/// settings fields apply without a Save button. It also commits the typed
/// text at a window close and when it unmounts, because no blur comes then.
/// `onCommit` resolves to true when the value was saved.
export function CommitField({
  value,
  placeholder,
  onCommit,
  onDraft,
  inputMode,
  width = "w-64",
  label,
  disabled,
}: {
  value: string;
  placeholder?: string;
  onCommit: (v: string, how: CommitHow) => Promise<boolean>;
  /// Receives the typed text that differs from the value, or null.
  onDraft?: (draft: string | null) => void;
  inputMode?: "numeric";
  width?: string;
  label: string;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState(value);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const onCommitRef = useRef(onCommit);
  onCommitRef.current = onCommit;
  // The value that the field shows when it has no edit.
  const shown = useRef(value);
  // The text of the last commit that was sent, and of the last commit that
  // saved. The commits of the field run one after the other.
  const sentText = useRef<string | null>(null);
  const savedText = useRef<string | null>(null);
  const chain = useRef<Promise<boolean>>(Promise.resolve(true));
  const running = useRef(0);

  // A new value replaces the draft only when the draft is not the user's
  // own edit: it is the value before, or a text that a commit saved. A
  // change from another computer keeps the text that the user types.
  useEffect(() => {
    if (value === shown.current) return;
    const before = shown.current;
    shown.current = value;
    sentText.current = null;
    setDraft((d) => (d === before || d === savedText.current ? value : d));
  }, [value]);

  const flush = useCallback((how: CommitHow) => {
    const text = draftRef.current;
    // An unattended commit does not send a text again that a commit sent:
    // the user saw its result. Return sends it again when no commit runs.
    const sentBefore = text === sentText.current && (how === "unattended" || running.current > 0);
    if (text !== shown.current && !sentBefore) {
      sentText.current = text;
      const valueAtSend = shown.current;
      running.current += 1;
      chain.current = chain.current.then(async () => {
        let saved = false;
        try {
          saved = await onCommitRef.current(text, how);
        } catch (e) {
          console.error(`The ${label} could not be saved:`, e);
        } finally {
          running.current -= 1;
        }
        if (saved) {
          savedText.current = text;
          // The value changed before the save returned: show it.
          if (draftRef.current === text && shown.current !== valueAtSend) setDraft(shown.current);
        }
        return saved;
      });
    }
    return chain.current;
  }, [label]);

  // React fires no blur when the field unmounts, so the unmount commits the
  // typed text. A window close during that commit waits for it.
  useEffect(
    () => () => {
      if (draftRef.current === shown.current && running.current === 0) return;
      const remove = registerCloseSave(() => chain.current);
      void flush("unattended").finally(remove);
    },
    [flush],
  );
  // While the field has typed text, a window close commits it.
  const dirty = draft !== value;
  useEffect(() => {
    if (!dirty) return;
    return registerCloseSave(() => flush("unattended"));
  }, [dirty, flush]);
  const onDraftRef = useRef(onDraft);
  onDraftRef.current = onDraft;
  useEffect(() => onDraftRef.current?.(dirty ? draft : null), [dirty, draft]);

  const commit = () => void flush("user");
  return (
    <input
      aria-label={label}
      className={`${fieldClass} ${width} disabled:text-label-secondary`}
      value={draft}
      disabled={disabled}
      placeholder={placeholder}
      inputMode={inputMode}
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") setDraft(value);
      }}
    />
  );
}
