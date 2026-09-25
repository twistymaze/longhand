// Primitives after macOS's grouped settings form: inset groups of rows, a
// title and a description on the left and the control on the right, and
// footnotes under a group.

import type { ReactNode } from "react";

export function Pane({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mx-auto max-w-[40rem] px-8 pt-7 pb-10">
      <h1 className="mb-5 text-[1.6rem] leading-tight font-bold tracking-[-0.025em]">{title}</h1>
      <div className="grid gap-6">{children}</div>
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
      <div className="flex min-h-[3.25rem] items-center gap-4 py-2.5">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{label}</div>
          {detail && <div className="mt-0.5 text-[0.85rem] leading-[1.4] text-label-secondary">{detail}</div>}
        </div>
        {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
      </div>
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
    ? "bg-accent text-white shadow-[0_1px_1.5px_rgb(0_0_0/0.14)] active:bg-accent-pressed"
    : "bg-fill text-label shadow-[0_0_0_0.5px_var(--separator)] active:bg-fill-pressed";
  return (
    <button
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
