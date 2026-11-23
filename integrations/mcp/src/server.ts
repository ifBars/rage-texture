import { execFile } from "node:child_process";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const config = process.argv[2];
if (!config) throw new Error("Usage: bun src/server.ts <config.json>");
const worker = fileURLToPath(new URL("./research.py", import.meta.url));
const run = promisify(execFile);
const server = new McpServer({ name: "rage-research", version: "0.1.0" });
const source = z.string().min(1).max(64);
const path = z.string().min(1).max(512);
const offset = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const term = z.string().min(2).max(64);
const locator = z.object({
  source,
  path,
  offset,
  sha256: z.string().regex(/^[a-f0-9]{64}$/),
});
let busy = false;

function tool(name: string, description: string, inputSchema: z.ZodRawShape) {
  server.registerTool(name, { description, inputSchema, annotations: { readOnlyHint: true } },
    async (args) => {
      if (busy) return { isError: true, content: [{ type: "text", text: "Another investigation call is running. Retry shortly." }] };
      busy = true;
      try {
        const { stdout } = await run(process.env.RAGE_RESEARCH_PYTHON ?? "python",
          ["-I", worker, config, name, JSON.stringify(args)],
          { timeout: 90_000, maxBuffer: 2 * 1024 * 1024, windowsHide: true });
        const result = JSON.parse(stdout);
        if (name === "inspect_texture" || name === "inspect_game_texture") {
          const { previewBase64, ...metadata } = result;
          return { structuredContent: metadata, content: [
            { type: "text", text: JSON.stringify(metadata) },
            { type: "image", mimeType: "image/jpeg", data: previewBase64 },
          ] };
        }
        return { structuredContent: result, content: [{ type: "text", text: stdout }] };
      } catch (error) {
        const stderr = (error as { stderr?: string }).stderr?.trim();
        return { isError: true, content: [{ type: "text", text: stderr?.slice(0, 500) || "Research worker failed or timed out." }] };
      } finally {
        busy = false;
      }
    });
}

tool("list_sources", "List configured build labels and their game names.", {});
tool("find_files", "Search loose filenames under one configured source. Archive entries are not filesystem paths.", {
  source, query: term, after: path.optional(), limit: z.number().int().min(1).max(100).optional(),
});
tool("search_evidence", "Search a bounded data window for literal ASCII and UTF-16LE terms. RPF9 mode checks readable zlib streams in the data area. Hits are leads, not proof of an Easter egg or cross-game reuse. Resume using nextOffset.", {
  source, path, mode: z.enum(["raw", "rpf9"]), terms: z.array(term).min(1).max(8),
  offset: offset.optional(), bytes: z.number().int().min(1).max(64 * 1024 * 1024).optional(),
  limit: z.number().int().min(1).max(100).optional(),
});
tool("inspect_stream", "Replay a zlib lead at its archive offset and verify its decoded payload digest before returning context.", {
  locator, terms: z.array(term).min(1).max(8),
});
tool("compare_evidence", "Compare an RPF9 stream with another verified stream or an already extracted loose file, such as a GTA V resource in a separate source. Different bytes allow for re-encoding. Equal bytes establish content identity only.", {
  left: locator, right: z.union([locator.strict(), z.object({ source, path }).strict()]),
});
const imageFields = {
  source, path, sha256: z.string().regex(/^[a-f0-9]{64}$/).optional(), ocr: z.boolean().optional(),
  crop: z.object({ x: offset, y: offset, width: z.number().int().positive(), height: z.number().int().positive() }).optional(),
};
const textureHash = z.string().regex(/^(?:0x)?[0-9a-fA-F]{16}$/);
tool("inspect_texture", "View an exported PNG or crop from a configured source. Enable optional English OCR for candidate words with confidence and original pixel boxes [left, top, right, bottom]. Inspect the preview to verify handwriting or uncertain readings.", {
  ...imageFields,
});
tool("find_texture", "Find a 64-bit asset key in TTPF catalogs under a game source. Resume with nextAfter. Check errors for unreadable catalogs.", {
  source, textureHash, after: path.optional(), storeLimit: z.number().int().min(1).max(64).optional(),
});
tool("inspect_game_texture", "Decode a TTPF texture in memory by path and key from find_texture. Returns a preview, decoded PNG digest and quality diagnostics. Crops and optional English OCR use coordinates of the selected mip. Fitted decoding requires explicit opt-in.", {
  ...imageFields, textureHash, catalogIndex: offset.optional(), mip: offset.optional(), slice: offset.optional(),
  decodePolicy: z.enum(["lossless", "fitted"]).optional(),
});
tool("used_by", "Find the materials and indexed models using a 64-bit texture hash. Follow other slots on each material for related maps. Results come from the configured SixNG texture-users index. Pass indexSha256 when paging to pin the same index.", {
  source,
  textureHash: z.string().regex(/^(?:0x)?[0-9a-fA-F]{16}$/),
  offset: offset.optional(),
  limit: z.number().int().min(1).max(20).optional(),
  modelOffset: offset.optional(),
  modelLimit: z.number().int().min(1).max(10).optional(),
  indexSha256: z.string().regex(/^[0-9a-f]{64}$/).optional(),
});

server.registerPrompt("investigate_ufo", {
  description: "Investigate UFO-related leads and possible links between configured GTA VI and GTA V sources.",
}, () => ({
  messages: [{ role: "user", content: { type: "text", text:
    "What UFO-related content or Easter eggs are supported by the local game files, and is there evidence of a GTA V connection? List the sources. Search loose filenames and bounded readable RPF9 windows for UFO, saucer, Chiliad, and alien. Prioritize smaller script, common, and data archives if present. For useful leads, replay the evidence and cite the source, relative archive path, byte offset and digest. For texture hashes, locate stores with find_texture and inspect candidates with inspect_game_texture. Inspect relevant exported PNGs with inspect_texture and verify any OCR against the crop. Group names, text and exact payload matches by what they support. Treat shared words as leads. Explain the scan coverage, and report GTA V as unavailable if no matching source is configured." } }],
}));

await server.connect(new StdioServerTransport());
