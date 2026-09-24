import { execFileSync } from "node:child_process";
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import assert from "node:assert/strict";
import { TARGETS, assetName, packRelease, publishPacked, readWrapperPackage } from "./publish-npm.mjs";

test("packs a wrapper plus one binary package per release archive", () => {
  const assets = mkdtempSync(path.join(tmpdir(), "mcp-gateway-assets-"));
  const staging = path.join(assets, "staging");
  mkdirSync(staging, { recursive: true });
  try {
    for (const target of TARGETS) {
      const folder = path.join(staging, `mcp-gateway-cli-${target.triple}`);
      mkdirSync(folder, { recursive: true });
      const binary = path.join(folder, target.bin);
      writeFileSync(binary, `binary-${target.triple}`);
      chmodSync(binary, 0o755);
      const archive = path.join(assets, assetName(target));
      if (target.archive === "zip") {
        execFileSync("zip", ["-qr", archive, path.basename(folder)], { cwd: staging });
      } else {
        execFileSync("tar", ["-cJf", archive, "-C", staging, path.basename(folder)]);
      }
    }

    const out = mkdtempSync(path.join(tmpdir(), "mcp-gateway-npm-"));
    const version = readWrapperPackage().version;
    const packed = packRelease({ version, assetsDir: assets, outDir: out });
    const meta = JSON.parse(readFileSync(path.join(packed.metaDir, "package.json"), "utf8"));
    assert.equal(meta.name, "@fetch-hive/mcp-gateway");
    assert.equal(meta.version, version);
    assert.equal(meta.bin["mcp-gateway"], "bin/mcp-gateway");
    assert.equal(Object.keys(meta.optionalDependencies).length, TARGETS.length);
    for (const target of TARGETS) {
      assert.equal(meta.optionalDependencies[target.pkg], version);
      const dir = path.join(out, target.pkg.split("/").pop());
      const pkg = JSON.parse(readFileSync(path.join(dir, "package.json"), "utf8"));
      assert.equal(pkg.os[0], target.os);
      assert.equal(pkg.cpu[0], target.cpu);
      assert.equal(readFileSync(path.join(dir, target.bin), "utf8"), `binary-${target.triple}`);
    }
    const listed = execFileSync("npm", ["pack", "--dry-run", "--json"], {
      cwd: packed.metaDir,
      encoding: "utf8",
    });
    const files = JSON.parse(listed)[0].files.map((file) => file.path);
    assert.ok(files.includes("bin/mcp-gateway"));
  } finally {
    rmSync(assets, { recursive: true, force: true });
  }
});

test("skips npm publish when that version is already public", () => {
  const out = mkdtempSync(path.join(tmpdir(), "mcp-gateway-publish-"));
  const names = [...TARGETS.map((target) => target.pkg), "@fetch-hive/mcp-gateway"];
  try {
    for (const name of names) {
      const dir = path.join(out, name.split("/").pop());
      mkdirSync(dir, { recursive: true });
      writeFileSync(path.join(dir, "package.json"), JSON.stringify({ name, version: "0.6.0" }));
    }
    const seen = [];
    publishPacked(out, "0.6.0", {
      isPublished(name, version) {
        seen.push(`${name}@${version}`);
        return true;
      },
    });
    assert.deepEqual(seen.sort(), names.map((name) => `${name}@0.6.0`).sort());
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
});
