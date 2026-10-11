# Longhand

Longhand is a note-taking app for meetings and conversations. You write the notes that matter to you, and Longhand records the audio. When the conversation ends, it uses the transcript to turn your short notes into a complete summary.

Recording and transcription happen on your computer. Your notes are files that you own, and you decide which AI service, if any, sees them.

Longhand is a young project. You are welcome to follow along.

## The idea

In a meeting, you want to take part in the conversation, not type everything that is said. But a few days later you need the details: what was decided, who agreed to what, and the exact numbers someone mentioned.

With Longhand, your notes can stay short. While you talk, you type a few words in a simple Markdown notepad. The transcript holds the details you did not write. Afterwards, Longhand combines your notes and the transcript into a summary, with your notes showing what was important to you.

## Privacy

- **Audio never leaves your computer.** Longhand records and transcribes with local speech models. It marks where the speaker changes, but it never tries to find out who is speaking.
- **You control the AI service.** The summary comes from an OpenAI-compatible endpoint that you set in Settings: a cloud service you trust, or a model server that you run yourself. Only your notes and the transcript go to that endpoint, and only when Longhand makes a summary.
- **No account and no Longhand cloud.** Longhand has no server of its own and collects no usage data. Apart from the summary endpoint, it connects to the internet only to download speech models and to check for updates.
- **Your notes are plain files.** Audio, transcripts, notes and summaries are files in a storage folder that you choose. You can read, copy or delete them with any tool.

## Using more than one computer

Longhand has no built-in sync. If you want your notes on more than one computer, put the storage folder in a folder that a sync service already manages, such as OneDrive, Google Drive or Dropbox. Longhand is built to share one storage folder between computers this way.

## How it works

- **It records from your microphone.** That works in a room, and on a call when your microphone can hear the other people. Direct recording of system audio is coming soon.
- **Long sessions work.** Longhand supports sessions of four hours and more.
- **Organization stays small.** Each note has a note type and any number of tags.

Longhand runs on macOS first. Windows is supported, with less testing.

## Get Longhand

Official releases are coming soon. Until then, you can build Longhand from source.

### Build on macOS

You need:

- macOS 15 or later
- Rust 1.94.0 (the repository's `rust-toolchain.toml` selects it)
- [Bun](https://bun.sh)
- CMake, Ninja and Git

The first build also downloads and compiles the speech recognition libraries, so it takes some time.

```bash
bun install
APPLE_SIGNING_IDENTITY="-" bun tauri build --bundles app
```

`APPLE_SIGNING_IDENTITY="-"` signs the app for your own computer only. Without it, the build tries to use the project's Developer ID certificate, which you do not have. The app is in `target/release/bundle/macos/`.

### Build on Windows

A Windows build also needs the Microsoft C++ build tools, Git Bash and the Vulkan SDK.

## How Longhand is built

Longhand is built with a lot of AI help. Claude Code and Codex plan, write and review most of the code. I direct the work, test it and review it. Some people want to know this about a project, so I say it here.

The workflow comes from Every's [Compound Engineering](https://github.com/EveryInc/compound-engineering-plugin) plugin. Its brainstorm, plan, work and review steps are why the plans in [`docs/plans/`](docs/plans/) and the lessons in [`docs/solutions/`](docs/solutions/) exist. Thank you to the people at Every for sharing it.

## Thanks

- **[Handy](https://handy.computer/)**: thank you for an amazing local-only dictation tool that I still use every day. The model list and downloads, speech recognition, voice detection and parts of note processing started from Handy's code.
- **[anarlog](https://anarlog.so/)** inspired me to make Longhand. The work its team is doing is outstanding. I wanted something smaller and simpler for my own use, and I started from what they built. Recording, audio processing, telling speakers apart and the note editor started from anarlog's code.

Longhand also depends on whisper.cpp, ggml, transcribe.cpp, ONNX Runtime, pyannote, Tauri and many other open-source projects. The app's About pane lists them all, with their licences.

## Contributing

Longhand is not taking outside contributions yet. [CONTRIBUTING.md](CONTRIBUTING.md) explains why.

## Licence

Longhand is released under the [MIT licence](LICENSE). The licences of the code and libraries that Longhand uses are in [`licences/`](licences/) and in the app's About pane. Each file that started from another project's code is recorded in a `PROVENANCE.md` file.
