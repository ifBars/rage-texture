import { afterAll, expect, test } from "bun:test";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync, gzipSync } from "node:zlib";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const scratch = await mkdtemp(join(tmpdir(), "rage-research-test-"));
const vi = join(scratch, "vi");
const v = join(scratch, "v");
await mkdir(vi);
await mkdir(v);
const config = join(scratch, "fixtures.json");
const textureIndex = join(scratch, "texture_users.jsonl.gz");
const target = "ffffffffffffffa1";
const related = "00000000000000b2";
function writeIndex(rows: object[]) {
  return writeFile(textureIndex, gzipSync(rows.map((row) => JSON.stringify(row)).join("\n") + "\n"));
}
const materials = [
  { m: ["decal", [["diffuse", target]]] },
  { m: ["standard", [["diffuse", target], ["normal", related]]] },
  { m: ["other", [["packed", related]]] },
];
const models = [
  { id: "levels.rpf@40#prop", name: "prop", mats: [0, 1, 1] },
  { id: "levels.rpf@80", name: "prop_lod", mats: [1] },
  { id: "levels.rpf@100", name: "unrelated", mats: [2] },
];
const indexRows = [{ texture_users: 1, materials: 3, models: 3 }, ...materials, ...models];
await writeIndex(indexRows);
await writeFile(config, JSON.stringify({ sources: [
  { id: "vi", root: vi, game: "Synthetic VI", textureUsersIndex: "texture_users.jsonl.gz" },
  { id: "v", root: v, game: "Synthetic V" },
] }));

const header = Buffer.alloc(64);
header.write("9FPR");
header.writeUInt32LE(64, 4);
const payload = Buffer.concat([Buffer.from("obj_ufo_marker UFOlogy\0"), Buffer.from("Flying Saucer", "utf16le")]);
const hidden = Buffer.from("alien clue in second stream");
const frame = (data: Buffer) => {
  const stream = deflateSync(data);
  return Buffer.concat([stream, Buffer.alloc((16 - stream.length % 16) % 16)]);
};
await writeFile(join(vi, "sample.rpf"), Buffer.concat([header, frame(payload), frame(hidden)]));
await writeFile(join(vi, "ufo-notes.txt"), "xx UFO xx UFO");
await writeFile(join(vi, "bad.rpf"), Buffer.from("wrong header"));
await writeFile(join(v, "ufo-resource.bin"), payload);
await writeFile(join(v, "changed.bin"), Buffer.concat([payload, Buffer.from(" extra")]));

const client = new Client({ name: "integration-test", version: "1.0.0" });
await client.connect(new StdioClientTransport({ command: process.execPath, args: ["src/server.ts", config],
  env: { RAGE_RESEARCH_PYTHON: process.env.RAGE_RESEARCH_PYTHON ?? "python" } }));
afterAll(async () => {
  await client.close();
  await rm(scratch, { recursive: true, force: true });
});
const call = (name: string, args: Record<string, unknown> = {}) => client.callTool({ name, arguments: args });
async function data(name: string, args?: Record<string, unknown>) {
  const result = await call(name, args);
  if (result.isError) throw new Error(JSON.stringify(result.content));
  return result.structuredContent as any;
}

test("discovers sources, tools and the UFO prompt over stdio", async () => {
  expect((await client.listTools()).tools).toHaveLength(9);
  expect((await data("list_sources")).sources.map((s: { id: string }) => s.id)).toEqual(["vi", "v"]);
  expect((await client.getPrompt({ name: "investigate_ufo" })).messages[0].role).toBe("user");
  expect((await data("find_files", { source: "vi", query: "ufo" })).files).toEqual(["ufo-notes.txt"]);
});

test("ranks materials, returns exact sibling hashes and pages model evidence", async () => {
  const query = { source: "vi", textureHash: "0x" + target.toUpperCase(), limit: 1, modelLimit: 1 };
  const first = await data("used_by", query);
  expect(first.textureHash).toBe(target);
  expect(first.matchedMaterials).toBe(2);
  expect(first.matchedModels).toBe(2);
  expect(first.materials[0].materialId).toBe(1);
  expect(first.materials[0].indexLine).toBe(3);
  expect(first.materials[0].slots).toEqual([
    { parameter: "diffuse", textureHash: target, matchesTarget: true },
    { parameter: "normal", textureHash: related, matchesTarget: false },
  ]);
  expect(first.materials[0].models[0].id).toBe("levels.rpf@40#prop");
  const pinned = { ...query, indexSha256: first.indexSha256 };
  const moreModels = await data("used_by", { ...pinned, modelOffset: first.materials[0].nextModelOffset });
  expect(moreModels.materials[0].models[0].name).toBe("prop_lod");
  expect(moreModels.materials[0].nextModelOffset).toBeNull();
  const moreMaterials = await data("used_by", { ...pinned, offset: first.nextOffset });
  expect(moreMaterials.materials[0].shader).toBe("decal");
  expect(moreMaterials.nextOffset).toBeNull();
  const missing = await data("used_by", { ...query, textureHash: "0000000000000001" });
  expect(missing.matchedMaterials).toBe(0);
  expect((await call("used_by", { ...query, source: "v" })).isError).toBe(true);
  expect((await call("used_by", { ...query, textureHash: "0000000000000000" })).isError).toBe(true);
  await writeIndex(indexRows.slice(0, -1));
  try {
    const changed = await call("used_by", pinned);
    expect(JSON.stringify(changed.content)).toContain("index changed");
    const truncated = await call("used_by", query);
    expect(truncated.isError).toBe(true);
  } finally {
    await writeIndex(indexRows);
  }
});

