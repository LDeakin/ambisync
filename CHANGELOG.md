# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased](https://github.com/LDeakin/ambisync/compare/v0.1.0...HEAD)

### Added
- `#[ambisync::paired]` now accepts `impl` blocks (trait and inherent) in addition to traits
- `#[ambisync::paired]` now supports `#[sync_only]` / `#[async_only]` on async methods
- Add trusted publishing
- `#[sync_only]` / `#[async_only]` on a generic parameter keep it, and the where-clause predicates bounding it, in one variant only

### Changed
- Trait and impl methods that no `fns(...)` entry matches keep their names when the sync trait or impl is distinct from the async one, instead of failing to expand
- Clarify the error for a method whose sync name cannot be derived, suggesting `#[sync_name(...)]`

### Fixed
- Pattern entries no longer rename prose words in doc comments (`types("Async{}")` turned "Asynchronously" into "hronously"); they now apply only inside code spans and link text

## [0.1.0](https://github.com/LDeakin/ambisync/releases/tag/v0.1.0) - 2026-07-21

### Added
- `#[ambisync]`: emit an async and a mechanically derived sync variant of an async-written `fn`, `trait`, `impl`, `struct`, or `enum`.
- `#[ambisync::test]`: emit a sync `#[test]` and a cfg-gated async test from one async-written test fn.
- `#[ambisync::scope]` / `ambisync::scoped!`: shared defaults for inline and file modules.
- `sync(types(...))` / `sync(fns(...))` rename tables: exact `A => B` entries and string-literal pattern entries (`types("Async{}")`, `fns("{}_async", "create_async_{} => create_{}")`) that both name the annotated items and rename matching types/calls/doc mentions, with exact entries taking precedence over patterns.
- Marker macros `alt!`, `sync_only!`, `async_only!` and the `#[sync_only]` / `#[async_only]` / `#[sync_attr(...)]` / `#[async_attr(...)]` helper attributes for divergent code.
- `#[sync_bounds(...)]` / `#[async_bounds(...)]` helper attributes for variant-specific generic type parameter bounds, including empty overrides that remove all bounds.
- `#[sync_name(...)]` for assigning an exact sync name to an individual trait or impl method, including non-async methods in an async API; calls follow the override automatically.
- Best-effort macro-body rewriting also renames turbofish call spellings (`name::<T>(...)`), not just `name(...)`.
- Add `#[sync_signature(fn ...)]` for trait and impl methods whose sync signature cannot be mechanically derived from the async signature
- Add `#[ambisync::paired]` for deriving synchronous siblings of async methods that coexist in one mixed trait
