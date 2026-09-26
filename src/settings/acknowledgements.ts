// The static text of the About pane (KTD9): the acknowledgements and one
// licence row for each major component. Each licence value comes from the
// component's own licence file at the version that Longhand ships. The
// anarlog and Handy texts are the author's words (R3, R4); change them only
// with the author's approval.

/// A project that the About pane thanks. `url` is the https address that
/// the opener opens. `src-tauri/capabilities/settings.json` allows exactly
/// these addresses, so a new entry needs a new address there too.
export type Acknowledgement = {
  name: string;
  url: string;
  /// The paragraphs under the name.
  text: string[];
};

/// The personal entries, anarlog first (R3, R4). The last paragraph of each
/// says which parts of Longhand come from the project, after its
/// `PROVENANCE.md` records.
export const personal: Acknowledgement[] = [
  {
    name: "anarlog",
    url: "https://anarlog.so/",
    text: [
      "anarlog inspired me to make Longhand. The work its team is doing is outstanding. I wanted something smaller and simpler for my own use, and I started from what they built.",
      "Recording, audio processing, telling speakers apart, and the note editor started from anarlog's code.",
    ],
  },
  {
    name: "Handy",
    url: "https://handy.computer/",
    text: [
      "Thank you for an amazing local-only dictation tool that I still use every day.",
      "The model list and downloads, speech recognition, voice detection and parts of note processing started from Handy's code.",
    ],
  },
];

/// The technical thanks, in the order of R5.
export const technical: Acknowledgement[] = [
  {
    name: "whisper.cpp",
    url: "https://github.com/ggml-org/whisper.cpp",
    text: ["By Georgi Gerganov and contributors. It showed that speech recognition can run fast on a laptop."],
  },
  {
    name: "ggml",
    url: "https://github.com/ggml-org/ggml",
    text: ["By Georgi Gerganov and contributors. It runs the speech recognition models on this computer."],
  },
  {
    name: "transcribe.cpp",
    url: "https://github.com/handy-computer/transcribe.cpp",
    text: ["It turns speech into text, and reads whisper.cpp's model format."],
  },
  {
    name: "ONNX Runtime",
    url: "https://github.com/microsoft/onnxruntime",
    text: ["It runs the voice detection and speaker models."],
  },
  {
    name: "pyannote",
    url: "https://github.com/pyannote/pyannote-audio",
    text: ["Its speaker models and methods find who spoke when."],
  },
  {
    name: "Tauri",
    url: "https://tauri.app/",
    text: ["The framework that makes Longhand a desktop app."],
  },
];

/// One row of the Licences group. A model row has a role in `kind`; a
/// component row has none. `revision` is a commit or a version.
export type LicenceEntry = {
  id: string;
  name: string;
  kind?: string;
  licence: { name: string; url: string; attribution: string };
  source: string;
  revision?: string;
};

/// One row for each major component (R7). whisper.cpp has no row: Longhand
/// contains none of its code, and transcribe.cpp only reads its model
/// format.
export const components: LicenceEntry[] = [
  {
    id: "ggml",
    name: "ggml",
    licence: {
      name: "MIT",
      url: "https://docs.rs/crate/transcribe-cpp-sys/0.2.3/source/ggml/LICENSE",
      attribution: "Copyright (c) 2023-2026 The ggml authors",
    },
    source: "github.com/ggml-org/ggml, in transcribe-cpp-sys",
    revision: "0.2.3",
  },
  {
    id: "transcribe-cpp",
    name: "transcribe.cpp",
    licence: {
      name: "MIT",
      url: "https://docs.rs/crate/transcribe-cpp-sys/0.2.3/source/LICENSE",
      attribution: "Copyright (c) 2026 The transcribe.cpp authors",
    },
    source: "github.com/handy-computer/transcribe.cpp",
    revision: "0.2.3",
  },
  {
    id: "onnxruntime",
    name: "ONNX Runtime",
    licence: {
      name: "MIT",
      url: "https://github.com/microsoft/onnxruntime/blob/v1.22.0/LICENSE",
      attribution: "Copyright (c) Microsoft Corporation",
    },
    source: "github.com/microsoft/onnxruntime",
    revision: "v1.22.0",
  },
  {
    id: "tauri",
    name: "Tauri",
    licence: {
      name: "MIT OR Apache-2.0",
      url: "https://docs.rs/crate/tauri/2.11.6/source/LICENSE_MIT",
      attribution: "Copyright (c) 2017 - Present Tauri Apps Contributors",
    },
    source: "github.com/tauri-apps/tauri",
    revision: "2.11.6",
  },
  {
    id: "prosemirror",
    name: "ProseMirror",
    licence: {
      name: "MIT",
      url: "https://unpkg.com/prosemirror-model@1.25.12/LICENSE",
      attribution: "Copyright (C) 2015-2017 by Marijn Haverbeke <marijn@haverbeke.berlin> and others",
    },
    source: "github.com/ProseMirror",
  },
  {
    id: "react",
    name: "React",
    licence: {
      name: "MIT",
      url: "https://unpkg.com/react@19.2.8/LICENSE",
      attribution: "Copyright (c) Meta Platforms, Inc. and affiliates.",
    },
    source: "github.com/facebook/react",
    revision: "19.2.8",
  },
  {
    id: "cpal",
    name: "cpal",
    licence: {
      name: "Apache-2.0",
      url: "https://docs.rs/crate/cpal/0.18.2/source/LICENSE",
      attribution: "The cpal contributors (RustAudio)",
    },
    source: "github.com/RustAudio/cpal",
    revision: "0.18.2",
  },
  {
    id: "opus",
    name: "Opus",
    licence: {
      name: "BSD-3-Clause",
      url: "https://docs.rs/crate/opusic-sys/0.7.5/source/opus/COPYING",
      attribution:
        "Copyright 2001-2023 Xiph.Org, Skype Limited, Octasic, Jean-Marc Valin, Timothy B. Terriberry, CSIRO, Gregory Maxwell, Mark Borgerding, Erik de Castro Lopo, Mozilla, Amazon",
    },
    source: "libopus, in opusic-sys 0.7.5",
    revision: "1.6.1",
  },
  {
    id: "dtln-aec",
    name: "DTLN-aec echo cancellation model",
    licence: {
      name: "MIT",
      url: "https://github.com/breizhn/DTLN-aec/blob/main/LICENSE",
      attribution: "Copyright (c) 2020 Nils L. Westhausen",
    },
    source: "github.com/breizhn/DTLN-aec, through anarlog",
    revision: "128-unit model",
  },
  {
    id: "anarlog",
    name: "Code from anarlog",
    licence: {
      name: "MIT",
      url: "https://github.com/fastrepl/anarlog/blob/39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e/LICENSE",
      attribution: "Copyright (c) 2023-present Fastrepl, Inc.",
    },
    source: "github.com/fastrepl/anarlog",
    revision: "39613e537ba4, 7040ace23d16, 66d59aa6c241",
  },
  {
    id: "handy",
    name: "Code from Handy",
    licence: {
      name: "MIT",
      url: "https://github.com/cjpais/Handy/blob/8f9cf53cd1410cda26beea39ff802ac306e39585/LICENSE",
      attribution: "Copyright (c) 2025 CJ Pais",
    },
    source: "github.com/cjpais/Handy",
    revision: "8f9cf53cd141",
  },
];
