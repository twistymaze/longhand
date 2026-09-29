import { describe, expect, test } from "bun:test";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ModelStatus } from "../bindings";
import { mockTauri } from "../test/tauri";
import { About, leadingAddress } from "./About";
import { components, personal, technical } from "./acknowledgements";

const model = (id: string, name: string, role: ModelStatus["role"]): ModelStatus => ({
  id,
  speaker_preferences: null,
  role,
  name,
  description: "",
  assessment: null,
  streaming: false,
  languages: [],
  recommended: false,
  size_bytes: 1,
  bundle: [id],
  bundle_size_bytes: 1,
  bundle_complete: true,
  required: true,
  state: { state: "present" },
  repository: `example/${id}`,
  revision: "0123456789abcdef",
  licence: { name: "MIT", url: `https://example.com/${id}/LICENSE`, attribution: `${name} authors` },
});

const models = [model("a", "Speech A", "recognition"), model("b", "Voices B", "embedding")];

// The mock opener and window commands, with a record of each call.
function mock() {
  return mockTauri({ "plugin:opener|open_url": () => null, open_licences: () => null });
}

const section = (name: string) => screen.getByRole("heading", { name }).closest("section") as HTMLElement;

describe("the version row", () => {
  test("shows the version and the short commit of a CI build", async () => {
    mockTauri({ build_info: () => ({ version: "0.1.0-preview.20260928143012", commit: "a1b2c3d", track: "preview" }) });
    render(<About models={models} />);
    expect(await screen.findByText("Version 0.1.0-preview.20260928143012 (a1b2c3d)")).toBeTruthy();
  });

  test("says local build when the build has no commit and no track", async () => {
    mockTauri({ build_info: () => ({ version: "0.1.0", commit: null, track: null }) });
    render(<About models={models} />);
    expect(await screen.findByText("Version 0.1.0 (local build)")).toBeTruthy();
  });

  test("comes before the acknowledgements", async () => {
    mockTauri({ build_info: () => ({ version: "0.1.0", commit: null, track: null }) });
    render(<About models={models} />);
    const row = await screen.findByText("Version 0.1.0 (local build)");
    expect(row.compareDocumentPosition(section("Acknowledgements")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("About", () => {
  test("the pane is About, with Acknowledgements, Licences and then the full list row", () => {
    render(<About models={models} />);
    expect(screen.getByRole("heading", { level: 1, name: "About" })).toBeTruthy();
    const order = [
      section("Acknowledgements"),
      section("Licences"),
      screen.getByRole("button", { name: /All open source licences/ }),
    ];
    for (let i = 1; i < order.length; i++) {
      expect(order[i - 1].compareDocumentPosition(order[i]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
  });

  test("anarlog comes first and Handy second, each with the author's words", () => {
    render(<About models={models} />);
    const group = within(section("Acknowledgements"));
    const names = group.getAllByRole("button").map((b) => b.textContent);
    expect(names[0]).toMatch(/^anarlog/);
    expect(names[1]).toMatch(/^Handy/);
    expect(names).toHaveLength(2);
    expect(
      group.getByText(
        /anarlog inspired me to make Longhand\. The work its team is doing is outstanding\. I wanted something smaller and simpler for my own use, and I started from what they built\./,
      ),
    ).toBeTruthy();
    expect(
      group.getByText(/Thank you for an amazing local-only dictation tool that I still use every day\./),
    ).toBeTruthy();
  });

  test("the technical thanks have their own group, apart from anarlog and Handy", () => {
    render(<About models={models} />);
    const thanks = within(section("Built with"));
    const names = thanks.getAllByRole("button").map((b) => b.textContent?.replace(/\(opens.*\)$/, "").trim());
    expect(names).toEqual(["whisper.cpp", "ggml", "transcribe.cpp", "ONNX Runtime", "pyannote", "Tauri"]);
    expect(thanks.queryByRole("button", { name: /anarlog|Handy/ })).toBeNull();
  });

  test("each acknowledgement says that it opens in the browser, and a click opens its page", async () => {
    const tauri = mock();
    render(<About models={models} />);
    const user = userEvent.setup();
    const anarlog = screen.getByRole("button", { name: /^anarlog.*opens in your browser/ });
    await user.click(anarlog);
    await user.click(screen.getByRole("button", { name: /^ggml.*opens in your browser/ }));
    const opened = tauri.calls.filter((c) => c.cmd === "plugin:opener|open_url").map((c) => c.args.url);
    expect(opened).toEqual(["https://anarlog.so/", "https://github.com/ggml-org/ggml"]);
  });

  test("the Licences group has one row for each model and each component, and no whisper.cpp row", () => {
    render(<About models={models} />);
    const rows = within(section("Licences")).getAllByRole("button");
    expect(rows).toHaveLength(models.length + components.length);
    expect(rows.map((r) => r.getAttribute("aria-expanded"))).toEqual(Array(rows.length).fill("false"));
    expect(within(section("Licences")).queryByRole("button", { name: /whisper/ })).toBeNull();
  });

  test("a model row shows its role and licence; a component row shows its licence only", () => {
    render(<About models={models} />);
    const licences = within(section("Licences"));
    expect(licences.getByRole("button", { name: /Speech A/ }).textContent).toContain("Speech recognition · MIT");
    const ggml = licences.getByRole("button", { name: /^ggml/ });
    expect(ggml.textContent).toBe("ggmlMIT");
  });

  test("each model row opens and shows its details", async () => {
    render(<About models={models} />);
    expect(screen.queryByText("Speech A authors")).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: /Speech A/ }));
    const row = screen.getByRole("button", { name: /Speech A/ });
    expect(row.getAttribute("aria-expanded")).toBe("true");
    // Open, the row line is the name only; the role and licence are in the details.
    expect(row.textContent).not.toContain("Speech recognition");
    const details = within(document.getElementById(row.getAttribute("aria-controls") ?? "")!);
    expect(details.getByText("Used for")).toBeTruthy();
    expect(details.getByText("Speech recognition")).toBeTruthy();
    expect(screen.getByText("Speech A authors")).toBeTruthy();
    expect(screen.getByText("https://example.com/a/LICENSE")).toBeTruthy();
    expect(screen.getByText("0123456789ab")).toBeTruthy();
    expect(screen.queryByText("Voices B authors")).toBeNull();
  });

  test("a click on a component row shows its licence, attribution, address and source, and a second click hides them", async () => {
    render(<About models={models} />);
    const user = userEvent.setup();
    const row = within(section("Licences")).getByRole("button", { name: /^ggml/ });
    await user.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    const details = document.getElementById(row.getAttribute("aria-controls") ?? "") as HTMLElement;
    const d = within(details);
    expect(d.getByText("MIT")).toBeTruthy();
    expect(d.getByText("Copyright (c) 2023-2026 The ggml authors")).toBeTruthy();
    expect(d.getByText("https://docs.rs/crate/transcribe-cpp-sys/0.2.4/source/ggml/LICENSE")).toBeTruthy();
    expect(d.getByText(/github\.com\/ggml-org\/ggml/)).toBeTruthy();
    await user.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(document.getElementById(row.getAttribute("aria-controls") ?? "")).toBeNull();
  });

  test("with no models, the component rows still show", () => {
    render(<About models={[]} />);
    expect(within(section("Licences")).getAllByRole("button")).toHaveLength(components.length);
  });

  test("the Settings opener allows only https, and every address the pane can open is https", async () => {
    const capability = await Bun.file(`${import.meta.dir}/../../src-tauri/capabilities/settings.json`).json();
    const opener = capability.permissions.find(
      (p: unknown) => typeof p === "object" && (p as { identifier: string }).identifier === "opener:allow-open-url",
    );
    const allowed = opener.allow.map((a: { url: string }) => a.url).sort();

    // The acknowledgement links, and the addresses in the licence details of
    // each component and of each model in the bundled catalog.
    const catalog = await Bun.file(`${import.meta.dir}/../../crates/models/catalog.json`).json();
    const details: string[] = components.flatMap((c) => [c.licence.url, c.source]);
    const walk = (v: unknown): void => {
      if (Array.isArray(v)) return v.forEach(walk);
      if (!v || typeof v !== "object") return;
      const o = v as { repository?: unknown; licence?: { url?: unknown } };
      if (typeof o.repository === "string") details.push(o.repository);
      if (typeof o.licence?.url === "string") details.push(o.licence.url);
      Object.values(v).forEach(walk);
    };
    walk(catalog);
    const expected = new Set([...personal, ...technical].map((a) => a.url));
    for (const text of details) {
      const address = leadingAddress(text);
      if (address) expected.add(address.url);
    }
    // The opener allows web pages over https only: no other scheme can start a
    // program from this window.
    expect(allowed).toEqual(["https://*"]);
    expect([...expected].filter((url) => !url.startsWith("https://"))).toEqual([]);
  });

  test("an address at the start of a detail opens in the browser", async () => {
    const tauri = mock();
    render(<About models={models} />);
    const user = userEvent.setup();
    await user.click(within(section("Licences")).getByRole("button", { name: /^ggml/ }));
    await user.click(screen.getByRole("button", { name: /^github\.com\/ggml-org\/ggml.*opens in your browser/ }));
    await user.click(screen.getByRole("button", { name: /ggml\/LICENSE.*opens in your browser/ }));
    await waitFor(() =>
      expect(tauri.calls.filter((c) => c.cmd === "plugin:opener|open_url").map((c) => c.args.url)).toEqual([
        "https://github.com/ggml-org/ggml",
        "https://docs.rs/crate/transcribe-cpp-sys/0.2.4/source/ggml/LICENSE",
      ]),
    );
    expect(leadingAddress("libopus, in opusic-sys 0.7.5")).toBeNull();
  });

  test("the full list row opens the licences window once", async () => {
    const tauri = mock();
    render(<About models={models} />);
    await userEvent.setup().click(screen.getByRole("button", { name: /All open source licences/ }));
    await waitFor(() => expect(tauri.calls.filter((c) => c.cmd === "open_licences")).toHaveLength(1));
  });
});
