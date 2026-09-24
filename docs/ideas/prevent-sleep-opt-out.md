---
title: Let the laptop sleep during a session (opt-out)
status: open
area: capture
added: 2026-09-23
source: "the retired FUTURE-IDEAS.md"
related: []
---

# Let the laptop sleep during a session (opt-out)

## Idea
A setting to stop holding the sleep assertion while recording: "let the laptop survive the session instead."

## Why
Holding a sleep assertion for four hours on battery is a real cost.

## Current state
Recording always holds a power assertion (`crates/power`); there is no setting to turn it off.
