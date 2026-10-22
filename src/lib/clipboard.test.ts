import { afterEach, describe, expect, test, vi } from "vitest";
import { clearMocks } from "@tauri-apps/api/mocks";
import { mockTauri } from "../test/tauri";
import { cleanHtml, copySelection, copyText, rangeHtml } from "./clipboard";

const userAgent = navigator.userAgent;
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0";

afterEach(() => {
  setPlatform(userAgent);
  clearMocks();
  document.body.innerHTML = "";
});

// A ClipboardItem as WebKit makes it: the types in their order, and a blob for each.
class FakeClipboardItem {
  readonly types: string[];
  constructor(readonly items: Record<string, Blob>) {
    this.types = Object.keys(items);
  }
}
const refuse = async () => {
  throw new Error("denied");
};
const setClipboard = (clipboard: object, item: unknown = FakeClipboardItem) => {
  Object.defineProperty(navigator, "clipboard", { value: clipboard, configurable: true });
  Object.defineProperty(globalThis, "ClipboardItem", { value: item, configurable: true, writable: true });
};
const setPlatform = (userAgent: string) =>
  Object.defineProperty(navigator, "userAgent", { value: userAgent, configurable: true });

describe("copyText", () => {
  test("on macOS, writes through the native command, which keeps the HTML as it is", async () => {
    setPlatform(MAC);
    const tauri = mockTauri({ write_clipboard: () => true });
    const write = vi.fn(async (_items: FakeClipboardItem[]) => {});
    setClipboard({ write, writeText: write });
    expect(await copyText("# A", "<h1>A</h1>")).toBe(true);
    expect(tauri.calls).toEqual([{ cmd: "write_clipboard", args: { text: "# A", html: "<h1>A</h1>" } }]);
    expect(write).not.toHaveBeenCalled();
  });

  test("on macOS, writes through the webview when the native write fails", async () => {
    setPlatform(MAC);
    mockTauri({ write_clipboard: () => false });
    const write = vi.fn(async (_items: FakeClipboardItem[]) => {});
    setClipboard({ write });
    expect(await copyText("# A", "<h1>A</h1>")).toBe(true);
    expect(write).toHaveBeenCalledTimes(1);
  });

  test("on Windows, writes through the webview without a call to the native command", async () => {
    setPlatform(WINDOWS);
    const tauri = mockTauri({ write_clipboard: () => true });
    const write = vi.fn(async (_items: FakeClipboardItem[]) => {});
    setClipboard({ write });
    expect(await copyText("# A", "<h1>A</h1>")).toBe(true);
    expect(tauri.calls).toEqual([]);
    expect(write).toHaveBeenCalledTimes(1);
  });

  test("writes one item with the HTML first and the markdown as plain text", async () => {
    const write = vi.fn(async (_items: FakeClipboardItem[]) => {});
    const writeText = vi.fn(async (_t: string) => {});
    setClipboard({ write, writeText });
    expect(await copyText("# A", "<h1>A</h1>\n")).toBe(true);
    const [item] = write.mock.calls[0][0];
    expect(item.types).toEqual(["text/html", "text/plain"]);
    expect(await item.items["text/html"].text()).toBe("<h1>A</h1>\n");
    expect(await item.items["text/plain"].text()).toBe("# A");
    expect(writeText).not.toHaveBeenCalled();
  });

  test("writes the markdown only when the clipboard refuses the item", async () => {
    const writeText = vi.fn(async (_t: string) => {});
    setClipboard({ write: refuse, writeText });
    expect(await copyText("# A", "<h1>A</h1>")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("# A");
  });

  test("writes the markdown only when there is no ClipboardItem", async () => {
    const writeText = vi.fn(async (_t: string) => {});
    setClipboard({ writeText }, undefined);
    expect(await copyText("# A", "<h1>A</h1>")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("# A");
  });

  test("reports a clipboard that refuses both", async () => {
    setClipboard({ write: refuse, writeText: refuse });
    expect(await copyText("# A", "<h1>A</h1>")).toBe(false);
  });
});

describe("cleanHtml", () => {
  test("keeps the structure and removes each style, class and other attribute", () => {
    const box = document.createElement("div");
    box.innerHTML =
      '<h3 class="text-lg" style="color: white">Plan</h3>' +
      '<ul class="grid" style="color: white"><li><span>a </span><strong class="x">b</strong></li></ul>' +
      '<div class="row"><span data-x="1">00:01</span> Hello.</div>' +
      '<a href="https://example.com" style="color: blue" target="_blank">link</a><a href=" javascript:x()">bad</a>' +
      '<a href="minutes.md#plan" class="x">note</a>';
    expect(cleanHtml(box)).toBe(
      "<h3>Plan</h3><ul><li>a <strong>b</strong></li></ul><p>00:01 Hello.</p>" +
        '<a href="https://example.com">link</a><a>bad</a><a href="minutes.md#plan">note</a>',
    );
  });

  test("keeps a rule, the first number of a list, and the tasks of the editor", () => {
    const box = document.createElement("div");
    box.innerHTML =
      '<hr class="x"><ol start="3" style="counter-reset: ol-counter 2"><li>c</li></ol>' +
      '<ul data-type="taskList" class="task-list"><li data-type="taskItem" data-checked="true" class="x"><p>done</p></li></ul>';
    expect(cleanHtml(box)).toBe(
      '<hr><ol start="3"><li>c</li></ol>' +
        '<ul data-type="taskList"><li data-type="taskItem" data-checked="true"><p>done</p></li></ul>',
    );
  });
});

describe("rangeHtml", () => {
  const page = (html: string) => {
    document.body.innerHTML = html;
    return document.body;
  };

  test("a range over two list items keeps their list", () => {
    const body = page('<ul class="grid"><li>one</li><li><strong>two</strong></li></ul>');
    const range = document.createRange();
    range.setStart(body.querySelector("li")!.firstChild!, 1);
    range.setEnd(body.querySelector("strong")!.firstChild!, 3);
    expect(rangeHtml(range)).toBe("<ul><li>ne</li><li><strong>two</strong></li></ul>");
  });

  test("a range in one bold run keeps the run", () => {
    const body = page("<p>a <strong>bold</strong> b</p>");
    const range = document.createRange();
    range.selectNodeContents(body.querySelector("strong")!.firstChild!);
    expect(rangeHtml(range)).toBe("<strong>bold</strong>");
  });

  test("a range in one heading keeps the heading", () => {
    const body = page('<h3 class="text-lg">Next steps</h3>');
    const range = document.createRange();
    range.setStart(body.querySelector("h3")!.firstChild!, 0);
    range.setEnd(body.querySelector("h3")!.firstChild!, 4);
    expect(rangeHtml(range)).toBe("<h3>Next</h3>");
  });
});

describe("copySelection", () => {
  const selectAll = (html: string) => {
    document.body.innerHTML = html;
    const range = document.createRange();
    range.selectNodeContents(document.body.firstElementChild!);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(range);
  };
  const copyEvent = (target = document.body.firstElementChild) => {
    const data = new Map<string, string>();
    const event = new Event("copy", { cancelable: true }) as ClipboardEvent;
    Object.defineProperty(event, "clipboardData", {
      value: { setData: (type: string, value: string) => data.set(type, value) },
    });
    Object.defineProperty(event, "target", { value: target });
    return { event, data };
  };

  test("on macOS, writes the selection with no styles through the native command", async () => {
    setPlatform(MAC);
    let written: (args: Record<string, unknown>) => void = () => {};
    const call = new Promise<Record<string, unknown>>((resolve) => (written = resolve));
    mockTauri({
      write_clipboard: (args) => {
        written(args);
        return true;
      },
    });
    selectAll('<div class="summary"><h3 style="color: white">Plan</h3><ul><li>one</li></ul></div>');
    const { event, data } = copyEvent();
    copySelection(event);
    expect(event.defaultPrevented).toBe(true);
    expect(data.size).toBe(0);
    expect((await call).html).toBe("<h3>Plan</h3><ul><li>one</li></ul>");
  });

  test("does not change a copy from a text field", () => {
    setPlatform(WINDOWS);
    selectAll("<div><p>Hello</p></div>");
    document.body.insertAdjacentHTML("beforeend", "<input value='title'>");
    const { event, data } = copyEvent(document.querySelector("input"));
    copySelection(event);
    expect(event.defaultPrevented).toBe(false);
    expect(data.size).toBe(0);
  });

  test("on Windows, writes the selection with no styles to the copy event", () => {
    setPlatform(WINDOWS);
    selectAll('<div><p style="color: white">Hello <em>there</em></p></div>');
    const { event, data } = copyEvent();
    copySelection(event);
    expect(event.defaultPrevented).toBe(true);
    expect(data.get("text/html")).toBe("<p>Hello <em>there</em></p>");
    expect(data.get("text/plain")).toBe("Hello there");
  });

  test("does not change a copy that an editor already wrote", () => {
    selectAll("<div><p>Hello</p></div>");
    const { event, data } = copyEvent();
    event.preventDefault();
    copySelection(event);
    expect(data.size).toBe(0);
  });
});
