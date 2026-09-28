---
title: Windows Rust selected Git's linker after MSVC setup
date: 2026-09-28
module: Windows CI and native helper builds
problem_type: build_error
component: infrastructure
severity: high
symptoms:
  - "The native NeMo helper compiled and started, but subsequent Rust build scripts failed to link."
  - "Rust invoked Git's link.exe with Microsoft linker arguments."
root_cause: config_error
resolution_type: config_change
framework_version: Rust 1.94.0
tags:
  - windows-ci
  - msvc
  - git-bash
  - powershell
  - native-helper
---

# Windows Rust selected Git's linker after MSVC setup

## Problem

Adding the NeMo helper required an initialized Microsoft C++ build environment.
The helper compiled, packaged its dependencies, and passed its protocol check.
Rust then failed while linking dependency build scripts. The error named Git's
`link.exe`, not the Microsoft linker.

[PR #18](https://github.com/twistymaze/longhand/pull/18) fixed the problem.
The important diagnostic distinction was that native helper success did not
prove that a later Cargo command selected the same compiler tools.

## Root cause

The workflow added `ilammy/msvc-dev-cmd` for the helper. It set `VCINSTALLDIR`
and `VSCMD_ARG_TGT_ARCH`. In the Rust 1.94 tool-discovery path inspected during
diagnosis, a matching developer environment caused discovery to use the first
matching tool on `PATH`. Git Bash placed its own tools before Microsoft tools.
Git's filesystem utility has the same `link.exe` name as the Microsoft linker.

Before this environment change, Rust used Visual Studio discovery. Thus a
previously working Bash-based Rust step could fail without any change to its
Cargo command. CMake's successful use of explicit compiler paths did not rule
out this separate tool-discovery failure.

The [MSVC action documentation](https://github.com/ilammy/msvc-dev-cmd#name-conflicts-with-shell-bash)
describes this name conflict. The diagnosis also inspected
[find-msvc-tools 0.1.5](https://docs.rs/crate/find-msvc-tools/0.1.5/source/src/find_tools.rs),
the version in the Rust 1.94 compiler dependency graph.

## Solution

In `.github/workflows/ci.yml`, all Windows steps that invoke Rust use a literal
`shell: pwsh`: the early helper tests, workspace tests, and Tauri installer build.
The native helper build script still runs through Bash. It does not invoke Cargo.

The early test checks the selected linker before compiling the expensive SDK.
PowerShell application discovery can return multiple matches. Select the first
match before comparing its path:

```powershell
$actual = (Get-Command link.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
```

Comparing all returned `.Source` values to one expected path falsely rejected a
valid environment with Microsoft first and Git second. An actual PowerShell
fixture reproduced that error. The corrected fixture accepted Microsoft first,
rejected Git first, and rejected a missing linker.

Use separate OS-gated steps when the shells differ. GitHub rejected a `runner.os`
expression in the step's `shell` field before it created any jobs. The PR then
showed required checks as "Expected" rather than showing a failed job. YAML
parsing did not detect this restriction; `actionlint` did.

## Validation and prevention

[The final CI run](https://github.com/twistymaze/longhand/actions/runs/36423174397)
passed the early Rust tests, full workspace tests, installer build, and CPU
inference with the helper extracted from the Windows installer. The macOS job
also passed. This does not establish Windows GPU inference or meeting-length
recording behavior.

When adding a native build environment, inspect every later Rust invocation,
including commands inside packaging tools. Check the selected tool in the shell
that will execute it. Validate workflow changes with `actionlint`, and execute
PowerShell checks in PowerShell rather than relying on syntax inspection alone.

Keep the full-workspace Cargo invocation intact. Splitting it can cause the
separate [feature-unification rebuild problem](../performance-issues/cargo-feature-unification-ci-rebuilds.md).
