---
title: reqwest 0.13 keeps native-tls only while the joined features select it
date: 2026-10-19
category: integration-issues
module: HTTP clients (crates/summarize/src/client.rs, crates/models/src/download.rs)
problem_type: best_practice
component: service_layer
severity: medium
applies_when:
  - You update reqwest, or a crate that depends on reqwest, such as tauri-plugin-updater
  - You add a reqwest feature (for example http3) in any crate of the build
  - A TLS test in crates/summarize fails after a dependency change
tags:
  - reqwest
  - native-tls
  - rustls
  - cargo-features
  - tauri-plugin-updater
---

# reqwest 0.13 keeps native-tls only while the joined features select it

## Context

PR twistymaze/longhand#50 updated reqwest from 0.12 to 0.13 in `crates/models` and `crates/summarize`. In reqwest 0.13, rustls is the default TLS backend. Both crates set `default-features = false` and the `native-tls` feature.

`tauri-plugin-updater` 2.12 also uses reqwest 0.13 and enables rustls features. Before the update, Longhand's crates used reqwest 0.12 and the updater used reqwest 0.13, so they were two different crates. After the update, they are one crate. Cargo joins the features of a crate for the whole build, so Longhand's clients get a reqwest that has both native-tls and rustls.

## Guidance

The backend that `reqwest::Client::builder()` uses comes from `TlsBackend::default` in reqwest 0.13.5 (`src/tls.rs`, lines 621-635):

- `NativeTls` when the internal feature `__native-tls` is on and `http3` is off.
- `Rustls` when `http3` is on, or when `__native-tls` is off.

Longhand's clients do not set a backend. They use native-tls only because of this default. If a crate in the build enables `http3` on reqwest, the backend changes to rustls, and no compiler error or warning occurs.

The proof at run time is the TLS test in `crates/summarize`. `is_tls_failure` (`crates/summarize/src/client.rs:905`) looks for `native_tls::Error` in the cause chain. `a_tls_failure_is_tls_and_not_unreachable` (`crates/summarize/tests/transport.rs:188`) makes a TLS handshake fail and expects `TransportError::Tls`. With rustls, the cause is not a `native_tls::Error`, so this test fails.

## Why This Matters

With rustls, `is_tls_failure` returns false for a certificate problem. The summary client then reports a TLS failure as an unreachable server and can send the request again. The certificate checks also change: native-tls uses the system trust store and its settings, and rustls in reqwest 0.13 uses `rustls-platform-verifier`.

## When to Apply

- Before you merge a reqwest or tauri-plugin-updater update, run `cargo test -p summarize` and make sure that the TLS tests pass.
- When you add a dependency, run `cargo tree -e features -i reqwest` and look for `http3`.
- If the joined features cannot keep native-tls, set the backend on each client builder (`ClientBuilder::tls_backend_native()` in reqwest 0.13) and do not depend on the default.

## Examples

After the update, `cargo tree -i reqwest -e normal --depth 1` showed one reqwest for all users:

```text
reqwest v0.13.5
├── models
├── summarize
└── tauri-plugin-updater v2.12.0
```

Related: [A dependency feature changed the key order of the shared settings file](dependency-feature-changed-json-key-order.md) records another effect of joined Cargo features.
