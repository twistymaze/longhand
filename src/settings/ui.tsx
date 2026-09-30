// Primitives after macOS's grouped settings form: inset groups of rows, a
// title and a description on the left and the control on the right, and
// footnotes under a group.

import {
  type MouseEvent,
  type ReactNode,
  type Ref,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { ConfirmDialog } from "../lib/ConfirmDialog";

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
};

// Pressed state is instant: feedback belongs to pointer-down, not release.
export function Button({ children, onClick, disabled, prominent, destructive, ref }: ButtonProps) {
  const tone = destructive
    ? "bg-negative text-white shadow-[0_1px_1.5px_rgb(0_0_0/0.14)] active:opacity-80"
    : prominent
      ? "bg-accent text-white shadow-[0_1px_1.5px_rgb(0_0_0/0.14)] active:bg-accent-pressed"
      : "bg-fill text-label shadow-[0_0_0_0.5px_var(--separator)] active:bg-fill-pressed";
  return (
    <button
      ref={ref}
      type="button"
      className={`rounded-[7px] px-3 py-[3px] font-medium whitespace-nowrap select-none ${tone} outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40`}
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
    <span className={`inline-flex items-center gap-1.5 ${toneClass[tone]}`}>
      <span aria-hidden className={`size-2 rounded-full ${dotClass[tone]}`} />
      {children}
    </span>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  return <p className="px-3 pb-2 text-[0.85rem] text-negative">{children}</p>;
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

/// A text field that commits on Return or when focus leaves, the way macOS
/// settings fields apply without a Save button.
export function CommitField({
  value,
  placeholder,
  onCommit,
  inputMode,
  width = "w-64",
  label,
  disabled,
}: {
  value: string;
  placeholder?: string;
  onCommit: (v: string) => void;
  inputMode?: "numeric";
  width?: string;
  label: string;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
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
