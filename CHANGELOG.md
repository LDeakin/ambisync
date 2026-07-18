# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `#[ambisync]`: emit an async and a mechanically derived sync variant of an
  async-written `fn`, `trait`, `impl`, `struct`, or `enum`.
- `#[ambisync::test]`: emit a sync `#[test]` and a cfg-gated async test from
  one async-written test fn.
- `#[ambisync::scope]` / `ambisync::scoped!`: shared defaults for inline and
  file modules.
- Marker macros `alt!`, `sync_only!`, `async_only!` and the `#[sync_only]` /
  `#[async_only]` / `#[sync_attr(...)]` / `#[async_attr(...)]` helper
  attributes for divergent code.
- `MaybeSend` / `MaybeSync` bounds (equal to `Send`/`Sync` except on `wasm32`).
