// Primitives after macOS's grouped settings form: inset groups of rows, a
// label on the left and its control on the right, footnotes under a group.

import type { ReactNode } from "react";

export function Pane({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mx-auto max-w-[34rem] px-6 pt-5 pb-8">
      <h1 className="mb-4 text-[1.35rem] leading-tight font-semibold tracking-[-0.01em]">
        {title}
      </h1>
      <div className="grid gap-5">{children}</div>
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
      {header && <h2 className="mb-1.5 px-1 font-semibold">{header}</h2>}
      <div className="divide-y divide-separator overflow-hidden rounded-[10px] bg-group shadow-[0_0_0_0.5px_var(--separator)]">
        {children}
      </div>
      {footer && (
        <div className="mt-1.5 px-1 text-[0.85rem] text-label-secondary">{footer}</div>
      )}
    </section>
  );
}

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
    <div className="flex min-h-10 items-center gap-3 px-3 py-2">
      <div className="min-w-0 flex-1">
        <div>{label}</div>
        {detail && <div className="text-[0.85rem] text-label-secondary">{detail}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

type ButtonProps = {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  prominent?: boolean;
};

// Pressed state is instant: feedback belongs to pointer-down, not release.
export function Button({ children, onClick, disabled, prominent }: ButtonProps) {
  const tone = prominent
    ? "bg-accent text-white active:bg-accent-pressed"
    : "bg-fill text-label active:bg-fill-pressed";
  return (
    <button
      type="button"
      className={`rounded-md px-2.5 py-[3px] whitespace-nowrap select-none ${tone} outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40`}
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

export function ProgressBar({ fraction }: { fraction: number }) {
  const percent = Math.round(Math.min(1, Math.max(0, fraction)) * 100);
  return (
    <div
      role="progressbar"
      aria-valuenow={percent}
      aria-valuemin={0}
      aria-valuemax={100}
      className="h-1.5 w-28 overflow-hidden rounded-full bg-fill-pressed"
    >
      <div className="progress-fill h-full rounded-full bg-accent" style={{ width: `${percent}%` }} />
    </div>
  );
}

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
