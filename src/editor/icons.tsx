// Inline SVG icons for the editor toolbar and the `/` menu. Each icon draws
// in `currentColor` on a 16 × 16 grid.
import type { ReactNode } from "react";

type IconProps = { className?: string };

function Icon({ className, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      {children}
    </svg>
  );
}

export function BoldIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4.5 2.75h4.25a2.5 2.5 0 0 1 0 5H4.5zM4.5 7.75h5a2.75 2.75 0 0 1 0 5.5h-5z" strokeWidth="1.75" />
    </Icon>
  );
}

export function ItalicIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M6.5 2.75h5M4.5 13.25h5M9.75 2.75l-3.5 10.5" />
    </Icon>
  );
}

export function StrikeIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M2.5 8h11M11 4.25C10.5 3.3 9.4 2.75 8 2.75c-1.9 0-3.25 1-3.25 2.4 0 .8.4 1.4 1.1 1.85M5 11.75c.5.95 1.6 1.5 3 1.5 1.9 0 3.25-1 3.25-2.4 0-.35-.07-.65-.2-.95" />
    </Icon>
  );
}

export function HighlightIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M9.5 2.75l3.75 3.75-5.5 5.5H4v-3.75zM2.5 13.25h11" />
    </Icon>
  );
}

export function LinkIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M6.75 9.25a2.75 2.75 0 0 0 3.9 0l2.1-2.1a2.75 2.75 0 0 0-3.9-3.9l-.6.6M9.25 6.75a2.75 2.75 0 0 0-3.9 0l-2.1 2.1a2.75 2.75 0 0 0 3.9 3.9l.6-.6" />
    </Icon>
  );
}

export function TextIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3 3.25h10M8 3.25v9.5" />
    </Icon>
  );
}

function HeadingIcon({ level, ...props }: IconProps & { level: 1 | 2 | 3 }) {
  const digit = {
    1: "M11.5 7.25l1.5-1v7",
    2: "M10.75 7a1.4 1.4 0 0 1 2.7.5c0 1.2-2.7 2.6-2.7 5.25h2.75",
    3: "M10.75 6.5h2.5l-1.4 2c1 0 1.65.6 1.65 1.5 0 1-.8 1.75-1.8 1.75-.5 0-1-.2-1.2-.5",
  }[level];
  return (
    <Icon {...props}>
      <path d={`M2.5 3.25v9.5M8 3.25v9.5M2.5 8H8 ${digit}`} />
    </Icon>
  );
}

export function Heading1Icon(props: IconProps) {
  return <HeadingIcon level={1} {...props} />;
}

export function Heading2Icon(props: IconProps) {
  return <HeadingIcon level={2} {...props} />;
}

export function Heading3Icon(props: IconProps) {
  return <HeadingIcon level={3} {...props} />;
}

export function BulletListIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M6.5 4h7M6.5 8h7M6.5 12h7" />
      <circle cx="3" cy="4" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="3" cy="8" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="3" cy="12" r="0.9" fill="currentColor" stroke="none" />
    </Icon>
  );
}

export function NumberedListIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M6.5 4h7M6.5 8h7M6.5 12h7M2.5 3l1-.5v3M2.4 7.2a.8.8 0 0 1 1.5.35c0 .7-1.5 1.3-1.5 2.2h1.6" strokeWidth="1.25" />
    </Icon>
  );
}

export function TaskListIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="2" y="2.5" width="4" height="4" rx="1" />
      <path d="M8.5 4.5h5M8.5 11.5h5M2.5 11.25l1.25 1.25L6 10" />
    </Icon>
  );
}

export function QuoteIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3 3v10M6.5 5h6.5M6.5 8h6.5M6.5 11h4" />
    </Icon>
  );
}

export function CodeIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M5.5 4.5L2 8l3.5 3.5M10.5 4.5L14 8l-3.5 3.5" />
    </Icon>
  );
}

export function DividerIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M2 8h12" />
    </Icon>
  );
}
