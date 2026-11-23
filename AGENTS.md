# Project rules

- User release gate: full texture extraction features with equal or better output quality/accuracy/coverage against Leonida and SixNG, plus a reproducible performance or accuracy advantage. Cached microbenchmarks alone do not establish releasability. Unresolved disagreements remain blockers.
- Combined project is GPL-3.0-only, allowing attributed reuse of SixNG and Leonida. Preserve source headers, upstream revisions, and licenses. Do not copy incompatibly licensed source-available code.
- User explicitly authorizes OpenCode automatic local tool approvals. Use bypassPermissions for workers, keep assigned file ownership and preserve concurrent edits. User also authorizes applicable upstream PRs; use human-writing and bars-voice skills for their prose. The user authorized a public source repository. Tagged releases and package/binary publication remain separate actions.
- Cache correctness is mandatory: scope entries to a source snapshot and decoder/layout identity; invalidate on source update/replacement, reparse on refresh, and test cross-file/build isolation. Do not imply unimplemented PS5/PC adapters are supported.
- Primary performance gate is complete extraction with decoded-result caching disabled in both implementations. Report cache-assisted gains separately. Verify zero cache hits/residency, equal output, and distinguish warm filesystem cache from application cache. Never substitute an empty-at-start cache for disabled caching.
- Game files are read-only inputs. Keep all game data, decoded bytes, local paths, and raw benchmark artifacts outside Git. Synthetic fixtures only in tests.
- Rust validation: cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test. Cargo is used for Rust; Bun for any JavaScript package management.
- Own changes narrowly when collaborating; do not revert concurrent work.
- Unsupported formats fail explicitly. Never silently zero-fill data or label fitted reconstruction exact.
