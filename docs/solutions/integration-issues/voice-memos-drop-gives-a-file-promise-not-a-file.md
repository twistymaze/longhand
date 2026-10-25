---
title: "A drop from Voice Memos did nothing: the drag has a file promise, and wry reads only file URLs"
date: 2026-10-25
category: integration-issues
module: "Drop of files on Home (src-tauri/src/file_promise.rs, src/home/useFileDrop.ts; crates/audio-import/src/lib.rs)"
problem_type: integration_issue
component: tauri_backend
severity: medium
symptoms:
  - "A drag from Voice Memos over the main window showed the + cursor, but the drop did nothing"
  - "After the first fix, the log said NSKVONotifying_...WryWebView has no draggingEntered:"
  - "After the second fix, the app quit at the drag with no crash report; stderr said unexpected NULL returned from -[NSFilePromiseReceiver fileNames]"
  - "After the third fix, the received file was a .qta file, which import refused"
root_cause: unverified_assumption
resolution_type: code_fix
framework_version: "tauri 2 with wry 0.57.1, objc2-app-kit 0.3.2, macOS 27"
related_components:
  - audio_import
  - frontend
tags: [macos, drag-and-drop, file-promise, voice-memos, wry, objc2, qta, nsfilepromisereceiver]
---

# A drop from Voice Memos did nothing: the drag has a file promise, and wry reads only file URLs

## Problem

Home imports the audio files that the user drops on the window (PR #68). A drop from Finder worked. A drop from Voice Memos did nothing. Voice Memos does not put a file on the drag pasteboard. It puts a file promise: the other app writes the file only when the drop target asks for it. wry's drop handler reads only file URLs and `NSFilenamesPboardType` (`collect_paths` in `src/wkwebview/drag_drop.rs` of the wry 0.57.1 crate, in the Cargo registry, not in this repository), so the drop gave Home an empty path list. wry's `draggingEntered:` returns `NSDragOperation::Copy` when its drop handler accepts the event, also with no paths, so the cursor showed + and the user expected a drop.

## Symptoms

- A drag from Voice Memos showed the + cursor. The drop did nothing, and the log had no line.
- The fix then failed in three more ways during the manual test. Each one is a trap for the next native drag code (see What Didn't Work).

## What Didn't Work

- **Look for the drag methods on the class of the webview object.** The first build replaced the methods only when the object's class had them itself, so that a change could not reach WKWebView. The log said `..NSKVONotifying_wry::...::WryWebView0.57.0 has no draggingEntered:`. macOS puts a key-value observing subclass on top of wry's class at run time, and that subclass has no drag methods of its own. The search must go up the superclasses to the class that has the method, and stop at `WKWebView`.
- **Count the promised files with `fileNames`.** `-[NSFilePromiseReceiver fileNames]` returned nil at the drag start in the manual test (it has values only after the receive starts). The objc2-app-kit 0.3.2 binding declares a non-null return, so it panicked with `unexpected NULL returned from -[NSFilePromiseReceiver fileNames]`. The panic unwound through AppKit into tao's event loop, and tao stopped the app. macOS wrote no crash report, because the process exited normally after the panic. The panic text went only to stderr.
- **Only `m4a`, `mp3` and `wav`.** The promised file from Voice Memos was `New Recording 3.qta`. A `qta` file is AAC audio in a QuickTime container (`ftyp` brand `qt  `). The import sheet refused it by its extension, although Symphonia's isomp4 demuxer decodes it.

## Solution

`src-tauri/src/file_promise.rs` (PR #68):

1. After the build of the main window, replace `draggingEntered:` and `performDragOperation:` on wry's `WryWebView` class (`accept_promised_files`, line 172). `method_owner` (line 113) walks from the object's class up to the class that has the method itself, and returns `None` at `WKWebView`. Each replacement first calls wry's own implementation, so a drop of files does not change.
2. For a drag on the main webview that has `NSFilePromiseReceiver` objects and no file URLs (`promises_only`, line 340): at the drag start, send `PromisedFiles::Over`, so Home shows the drop target. At the drop, `receive` (line 374) calls `receivePromisedFilesAtDestination:options:operationQueue:reader:` with a new folder `<app cache>/received-files/<uuid>/`. After the last reader call, it sends `PromisedFiles::Received { paths, failed }`. Home (`src/home/useFileDrop.ts`) opens the import sheet with the paths, as for a drop of files, and shows a message when the other app did not give a file.
3. Count the files from `fileTypes`, read with a call that accepts nil (`promised_count`, line 440):

   ```rust
   let types: Option<Retained<NSArray>> = unsafe { msg_send![receiver, fileTypes] };
   types.map_or(1, |types| types.count().max(1))
   ```

4. Run each step in an AppKit callback through `guarded` (line 286): `std::panic::catch_unwind` around `objc2::exception::catch` (objc2 feature `exception`, `src-tauri/Cargo.toml:95`). A panic or an Objective-C exception gives `None` and a log line, and the drop goes on as wry handles it. The reader blocks run on an operation-queue thread, so they are guarded too.
5. Accept `qta` (`crates/audio-import/src/lib.rs:24`). The `mvhd` atom gives its recording time, as for `m4a` (`crates/audio-import/src/metadata.rs:31`).

The import worker copies a file later, so a received file must stay until then. Each start removes the folders in `received-files` that are older than 7 days (`KEEP_RECEIVED`, line 40).

## Why This Works

A promise drag has the data that `NSFilePromiseReceiver` reads (`com.apple.NSFilePromiseItemMetaData`), and no file URL. Only the drop target can start the write of the file, and wry never does. The replaced methods add that step and leave wry's own work in place. The search up the superclasses finds wry's methods also when macOS adds an observing subclass. The change is on wry's class and not on `WKWebView`, so non-wry webviews do not change. All wry webviews (also Settings) get the replaced methods, but the added steps run only for the main webview's object, so a drop on another window imports nothing.

## Prevention

- In code that AppKit calls (a replaced method, a block on an operation queue), catch Rust panics and Objective-C exceptions, and fall back to the old behaviour. A panic there quits the whole app with no crash report.
- Do not trust a non-null objc2 binding for a value that Apple documents as "available after" an event. Read it with `msg_send!` into an `Option<Retained<_>>`.
- To change a method of a class that a framework defines at run time, start from the object's class and walk the superclasses. Stop before the system class, so that the change cannot reach other users of that class.
- Test a drop from Voice Memos (a promise), from Finder (file URLs) and from a text selection (neither) in a signed build. Unit tests cannot make a real drag.
- To read a panic of the signed app, run `Longhand.app/Contents/MacOS/longhand` from a shell: the panic text goes to stderr, not to `~/Library/Logs/net.ewal.longhand/Longhand.log`. In zsh, call `/usr/bin/log show --predicate 'process BEGINSWITH[c] "longhand"'`, because `log` is a shell builtin.
