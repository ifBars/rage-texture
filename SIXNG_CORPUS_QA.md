# Independent SixNG corpus reference

The reproducible harness is included at
[scripts/sixng-corpus-reference](scripts/sixng-corpus-reference/README.md).
It links pinned SixNG rather than the SDK. Both the reader and BCPack calls
come from that separate dependency graph.

The full run completed all 57 stores and 1,026,833 chunks:
151,568,699,392 decoded bytes, zero failures. Every store's ordered digest,
chunk count, successful byte count and unfitted/invalid diagnostics agrees
with the native audit. The complete reconciliation is described in
[CORPUS_QA.md](CORPUS_QA.md).

## Versions

- SixNG: 7881042c294f3eeea1e8153e7bcafd84f9d787b9.
- Its BC1 dependency: bcpack-rs 02a8c4e802800d55b55797a32dbfe8b16d7fc23f.
- Reference executable SHA-256:
  b2507f08abb0bda969d2613a2b4ef67a4ee8ebba029b32d7c99cf83f934372c1.
- Source bundle (main.rs, Cargo.toml and Cargo.lock):
  61604c028545e1924e506f6560a84617969346abbab96368502b27bb3006d1b7.

Each chunk is decoded once, hashed and dropped. There is no decoded cache or
automatic skip of prior results. A nonempty output directory is rejected.
Metadata/table stamps are diagnostics rather than content identities.
A future cache would require stronger source and decoder identity.

The reference keeps SixNG's zlib behavior; the SDK additionally validates
stream completion, checksums and trailing bytes. Wrong decoded lengths,
unknown codecs and failed reads/decodes are failures, never blank successes.
The file-length sentinel and post-sentinel ETPF entries are handled separately.

## Failure controls

A synthetic five-chunk fixture contains two valid chunks, corrupt zlib,
an unsupported codec and a stored-length mismatch. The reference records
failures at indices 1, 2 and 3, and its aggregate equals the concatenation
of successful chunks 0 and 4. Additional small real stores cover all six
observed BCPack codecs and sentinel placement. Formatting, Clippy and release
build checks pass for the standalone harness.

The run validates compatibility with a pinned implementation. It does not
confirm fitted initial histories or establish a general SixNG speed lead.
The upstream BC6H decoder investigation is documented separately in
[UPSTREAM_DECODER_QA.md](UPSTREAM_DECODER_QA.md).

## Later BC7 preset update

The original harness and fingerprints stay pinned for reproducibility.
SixNG PR #31 subsequently added 34 fitted BC7 endpoint presets. A second,
small harness pins d724355df2398a646eb771d6d3da17c3b1bafe11 at
[scripts/bc7-current-reference](scripts/bc7-current-reference/README.md).
All 5,167 affected BC7 chunks match the updated SDK, including all 251 whose
hashes changed. Missing BC7 preset flags fall from 839,119 to zero; every
result remains provisional and the built-in confirmed policy refuses all
5,167. This is an exhaustive affected-codec check layered onto the earlier
full-corpus run, not a second million-chunk audit or an accuracy certificate.
