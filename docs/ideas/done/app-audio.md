---
title: App audio
description: "Record the microphone plus the audio of the apps on a list (Teams, Zoom, Discord, a browser), as a Loopback device does; replaces whole-computer capture"
area: capture
added: 2026-10-22
source: "Brainstorm of 2026-10-22, from the user's Rogue Amoeba Loopback setup"
related: [system-audio-capture]
plan: docs/plans/2026-10-22-0844-feat-app-audio-plan.md
---

# App audio

## Idea
Settings holds a list of apps. Every note records the microphone plus the audio of those apps, saves them as two channels, and processes a plain mix. The note menu can add or remove an app for one note.

## Why
The user recorded calls for several weeks through a Loopback virtual device that mixes the microphone with chosen apps, on speakers, and the notes were good. Longhand can do the same without Loopback. A list of chosen apps also keeps out the mixer apps that made the parked whole-computer capture fail.

## Outcome
Shipped on 2026-10-23 in [PR 63](https://github.com/twistymaze/longhand/pull/63). Settings has the app list (App audio access, Add apps, then the apps), and the note menu has App Audio. macOS records the apps with a process tap; Windows 11 uses one process loopback client for each app, and records the output device of Teams while Teams plays. Processing uses a plain mix; the echo canceller is removed. The same PR makes Verify save a note that misses some speech, with a short warning on the Transcript tab. Not done yet: the hardware measurement of the two provisional levels, and a test on Windows hardware.
