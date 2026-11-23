# rage-research-mcp

A local MCP server for investigating GTA VI game files. Search for a lead,
see exactly where it came from, and replay the evidence before drawing a
conclusion.

A question this is built for: **What UFO-related content is in these files,
and is there evidence it connects to GTA V?**

## Setup

Use Bun and 64-bit Python 3.11 or newer. Build and install the
[rage-texture Python binding](../../bindings/python/README.md) from this checkout.
Install Pillow in the same Python environment. Activate that environment
or set `RAGE_RESEARCH_PYTHON` to its absolute Python executable.

From the repository root:

```sh
cd integrations/mcp
bun install --frozen-lockfile
bun test
```

Copy `config.example.json` to `config.local.json` and set the source directories.
Use a separate source for each game or build. For GTA V, point at a directory
of already readable loose or extracted resources.

Configure your MCP client to launch the server over stdio. Use absolute paths:

```json
{
  "mcpServers": {
    "rage-research": {
      "command": "bun",
      "args": [
        "D:/tools/rage-texture/integrations/mcp/src/server.ts",
        "D:/tools/rage-texture/integrations/mcp/config.local.json"
      ]
    }
  }
}
```

If Python isn't available as `python`, set `RAGE_RESEARCH_PYTHON` to the path
of the interpreter in your server environment.

## Investigate a lead

Ask the question above or use the `investigate_ufo` prompt in an MCP client
that supports prompts. The same tools work with other search terms.

| Tool | What it does |
| --- | --- |
| `list_sources` | Lists configured game and build labels. |
| `find_files` | Filters loose filenames within one source. |
| `search_evidence` | Finds literal ASCII and UTF-16LE matches in loose bytes or verified RPF9 zlib streams. |
| `inspect_stream` | Reopens the exact stream and checks its SHA-256 before showing more context. |
| `compare_evidence` | Compares a verified stream with another stream or a loose file of up to 8 MiB. |
| `find_texture` | Looks up a 64-bit asset key across a build's TTPF catalogs. |
| `inspect_game_texture` | Decodes a selected store record in memory for visual inspection or OCR. |
| `inspect_texture` | Shows an exported PNG or crop and optionally reads English lettering. |
| `used_by` | Finds a texture's materials, other texture slots, and models using a SixNG index. |

Search reports include byte offsets, coverage counters and a continuation
offset. A stream match records the archive-relative offset, payload SHA-256
and each match's offset within the decoded stream. Resume with `nextOffset`.
Raw byte hits may occur by chance, especially on short terms. Check nearby
text and look for corroboration across resources. Matching payloads prove
byte identity. Differing payloads can still contain related or re-encoded
content.

## Read lettering on a texture

Starting from a texture hash in a material, call `find_texture` with the
source and 16-digit hex `textureHash`. Follow `nextAfter` with `after`
until it is null to see all matches. Each page reports any unreadable stores.
Catalogs identify textures by key. For a clue known only by its wording,
start with `search_evidence` or an identified model's texture references.

Pass the returned `path`, `catalogIndex` and `textureHash` with the same
source to `inspect_game_texture`. It opens the store with the decoded cache
disabled and renders mip 0, slice 0 by default. Select another `mip` or
`slice` as needed. Lossless decoding is the default. If you explicitly
request `decodePolicy: "fitted"`, check the returned chunk quality counts
and diagnostics before citing the image. Pass the returned PNG `sha256` to
pin follow-up crops. The original game file stays read-only.

For previously exported images, add your decoded PNG export directory as its own source. Use `find_files`
to locate an atlas, then call `inspect_texture` with its source and relative
path. You get a preview and the SHA-256 of the original PNG. To zoom in,
pass that digest as `sha256` and a `crop` with `x`, `y`, `width`, and
`height` in original pixels from the top left.

Set `ocr: true` to run local Tesseract with English language data on the
selected region. Install Tesseract on PATH or set `RAGE_RESEARCH_TESSERACT`
to the executable. The server also checks its default Windows installation.
Each word has a confidence score and a box in the original atlas. Check
handwriting against the preview. Searches never start an OCR scan.

The loose-image tool accepts one static PNG of up to 32 MiB and 16 megapixels. The
preview is a JPEG up to 768 pixels on its longest side, composited on
white. OCR processes the selected region separately. Keep your export
manifest to trace the PNG digest back to its game archive.

## Follow a texture to its materials

SixNG's **Used By** panel starts with a texture hash and shows the materials
that use it. Use the same index in the MCP to answer questions like “Which
normal and roughness maps belong with this base colour, and which models use
them?” [SixNG issue #18](https://github.com/holdmysocks/SixNG/issues/18)

Build an index in SixNG or reuse one from your existing workspace. For a
small first pass, index just `data` and build its material references:

```sh
sixng --game <game-root> --work <sixng-workspace> index data
sixng --game <game-root> --work <sixng-workspace> texture-users data
```

Set `textureUsersIndex` on the matching source in `config.local.json` to
`<sixng-workspace>/index/texture_users.jsonl.gz`. Relative paths resolve
against the config folder. The server reads that configured file on demand.
Keep a separate index for each build and regenerate it when your sources
change. The index schema does not include a source build fingerprint.

Call `used_by` with the source ID and a 16-digit hexadecimal texture hash
from a texture catalog or material. Each result gives the shader, the matching
parameters, sibling slots and the models listed for that material. Use
`nextOffset` for more materials and `nextModelOffset` for more models.
Pass the returned `indexSha256` when paging. The index digest and one-based
JSONL row numbers identify the evidence. Results cover the models SixNG
indexed successfully for those archives. Large material slot lists return
up to 64 slots with the full slot count.

## Coverage

RPF9 search scans for readable, aligned zlib streams in the data area. It
checks stream completion and bounds the input, output and work per call.
It can miss content stored through other codecs, packed across streams
or encoded without searchable text. It does not resolve the archive table
of contents, decode script logic, read GTA V RPF7 archives or establish
whether a resource appears in playable content. A missed match only covers
the bytes and methods reported by the search.

The server reads local files under explicitly configured roots. It has no
export or write tool. Search snippets are returned to your MCP client, so
choose a client and model appropriate for the data you're reading. Keep
local configs, game files and investigation reports out of Git.

The RPF9 header and observed alignment were cross-checked against the public
[SixNG format notes](https://github.com/holdmysocks/SixNG/blob/702435cd56fd09de4f456ddc94d60886de7365ee/crates/sixng-core/src/rpf9.rs)
and [Leonida resource indexing](https://github.com/ItsNotRexed/Leonida/blob/aa2988cc555bad7ecd508b48a702adbb32fe7c95/tools/rpf9_names.py).
The scanner is independently implemented using Python's standard library.
