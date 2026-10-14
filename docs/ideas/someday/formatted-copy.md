---
title: Copy with formatting
description: "Copy writes a formatted version and the markdown together, so rich-text apps paste headings and lists and plain-text apps paste markdown"
area: library-and-sharing
added: 2026-10-14
source: "brainstorm about copy formats, 2026-10-14"
related: []
---

# Copy with formatting

## Idea
The Copy button on the Summary, Transcript and My Notes tabs writes two formats in one clipboard item:

- A formatted version (HTML). Mail, Notes, Slack, Word and Google Docs paste headings, bold, lists and links.
- The markdown text, the same text that Copy writes today. Terminals, code editors and markdown apps paste exact markdown.

The app that receives the paste chooses the format. macOS and Windows both support more than one format in one clipboard item.

The formatted version has no Longhand fonts, colours or sizes. The pasted text uses the styles of the receiving app.

## Why
The user asked. Today Copy writes only markdown, so a paste into Mail or Notes shows `#` and `**` marks.

## Notes / Current state
- Today: `src/session/copy.ts` calls `navigator.clipboard.writeText`. R17 says that the copy is the file text; this idea keeps that text as the plain-text format and adds the formatted one.
- Decided: the plain-text format stays markdown, not text with the marks removed.
- For planning: write both formats with `navigator.clipboard.write` and a `ClipboardItem` (`text/html` and `text/plain`) in WKWebView and WebView2. Make the HTML with the markdown renderer that the app already uses. Find out whether a click in WKWebView is a sufficient user gesture, and whether some apps need RTF also.
- If the formatted write fails, write the markdown only. Show the current error only when both fail.
