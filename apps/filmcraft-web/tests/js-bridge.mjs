// Node end-to-end test of the exported WASM JavaScript bridge against the CLI.
//
//   cargo build --locked -p filmcraft-cli
//   cargo build --locked --target wasm32-unknown-unknown -p filmcraft-web
//   wasm-bindgen --target nodejs --out-dir target/web/node target/wasm32-unknown-unknown/debug/filmcraft_web.wasm
//   node apps/filmcraft-web/tests/js-bridge.mjs target/web/node/filmcraft_web.js

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const wasmPath = resolve(process.argv[2] ?? "target/web/node/filmcraft_web.js");
const cliPath = resolve(process.env.FILMCRAFT_CLI ?? "target/debug/filmcraft-cli");
const { FilmCraftBridge } = createRequire(import.meta.url)(wasmPath);

function cli(args, input) {
  return execFileSync(cliPath, args, { encoding: "utf8", input, stdio: ["pipe", "pipe", "pipe"] });
}

const fields = (command) => ({
  id: command.id,
  label: command.label,
  menu: command.menu,
  shortcut: command.shortcut,
  shortcuts: command.shortcuts,
  defaultShortcut: command.defaultShortcut,
  params: command.params,
});

const bridge = new FilmCraftBridge();
const [bridgeCommands, cliCommands] = [bridge.commands(), JSON.parse(cli(["commands", "--json"]))];
assert.deepEqual(bridgeCommands.map(fields), cliCommands.map(fields), "command metadata matches the CLI");
assert.deepEqual(bridge.commands("seQuence").map(fields), JSON.parse(cli(["commands", "seQuence", "--json"])).map(fields), "filters match the CLI");
assert.deepEqual(bridge.describe("file.newSequence"), JSON.parse(cli(["describe", "file.newSequence"])), "describe matches the CLI");
assert.throws(() => bridge.describe("not.a.command"), /unknown command/);
const bridgePresets = await bridge.execute("export.presets.list", { query: "h264" });
assert.deepEqual(bridgePresets.presets, JSON.parse(cli(["export", "h264", "--list-presets", "--compact"])), "engine preset commands match the CLI");

const script = [
  "# CLI-compatible JSON Lines",
  JSON.stringify({ id: "file.newProject", params: { name: "Bridge CLI parity" } }),
  JSON.stringify({ id: "project.inspect" }),
].join("\n");
const bridgeRun = await bridge.run(script, false);
const cliRun = cli(["run", "-"], script)
  .trim()
  .split("\n")
  .map((line) => JSON.parse(line));
assert.equal(bridgeRun.ok, true);
assert.deepEqual(
  bridgeRun.results.map(({ line, id, ok, result }) => ({ line, id, ok, result })),
  cliRun,
  "JSON Lines results match the CLI",
);

const badRun = await bridge.run(
  `${JSON.stringify({ id: "no.such.command" })}\n${JSON.stringify({ id: "project.inspect" })}`,
  true,
);
assert.equal(badRun.ok, false);
assert.equal(badRun.results.length, 2);
assert.equal(badRun.results[0].ok, false);
assert.equal(badRun.results[1].ok, true);
await assert.rejects(bridge.execute("no.such.command", {}), /unknown command/);

const matte = await bridge.execute("file.newColorMatte", { seconds: 1, color: "#345678" });
const sequence = await bridge.execute("file.newSequence", {
  name: "Bridge export",
  fromItem: matte.item,
  width: 64,
  height: 36,
});
await bridge.execute("timeline.place", { item: matte.item, time: 0 });
const frame = bridge.renderFrame(0, 0.5);
assert.equal(frame.width, 32);
assert.equal(frame.height, 18);
assert.ok(frame.rgba instanceof Uint8Array);
assert.equal(frame.rgba.length, frame.width * frame.height * 4);
assert.ok(sequence.sequence);

const exported = await bridge.exportMedia({
  format: "png",
  path: "bridge-e2e.png",
  range: "custom",
  startSeconds: 0,
  endSeconds: 0.1,
});
assert.ok(exported.path);
const png = bridge.readFile(exported.path);
assert.deepEqual(Array.from(png.subarray(0, 8)), [137, 80, 78, 71, 13, 10, 26, 10], "export bytes are readable");

const h264Exported = await bridge.exportMedia({
  format: "h264",
  path: "bridge-e2e.mp4",
  range: "custom",
  startSeconds: 0,
  endSeconds: 0.1,
  audio: false,
});
const mp4 = bridge.readFile(h264Exported.path);
assert.equal(Buffer.from(mp4.subarray(4, 8)).toString(), "ftyp", "WASM H.264 export produces an MP4");

