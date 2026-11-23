# Third-party code and licenses

The combined project is GPL-3.0-only. LICENSE contains the full GPL text.
LICENSE-MIT records the original reader foundation's additional grant; it does
not relicense the GPL-derived files listed below.

## Bundled sources

- `src/bc6h.rs`: adapted from SixNG contributors' existing PR #23,
  https://github.com/holdmysocks/SixNG/pull/23, revision
  `2979c57fdeb35863bb8885e6b48d09090cd1f7de`, GPL-3.0-only. The adaptation
  adds checked image bounds, RGB half-float output and canonical signed-zero
  handling. Retain the upstream credit; the decoder correction is not claimed
  as a new project invention. bcdec is used only as an external test oracle,
  not bundled or linked into the library.

- `src/bc3pack*.rs`, `src/bc45pack*.rs`, `src/bc6pack*.rs`, `src/bc7pack*.rs`:
  SixNG contributors, https://github.com/holdmysocks/SixNG, revision
  `7881042c294f3eeea1e8153e7bcafd84f9d787b9`, GPL-3.0-only. Copies carry
  attribution headers; their algorithms and fitted tables remain upstream code.
- `src/bc7pack_tables.rs` additionally incorporates SixNG's 34 fitted endpoint
  presets from revision `d724355df2398a646eb771d6d3da17c3b1bafe11` (PR #31),
  GPL-3.0-only. These remain provisional; they do not establish confirmed
  initial history or original-pixel accuracy.
- `src/layout.rs`, `src/dds.rs`: attributed Rust ports of Leonida's Gfx10
  layout/detiling/DDS rules from `tools/tex_headers.py` and
  `tools/ttpf_mips.py`, https://github.com/ItsNotRexed/Leonida, revision
  `aa2988cc555bad7ecd508b48a702adbb32fe7c95`. Leonida is GPL-3.0-or-later;
  this combined distribution uses GPL version 3. Preserve the file notices.
- `src/history_bc.rs`, `src/history_bc_tables.json`, `src/history_bptc.rs`:
  attributed Rust ports of Leonida `tools/bcpack.py`, `bcpack_tables.py` and
  `bcpack_tables_bptc.py` at revision
  `aa2988cc555bad7ecd508b48a702adbb32fe7c95`, GPL-3.0-or-later, distributed
  here under GPL version 3. Static tables come from that public source, not
  from bundled game data. See HISTORY.md for policy and validation limits.
- RDS5 reader, cache/source-generation guards, catalog integration, CLI and
  preview glue are project code. Format observations were checked against the
  caller's local files; no game data is bundled.

## Direct Rust dependencies

- **bcpack-rs by Ahmet Dedeler**, https://github.com/Ahmet-Dedeler/bcpack-rs,
  revision `5bf5cc56ed56f1ababf1f9592300bd182ff90bd8`: MIT OR Apache-2.0.
  This is a dependency, not copied source. Its entropy tables include Microsoft's
  MIT-licensed silicon-rans work; retain the upstream THIRD_PARTY_NOTICES.md.
- flate2 and zlib-rs: MIT OR Apache-2.0.
- serde, serde_json, sha2, png, texture2ddecoder: MIT OR Apache-2.0.
- same-file: Unlicense OR MIT.
- tempfile: MIT OR Apache-2.0.
- ddsfile 0.5.2: MIT; development-only independent DDS container QA.

Cargo.lock pins the transitive graph. Dependency source distributions retain
individual license files. Binary redistribution must include applicable
third-party notices as well as corresponding GPL source; no binary publication
has been performed by this project.

`scripts/sixng-reference`, `scripts/sixng-corpus-reference` and
`scripts/bc7-current-reference` are separate GPL
validation harnesses depending on the unmodified pinned SixNG core (including
its own dependencies). Neither is linked into the main library or CLI.

Fitted BCPack output remains provisional even when no unfitted blocks are
reported. Matching another fitted decoder is compatibility evidence, not an
independent original-pixel accuracy certificate.

`bindings/python` is an optional, separately built extension using PyO3
0.30.1 (MIT OR Apache-2.0). Its wheel includes this notice and GPL license;
public binary redistribution still requires the complete dependency-license
and corresponding-source requirements above. Local wheels are development
artifacts; the SDK has not been published.
