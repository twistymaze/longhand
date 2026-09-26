// Small line icons in the style of SF Symbols, for Home and the Settings
// sidebar. Each icon draws on a 16-unit grid with the current text colour.

import type { ReactNode } from "react";
import appIcon from "../assets/longhand-icon.png";

function Icon({ children, className = "size-4" }: { children: ReactNode; className?: string }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.3}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`shrink-0 ${className}`}
    >
      {children}
    </svg>
  );
}

type IconProps = { className?: string };

/// The app icon: the dark rounded square with the cream "L.".
export function AppIcon({ className = "size-7" }: IconProps) {
  return <img src={appIcon} alt="" aria-hidden draggable={false} className={`shrink-0 ${className}`} />;
}

export const PlusIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M8 3.5v9M3.5 8h9" strokeWidth={1.6} />
  </Icon>
);

/// A page with lines of text: one note.
export const NoteIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M4.5 1.75h4.75l2.75 2.75v8.25a1.5 1.5 0 0 1-1.5 1.5h-6a1.5 1.5 0 0 1-1.5-1.5v-9.5a1.5 1.5 0 0 1 1.5-1.5Z" />
    <path d="M9 1.9v2.85h2.85M5.5 8h5M5.5 10.5h3.5" />
  </Icon>
);

/// Four bars of different heights: sound.
export const WaveformIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M3 6.5v3M6 4v8M9 2.5v11M12 5.5v5" strokeWidth={1.6} />
  </Icon>
);

export const ChevronIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="m6 3.5 4.5 4.5L6 12.5" strokeWidth={1.5} />
  </Icon>
);

export const MoreIcon = ({ className }: IconProps) => (
  <svg aria-hidden viewBox="0 0 16 16" className={`shrink-0 ${className ?? "size-4"}`}>
    <circle cx="3.5" cy="8" r="1.25" fill="currentColor" />
    <circle cx="8" cy="8" r="1.25" fill="currentColor" />
    <circle cx="12.5" cy="8" r="1.25" fill="currentColor" />
  </svg>
);

export const CautionIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M7.13 2.5a1 1 0 0 1 1.74 0l5.4 9.5a1 1 0 0 1-.87 1.5H2.6a1 1 0 0 1-.87-1.5Z" />
    <path d="M8 6.25v3M8 11.25v.01" strokeWidth={1.5} />
  </Icon>
);

/// Sliders: general settings.
export const SlidersIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M2.5 4.5h6M11.5 4.5h2M2.5 11.5h2M7.5 11.5h6" />
    <circle cx="10" cy="4.5" r="1.5" />
    <circle cx="6" cy="11.5" r="1.5" />
  </Icon>
);

/// A microphone on a stand: the Recording pane.
export const MicrophoneIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <rect x="5.75" y="1.75" width="4.5" height="8" rx="2.25" />
    <path d="M3.5 7.75a4.5 4.5 0 0 0 9 0M8 12.25v2M5.75 14.25h4.5" />
  </Icon>
);

export const FolderIcon =({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M1.75 4.25a1.5 1.5 0 0 1 1.5-1.5h2.8l1.5 1.5h5.2a1.5 1.5 0 0 1 1.5 1.5v5.75a1.5 1.5 0 0 1-1.5 1.5h-9.5a1.5 1.5 0 0 1-1.5-1.5Z" />
    <path d="M1.75 6.5h12.5" />
  </Icon>
);

/// A sparkle: summaries that a model writes.
export const SparkleIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M7 1.75c.4 2.9 1.6 4.1 4.5 4.5-2.9.4-4.1 1.6-4.5 4.5-.4-2.9-1.6-4.1-4.5-4.5 2.9-.4 4.1-1.6 4.5-4.5Z" />
    <path d="M12.25 9.75c.2 1.35.65 1.8 2 2-1.35.2-1.8.65-2 2-.2-1.35-.65-1.8-2-2 1.35-.2 1.8-.65 2-2Z" />
  </Icon>
);

/// A speech bubble with lines: transcription.
export const TranscriptIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M3.25 2.25h9.5a1.5 1.5 0 0 1 1.5 1.5v6a1.5 1.5 0 0 1-1.5 1.5H8l-3 2.5v-2.5H3.25a1.5 1.5 0 0 1-1.5-1.5v-6a1.5 1.5 0 0 1 1.5-1.5Z" />
    <path d="M4.75 5.5h6.5M4.75 8h4" />
  </Icon>
);

/// A check mark: the selected item in a list.
export const CheckIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="m3.25 8.5 3 3 6.5-7" strokeWidth={1.8} />
  </Icon>
);

/// A circle with an "i": the About pane.
export const InfoIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <circle cx="8" cy="8" r="6" />
    <path d="M8 7.25v3.75" strokeWidth={1.5} />
    <path d="M8 5v.01" strokeWidth={1.8} />
  </Icon>
);

/// An arrow up and to the right: the link opens outside the app.
export const ExternalIcon = ({ className }: IconProps) => (
  <Icon className={className}>
    <path d="M5.5 10.5 10.5 5.5M6.5 5.5h4v4" strokeWidth={1.5} />
  </Icon>
);