const wav = Buffer.alloc(44 + 480 * 2);
wav.write("RIFF", 0);
wav.writeUInt32LE(wav.length - 8, 4);
wav.write("WAVEfmt ", 8);
wav.writeUInt32LE(16, 16);
wav.writeUInt16LE(1, 20);
wav.writeUInt16LE(1, 22);
wav.writeUInt32LE(48_000, 24);
wav.writeUInt32LE(96_000, 28);
wav.writeUInt16LE(2, 32);
wav.writeUInt16LE(16, 34);
wav.write("data", 36);
wav.writeUInt32LE(wav.length - 44, 40);
const mediaBytes = new Uint8Array(wav);
const probed = bridge.probe("short.wav", mediaBytes);
assert.equal(probed.name, "short.wav");
assert.equal(probed.audio_streams.length, 1);
const probeDir = mkdtempSync(join(tmpdir(), "filmcraft-bridge-probe-"));
try {
  const probePath = join(probeDir, "short.wav");
  writeFileSync(probePath, wav);
  assert.deepEqual(probed, JSON.parse(cli(["probe", probePath])), "media probe details match the CLI");
} finally {
  rmSync(probeDir, { recursive: true, force: true });
}
await assert.rejects(bridge.benchmarkDecode("short.wav", mediaBytes, 0), /between 1 and 10000/);
assert.throws(() => bridge.pumpJobs(0), /between 1 and 100/);
assert.throws(() => bridge.pumpJobs(101), /between 1 and 100/);
await assert.rejects(bridge.run("#\n".repeat(10_001), false), /10000-line/);
await assert.rejects(bridge.run("x".repeat((8 << 20) + 1), false), /8 MiB/);
await assert.rejects(
  bridge.execute("file.open", { path: "/files/outside.mp4" }),
  /outside this bridge/,
  "file commands are sandboxed to this bridge's virtual filesystem",
);
await assert.rejects(bridge.execute("file.import", { paths: ["/files/outside.mp4"] }), /outside this bridge/);
await assert.rejects(
  bridge.importFiles(new Array(1025).fill({ name: "a.wav", bytes: mediaBytes })),
  /limited to 1024 files/,
);
await assert.rejects(
  bridge.execute("project.inspect", { padding: "x".repeat(17 << 20) }),
  /16 MiB/,
  "oversized parameters are rejected before deserializing",
);
function bmpFrame(r, g, b) {
  const bytes = Buffer.alloc(58);
  bytes.write("BM", 0);
  bytes.writeUInt32LE(bytes.length, 2);
  bytes.writeUInt32LE(54, 10);
  bytes.writeUInt32LE(40, 14);
  bytes.writeInt32LE(1, 18);
  bytes.writeInt32LE(1, 22);
  bytes.writeUInt16LE(1, 26);
  bytes.writeUInt16LE(24, 28);
  bytes.writeUInt32LE(4, 34);
  bytes.set([b, g, r, 0], 54);
  return new Uint8Array(bytes);
}
const sequenceProbe = bridge.probeImageSequence([
  { name: "shot_0001.bmp", bytes: bmpFrame(12, 34, 56) },
  { name: "shot_0003.bmp", bytes: bmpFrame(65, 43, 21) },
]);
assert.equal(sequenceProbe.imageSequence.first, "shot_0001.bmp");
assert.equal(sequenceProbe.imageSequence.firstNumber, 1);
assert.equal(sequenceProbe.imageSequence.frames, 3);
assert.deepEqual(sequenceProbe.imageSequence.missing, [2]);
assert.equal(sequenceProbe.imageSequence.lastNumber, 3);
const imported = await bridge.importFile("short.wav", mediaBytes);
assert.equal(imported.items.length, 1);
assert.ok(imported.path.startsWith("/bridge/"));
const importedMany = await bridge.importFiles([
  { name: "short-a.wav", bytes: mediaBytes },
  { name: "short-b.wav", bytes: mediaBytes },
]);
assert.equal(importedMany.items.length, 2);
assert.deepEqual(importedMany.errors, []);
assert.equal(importedMany.paths.length, 2);
assert.ok(importedMany.items.every(Number.isSafeInteger));
assert.notEqual(importedMany.items[0], importedMany.items[1]);

const saved = await bridge.saveProject("bridge-roundtrip.fcproj");
const projectBytes = bridge.readFile(saved.path);
assert.ok(projectBytes.length > 0);
const reopened = new FilmCraftBridge();
await reopened.openProject("bridge-roundtrip.fcproj", projectBytes);
assert.equal(reopened.inspect("project").name, "bridge-roundtrip");
assert.equal(reopened.files().length, 1);
assert.throws(() => reopened.readFile(saved.path), /outside this bridge/);
assert.equal(reopened.pumpJobs(20), false);

// One operation at a time: a call while an export is still pending rejects cleanly instead of
// interleaving. A 30-second matte guarantees the first 20 ms pump cannot finish the export.
const busyMatte = await bridge.execute("file.newColorMatte", { seconds: 30, color: "#345678" });
await bridge.execute("file.newSequence", { name: "Busy export", fromItem: busyMatte.item, width: 64, height: 36 });
await bridge.execute("timeline.place", { item: busyMatte.item, time: 0 });
const slowExport = bridge.exportMedia({ format: "h264", path: "busy.mp4", range: "custom", startSeconds: 0, endSeconds: 30, audio: false });
assert.throws(() => bridge.inspect("project"), /still running/);
await slowExport;
assert.equal(bridge.pumpJobs(20), false, "the busy flag is released when the operation settles");

console.log("JS bridge: CLI command parity, run, errors, limits, concurrency, render, batch import, media probes, export, and project round-trip passed");
