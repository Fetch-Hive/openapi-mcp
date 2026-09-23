#!/usr/bin/env node
// Pack (and optionally publish) @fetch-hive/mcp-gateway from GitHub Release
// archives. Platform packages hold the binary; the wrapper selects one.
//
//   node scripts/publish-npm.mjs --pack-only --version 0.6.0 --assets ./assets --out ./target/npm
//   node scripts/publish-npm.mjs --publish --version 0.6.0
//
// Publish uses NODE_AUTH_TOKEN (npm Automation token for @fetch-hive).
import { execFileSync } from "node:child_process";
import { chmodSync, cpSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export const TARGETS = [
  {
    triple: "x86_64-unknown-linux-musl",
    archive: "tar.xz",
    pkg: "@fetch-hive/mcp-gateway-linux-x64-musl",
    os: "linux",
    cpu: "x64",
    bin: "mcp-gateway",
  },
  {
    triple: "aarch64-unknown-linux-musl",
    archive: "tar.xz",
    pkg: "@fetch-hive/mcp-gateway-linux-arm64-musl",
    os: "linux",
    cpu: "arm64",
    bin: "mcp-gateway",
  },
  {
    triple: "x86_64-apple-darwin",
    archive: "tar.xz",
    pkg: "@fetch-hive/mcp-gateway-darwin-x64",
    os: "darwin",
    cpu: "x64",
    bin: "mcp-gateway",
  },
  {
    triple: "aarch64-apple-darwin",
    archive: "tar.xz",
    pkg: "@fetch-hive/mcp-gateway-darwin-arm64",
    os: "darwin",
    cpu: "arm64",
    bin: "mcp-gateway",
  },
  {
    triple: "x86_64-pc-windows-msvc",
    archive: "zip",
    pkg: "@fetch-hive/mcp-gateway-win32-x64-msvc",
    os: "win32",
    cpu: "x64",
    bin: "mcp-gateway.exe",
  },
];

export function assetName(target) {
  return `mcp-gateway-cli-${target.triple}.${target.archive}`;
}

export function readWrapperPackage() {
  return JSON.parse(readFileSync(path.join(root, "npm", "package.json"), "utf8"));
}

export function packRelease({ version, assetsDir, outDir }) {
  const wrapper = readWrapperPackage();
  if (wrapper.version !== version) {
    throw new Error(
      `npm/package.json is ${wrapper.version}, release is ${version}. Run scripts/bump-version.sh first.`,
    );
  }
  const expected = TARGETS.map((target) => target.pkg).sort();
  const actual = Object.keys(wrapper.optionalDependencies ?? {}).sort();
  if (expected.join() !== actual.join()) {
    throw new Error(`optionalDependencies must be ${expected.join(", ")}`);
  }

  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });

  const repo = "https://github.com/Fetch-Hive/openapi-mcp";
  for (const target of TARGETS) {
    const archive = path.join(assetsDir, assetName(target));
    const extracted = path.join(outDir, `_extract-${target.triple}`);
    mkdirSync(extracted, { recursive: true });
    if (target.archive === "zip") {
      execFileSync("unzip", ["-q", archive, "-d", extracted]);
    } else {
      execFileSync("tar", ["-xJf", archive, "-C", extracted]);
    }
    const binary = findBinary(extracted, target.bin);
    const dir = path.join(outDir, target.pkg.split("/").pop());
    mkdirSync(dir, { recursive: true });
    const dest = path.join(dir, target.bin);
    cpSync(binary, dest);
    chmodSync(dest, 0o755);
    writeJson(path.join(dir, "package.json"), {
      name: target.pkg,
      version,
      description: `mcp-gateway binary for ${target.os} ${target.cpu}`,
      license: "Apache-2.0",
      repository: { type: "git", url: `git+${repo}.git` },
      os: [target.os],
      cpu: [target.cpu],
      files: [target.bin],
      preferUnplugged: true,
      publishConfig: { access: "public" },
    });
    rmSync(extracted, { recursive: true, force: true });
  }

  const metaDir = path.join(outDir, "mcp-gateway");
  mkdirSync(path.join(metaDir, "bin"), { recursive: true });
  cpSync(path.join(root, "npm", "bin", "mcp-gateway"), path.join(metaDir, "bin", "mcp-gateway"));
  chmodSync(path.join(metaDir, "bin", "mcp-gateway"), 0o755);
  cpSync(path.join(root, "npm", "README.md"), path.join(metaDir, "README.md"));
  const optionalDependencies = {};
  for (const target of TARGETS) optionalDependencies[target.pkg] = version;
  writeJson(path.join(metaDir, "package.json"), {
    ...wrapper,
    version,
    optionalDependencies,
  });
  return { outDir, metaDir };
}

export function publishPacked(outDir, version) {
  const tag = version.includes("-") ? "next" : "latest";
  const provenance = process.env.ACTIONS_ID_TOKEN_REQUEST_URL ? ["--provenance"] : [];
  const dirs = TARGETS.map((target) => path.join(outDir, target.pkg.split("/").pop()));
  dirs.push(path.join(outDir, "mcp-gateway"));
  for (const dir of dirs) {
    execFileSync(
      "npm",
      ["publish", "--access", "public", "--tag", tag, ...provenance],
      { cwd: dir, stdio: "inherit" },
    );
  }
}

export function downloadReleaseAssets(version, dest) {
  mkdirSync(dest, { recursive: true });
  execFileSync(
    "gh",
    [
      "release",
      "download",
      `v${version}`,
      "--repo",
      "Fetch-Hive/openapi-mcp",
      "--pattern",
      "mcp-gateway-cli-*",
      "--dir",
      dest,
      "--clobber",
    ],
    { stdio: "inherit" },
  );
  return dest;
}

function findBinary(dir, name) {
  const hits = [];
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.name === name) hits.push(full);
    }
  };
  walk(dir);
  if (hits.length !== 1) {
    throw new Error(`expected one ${name} under ${dir}, found ${hits.length}`);
  }
  return hits[0];
}

function writeJson(file, value) {
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function parseArgs(argv) {
  const opts = { publish: false, packOnly: false, version: "", assets: "", out: "" };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--publish") opts.publish = true;
    else if (arg === "--pack-only") opts.packOnly = true;
    else if (arg === "--version") opts.version = argv[++i];
    else if (arg === "--assets") opts.assets = argv[++i];
    else if (arg === "--out") opts.out = argv[++i];
    else throw new Error(`unknown argument ${arg}`);
  }
  if (opts.publish === opts.packOnly) {
    throw new Error("pass exactly one of --publish or --pack-only");
  }
  if (!opts.version) throw new Error("--version X.Y.Z is required");
  opts.version = opts.version.replace(/^v/, "");
  return opts;
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const assets = opts.assets
    ? path.resolve(opts.assets)
    : downloadReleaseAssets(opts.version, path.join(tmpdir(), `mcp-gateway-npm-${opts.version}`));
  const out = opts.out ? path.resolve(opts.out) : path.join(root, "target", "npm");
  const packed = packRelease({ version: opts.version, assetsDir: assets, outDir: out });
  console.log(`packed ${packed.outDir}`);
  if (opts.publish) publishPacked(packed.outDir, opts.version);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
