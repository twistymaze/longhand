import { describe, expect, test } from "bun:test";
import {
  checkOnnxRuntimeVersion,
  copyrightFromFiles,
  PART_TITLES,
  renderList,
  selectNative,
  splitCopyright,
  type LicensedComponent,
  type NativeEntry,
} from "./group";

const MIT_BODY = `Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.`;

function mit(copyright: string): string {
  return `MIT License\n\n${copyright}\n\n${MIT_BODY}\n`;
}

function crate(name: string, text: string, version = "1.0.0"): LicensedComponent {
  return { part: "rust", name, version, text };
}

function count(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

describe("splitCopyright", () => {
  test("separates the leading copyright lines from the body", () => {
    const { copyright, body } = splitCopyright(mit("Copyright (c) 2020 Alice"));
    expect(copyright).toEqual(["Copyright (c) 2020 Alice"]);
    expect(body).toBe(`MIT License\n\n${MIT_BODY}`);
  });

  test("joins a copyright line with its indented continuation lines", () => {
    const text = "Copyright 2001-2023 Xiph.Org, Skype Limited,\n        Jean-Marc Valin\n\nRedistribution is permitted.\n";
    expect(splitCopyright(text).copyright).toEqual(["Copyright 2001-2023 Xiph.Org, Skype Limited, Jean-Marc Valin"]);
  });

  test("drops template placeholders instead of keeping them as copyright", () => {
    const { copyright } = splitCopyright(mit("Copyright (c) <year> <copyright holders>"));
    expect(copyright).toEqual([]);
  });
});

describe("renderList", () => {
  test("AE3: one body, one component line for each package, each with its own copyright", () => {
    const out = renderList("Test", [
      crate("alpha", mit("Copyright (c) 2020 Alice")),
      crate("beta", mit("Copyright (c) 2021 Bob")),
      crate("gamma", mit("Copyright (c) 2022 Carol")),
    ]);
    expect(count(out, "Permission is hereby granted")).toBe(1);
    expect(out).toContain("alpha 1.0.0\n    Copyright (c) 2020 Alice");
    expect(out).toContain("beta 1.0.0\n    Copyright (c) 2021 Bob");
    expect(out).toContain("gamma 1.0.0\n    Copyright (c) 2022 Carol");
  });

  test("AE3 at scale: 40 crates under the same MIT text show it once", () => {
    const crates = Array.from({ length: 40 }, (_, i) => crate(`crate${i}`, mit(`Copyright (c) ${i} Author ${i}`)));
    const out = renderList("Test", crates);
    expect(count(out, "Permission is hereby granted")).toBe(1);
    for (let i = 0; i < 40; i++) expect(out).toContain(`crate${i} 1.0.0\n    Copyright (c) ${i} Author ${i}\n`);
  });

  test("texts that differ only in CRLF and trailing spaces group together", () => {
    const lf = mit("Copyright (c) 2020 Alice");
    const crlf = mit("Copyright (c) 2021 Bob").replace(/\n/g, "  \r\n");
    const out = renderList("Test", [crate("alpha", lf), crate("beta", crlf)]);
    expect(count(out, "Permission is hereby granted")).toBe(1);
    expect(out).not.toContain("\r");
    expect(out).not.toMatch(/ +\n/);
  });

  test("texts that differ only in line wrapping or in the title group together", () => {
    const rewrapped = `The MIT License (MIT)\n\nCopyright (c) 2021 Bob\n\n${MIT_BODY.replace(/\n/g, " ").replace(/\. /g, ".\n\n")}\n`;
    const out = renderList("Test", [crate("alpha", mit("Copyright (c) 2020 Alice")), crate("beta", rewrapped)]);
    expect(count(out, "Permission is hereby granted")).toBe(1);
    expect(out).toContain("beta 1.0.0\n    Copyright (c) 2021 Bob");
  });

  test("the same entries in a different order give byte-identical output", () => {
    const entries = [
      crate("alpha", mit("Copyright (c) 2020 Alice")),
      crate("beta", "Apache License\nVersion 2.0\n\nTerms.\n", "2.0.0"),
      { part: "interface", name: "react", version: "19.0.0", text: mit("Copyright (c) Meta") } as LicensedComponent,
      { part: "native", name: "ONNX Runtime", version: "1.22.0", text: mit("Copyright (c) Microsoft") } as LicensedComponent,
      crate("alpha", "Apache License\nVersion 2.0\n\nTerms.\n"),
    ];
    const forward = renderList("Test", entries);
    const backward = renderList("Test", [...entries].reverse());
    expect(backward).toBe(forward);
  });

  test("a text with no copyright line keeps a component line with the name and version only", () => {
    const out = renderList("Test", [crate("plain", `${MIT_BODY}\n`, "0.3.1")]);
    expect(out).toMatch(/^plain 0\.3\.1$/m);
    expect(out).not.toMatch(/^plain 0\.3\.1\n {4}/m);
  });

  test("each part starts with its title on a line of its own, in a fixed order", () => {
    const out = renderList("Test", [
      { part: "native", name: "n", version: "1", text: "N text" },
      { part: "interface", name: "i", version: "1", text: "I text" },
      crate("r", "R text"),
    ]);
    const lines = out.split("\n");
    const positions = PART_TITLES.map((title) => lines.indexOf(title));
    expect(PART_TITLES).toEqual(["Rust crates", "Interface packages", "Native libraries"]);
    expect(positions.every((p) => p >= 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
  });

  test("the output has LF line ends and ends with one newline", () => {
    const out = renderList("Test", [crate("a", "Text\r\n\r\n")]);
    expect(out.endsWith("\n")).toBe(true);
    expect(out.endsWith("\n\n")).toBe(false);
  });
});

describe("selectNative", () => {
  const entries: NativeEntry[] = [
    { name: "ONNX Runtime", version: "1.22.0", part: "native", platforms: ["macos", "windows"], files: ["a.txt"], source: "https://example.com/a" },
    { name: "Vulkan-Headers", version: "1.4.309", part: "native", platforms: ["windows"], files: ["b.txt"], source: "https://example.com/b" },
  ];
  const read = (path: string) => `text of ${path}`;

  test("a Windows-only entry is in the Windows list and not in the macOS list", () => {
    const mac = renderList("macOS", selectNative(entries, "macos", read));
    const win = renderList("Windows", selectNative(entries, "windows", read));
    expect(win).toContain("Vulkan-Headers 1.4.309");
    expect(mac).not.toContain("Vulkan-Headers");
    expect(mac).toContain("ONNX Runtime 1.22.0");
  });
});

describe("checkOnnxRuntimeVersion", () => {
  const dist = "cu12\tx86_64-pc-windows-msvc\thttps://cdn.pyke.io/0/pyke:ort-rs/ms@1.22.0/x.tgz\tABC\n" +
    "none\taarch64-apple-darwin\thttps://cdn.pyke.io/0/pyke:ort-rs/ms@1.22.0/y.tgz\tDEF\n";

  test("passes when dist.txt names the manifest's version", () => {
    expect(() => checkOnnxRuntimeVersion(dist, "1.22.0")).not.toThrow();
  });

  test("fails with both versions named when they differ", () => {
    expect(() => checkOnnxRuntimeVersion(dist, "1.21.0")).toThrow(/1\.21\.0.*1\.22\.0|1\.22\.0.*1\.21\.0/);
  });

  test("fails when dist.txt names no version", () => {
    expect(() => checkOnnxRuntimeVersion("nothing here\n", "1.22.0")).toThrow();
  });
});

describe("copyrightFromFiles", () => {
  const standard = `MIT License\n\n${MIT_BODY}\n`;

  test("takes the copyright lines of a crate file with the same licence body", () => {
    const files = [mit("Copyright (c) 2017 - Present Tauri Apps Contributors")];
    expect(copyrightFromFiles(standard, files)).toEqual(["Copyright (c) 2017 - Present Tauri Apps Contributors"]);
  });

  test("ignores a file with a different licence body", () => {
    const files = ["Copyright (c) 2016 Alex Crichton\n\nApache License\nVersion 2.0, January 2004\n\nTerms and conditions."];
    expect(copyrightFromFiles(standard, files)).toEqual([]);
  });

  test("keeps the text's own copyright lines when it has them", () => {
    const files = [mit("Copyright (c) 2017 Someone Else")];
    expect(copyrightFromFiles(mit("Copyright (c) 2020 Someone"), files)).toEqual(["Copyright (c) 2020 Someone"]);
  });
});