test("scans verified streams, resumes, replays and compares evidence across game roots", async () => {
  const search = { source: "vi", path: "sample.rpf", mode: "rpf9", terms: ["ufo", "saucer", "alien"], limit: 1 };
  const first = await data("search_evidence", search);
  expect(first.results[0].matches.map((m: { encoding: string }) => m.encoding)).toEqual(["ascii", "utf-16le"]);
  expect(first.results[0].matches[0].context).toContain("obj_ufo_marker");
  expect(first.results[0].matches).toHaveLength(2);
  const loc = first.results[0].locator;
  expect(loc.offset).toBe(64);
  const second = await data("search_evidence", { ...search, offset: first.nextOffset });
  expect(second.results[0].matches[0].term).toBe("alien");
  expect((await data("inspect_stream", { locator: loc, terms: ["saucer"] })).matches).toHaveLength(1);
  expect((await data("compare_evidence", { left: loc, right: { source: "v", path: "ufo-resource.bin" } })).relation).toBe("identical-decoded-payload");
  expect((await data("compare_evidence", { left: loc, right: { source: "v", path: "changed.bin" } })).relation).toBe("different-decoded-payload");
  expect((await call("inspect_stream", { locator: { ...loc, sha256: "0".repeat(64) }, terms: ["ufo"] })).isError).toBe(true);
});

test("finds hits crossing raw window boundaries and rejects escaping paths or bad archives", async () => {
  const search = { source: "vi", path: "ufo-notes.txt", mode: "raw", terms: ["ufo"], bytes: 4 };
  const first = await data("search_evidence", search);
  expect(first.results[0].byteOffset).toBe(3);
  expect((await data("search_evidence", { ...search, offset: first.nextOffset })).results).toHaveLength(0);
  expect((await call("search_evidence", { ...search, path: "../v/changed.bin" })).isError).toBe(true);
  expect((await call("search_evidence", { ...search, mode: "rpf9", path: "bad.rpf" })).isError).toBe(true);
});

test("previews original-pixel crops and pins them to the exported PNG", async () => {
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAgAAAAGCAIAAABxZ0isAAAAFElEQVR4nGP8//8/AzbAhFWUThIAvm0DCRxYCEQAAAAASUVORK5CYII=", "base64");
  await writeFile(join(vi, "atlas.png"), png);
  const query = { source: "vi", path: "atlas.png", crop: { x: 3, y: 1, width: 5, height: 2 } };
  const result = await call("inspect_texture", query);
  expect(result.isError).toBeFalsy();
  const digest = new Bun.CryptoHasher("sha256").update(png).digest("hex");
  expect(result.structuredContent).toEqual({ source: "vi", path: "atlas.png", sha256: digest,
    width: 8, height: 6, crop: query.crop });
  expect(result.content).toContainEqual(expect.objectContaining({ type: "image", mimeType: "image/jpeg" }));
  expect((await call("inspect_texture", { ...query, sha256: "0".repeat(64) })).isError).toBe(true);
  expect((await call("inspect_texture", { ...query, crop: { ...query.crop, x: 4 } })).isError).toBe(true);
  expect((await call("inspect_texture", { ...query, path: "../v/changed.bin" })).isError).toBe(true);
});

test("finds, decodes and pins a catalog record directly from a synthetic TTPF", async () => {
  const path = join(vi, "pool.ttpf");
  const fixture = Bun.spawnSync([process.env.RAGE_RESEARCH_PYTHON ?? "python", "-I", "-c",
    "import runpy,sys; from pathlib import Path; runpy.run_path(sys.argv[1])['make_texture_fixture'](Path(sys.argv[2]))",
    "../../bindings/python/tests/test_binding.py", path]);
  expect(fixture.exitCode).toBe(0);
  await writeFile(join(vi, "bad.ttpf"), "invalid");
  const key = { source: "vi", textureHash: "000000000000007b" };
  const first = await data("find_texture", { ...key, storeLimit: 1 });
  expect(first.errors[0].path).toBe("bad.ttpf");
  const page = await data("find_texture", { ...key, after: first.nextAfter });
  expect(page.nextAfter).toBeNull();
  expect(page.matches).toHaveLength(1);
  expect(page.matches[0]).toMatchObject({ path: "pool.ttpf", catalogIndex: 0, width: 64 });
  const input = { ...key, path: "pool.ttpf", catalogIndex: 0 };
  const preview = await data("inspect_game_texture", input);
  expect(preview.quality).toEqual({ lossless: 1 });
  expect(preview.width).toBe(64);
  const crop = { x: 6, y: 8, width: 10, height: 12 };
  expect((await data("inspect_game_texture", { ...input, sha256: preview.sha256, crop })).crop).toEqual(crop);
  for (const change of [{ sha256: "0".repeat(64) }, { catalogIndex: 1 }, { mip: 1 },
                        { textureHash: "0000000000000001" }, { path: "../v/changed.bin" }]) {
    expect((await call("inspect_game_texture", { ...input, ...change })).isError).toBe(true);
  }
});
