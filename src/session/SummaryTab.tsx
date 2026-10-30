// The Summary tab of a saved session: the summary, or the stub that a
// failed summary left, with Regenerate (R19, AE17). The body is rendered
// from parsed blocks, never as HTML.

import type { ReactNode } from "react";
import type { SummaryView } from "../bindings";
import { openLink } from "../lib/links";
import { type Inline, type ListBlock, parseMarkdown } from "./markdown";

function Runs({ content }: { content: Inline[] }) {
  return (
    <>
      {content.map((run, i) => {
        switch (run.kind) {
          case "strong":
            return <strong key={i}>{run.text}</strong>;
          case "em":
            return <em key={i}>{run.text}</em>;
          case "code":
            return (
              <code key={i} className="rounded bg-fill px-1 text-[0.92em]">
                {run.text}
              </code>
            );
          case "link":
            // The default browser opens the link: the window stays on the app.
            return (
              <a
                key={i}
                href={run.href}
                className="text-accent underline underline-offset-2 hover:opacity-80"
                onClick={(event) => {
                  event.preventDefault();
                  openLink(run.href);
                }}
              >
                {run.text}
              </a>
            );
          default:
            return <span key={i}>{run.text}</span>;
        }
      })}
    </>
  );
}

/// A list and its child lists. A child list has the same styles, indented
/// below its item.
function List({ list, nested = false }: { list: ListBlock; nested?: boolean }) {
  const items = list.items.map((item, j) => (
    <li key={j}>
      <Runs content={item.content} />
      {item.children.map((child, k) => (
        <List key={k} list={child} nested />
      ))}
    </li>
  ));
  const space = nested ? "mt-1 " : "";
  return list.ordered ? (
    <ol className={`${space}grid list-decimal gap-1 pl-5`}>{items}</ol>
  ) : (
    <ul className={`${space}grid list-disc gap-1 pl-5`}>{items}</ul>
  );
}

function Body({ markdown }: { markdown: string }) {
  const blocks = parseMarkdown(markdown);
  return (
    <div className="grid gap-3 leading-relaxed">
      {blocks.map((block, i) => {
        if (block.kind === "heading") {
          const size = block.level === 1 ? "text-[1.2rem] tracking-[-0.01em]" : "text-[1.05rem]";
          return (
            <h3 key={i} className={`${size} mt-1 leading-snug font-semibold`}>
              <Runs content={block.content} />
            </h3>
          );
        }
        if (block.kind === "list") return <List key={i} list={block} />;
        return (
          <p key={i}>
            <Runs content={block.content} />
          </p>
        );
      })}
    </div>
  );
}

export function SummaryTab({
  summary,
  regenerate,
}: {
  summary: SummaryView | null;
  /// The Regenerate control and its progress, from the page.
  regenerate: ReactNode;
}) {
  // A stub says why no summary was produced (U5); Regenerate is the way on.
  if (!summary || summary.stub) {
    return (
      <div className="rounded-[10px] bg-group px-4 py-3 shadow-[0_0_0_0.5px_var(--separator)]">
        {summary ? <Body markdown={summary.body} /> : <p>This note has no summary.</p>}
        <div className="mt-3">{regenerate}</div>
      </div>
    );
  }
  return (
    <div>
      <Body markdown={summary.body} />
      <footer className="mt-8 flex items-center gap-3 border-t border-separator pt-3 text-[0.85rem] text-label-secondary">
        <span className="min-w-0 flex-1">
          {summary.prompt ? `${summary.prompt} · ` : ""}Summarized by {summary.model} on {summary.host}
        </span>
        {regenerate}
      </footer>
    </div>
  );
}
