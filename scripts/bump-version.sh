#!/usr/bin/env bash
# Sync workspace version + GHCR image pins. Run before you commit a release.
#
#   ./scripts/bump-version.sh 0.5.0
#   ./scripts/bump-version.sh v0.5.0 --dry-run
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DRY_RUN=0
VERSION=""

usage() {
  sed -n '2,6p' "$0" | sed 's/^# \?//'
  echo "Usage: $0 [--dry-run] X.Y.Z"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    -*)
      echo "unknown flag: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$VERSION" ]]; then
        echo "unexpected argument: $1" >&2
        usage >&2
        exit 2
      fi
      VERSION="$1"
      shift
      ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  usage >&2
  exit 2
fi

VERSION="${VERSION#v}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "not a semver: $VERSION" >&2
  exit 2
fi

IMAGE_FILES=(
  README.md
  docker/compose.yaml
  docker/cloud-init.yaml
  render.yaml
)

python3 - "$ROOT" "$VERSION" "$DRY_RUN" "${IMAGE_FILES[@]}" <<'PY'
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
new = sys.argv[2]
dry = sys.argv[3] == "1"
image_files = sys.argv[4:]

cargo_toml = root / "Cargo.toml"
text = cargo_toml.read_text()
in_ws = False
old = None
out = []
for line in text.splitlines(keepends=True):
    stripped = line.strip()
    if stripped.startswith("[") and stripped.endswith("]"):
        in_ws = stripped == "[workspace.package]"
    if in_ws and stripped.startswith("version ="):
        m = re.match(r'version\s*=\s*"([^"]+)"', stripped)
        if not m:
            raise SystemExit("Cargo.toml [workspace.package] version is not a string")
        old = m.group(1)
        line = f'version = "{new}"\n'
    out.append(line)
if old is None:
    raise SystemExit("Cargo.toml has no [workspace.package] version")
new_toml = "".join(out)

lock = root / "Cargo.lock"
lock_text = lock.read_text()
lock_out = []
i = 0
lines = lock_text.splitlines(keepends=True)
changed_lock = 0
while i < len(lines):
    line = lines[i]
    lock_out.append(line)
    name_m = re.match(r'name = "(mcp-gateway-[^"]+)"\n', line)
    if name_m and i + 1 < len(lines):
        nxt = lines[i + 1]
        src = lines[i + 2] if i + 2 < len(lines) else ""
        if nxt.startswith("version = ") and not src.startswith("source = "):
            i += 1
            if nxt != f'version = "{new}"\n':
                changed_lock += 1
            lock_out.append(f'version = "{new}"\n')
    i += 1
new_lock = "".join(lock_out)

image_re = re.compile(r"ghcr\.io/fetch-hive/mcp-gateway:v?[0-9]+\.[0-9]+\.[0-9]+(?:[.-][0-9A-Za-z.-]+)?")
replacement = f"ghcr.io/fetch-hive/mcp-gateway:{new}"
image_updates = []
for rel in image_files:
    path = root / rel
    if not path.is_file():
        raise SystemExit(f"missing {rel}")
    body = path.read_text()
    if image_re.search(body) is None:
        raise SystemExit(f"{rel} has no ghcr.io/fetch-hive/mcp-gateway:X.Y.Z pin")
    updated = image_re.sub(replacement, body)
    image_updates.append((path, body, updated))

print(f"workspace {old} -> {new}")
print(f"Cargo.lock workspace crates rewritten: {changed_lock}")
for path, body, updated in image_updates:
    n = 0 if body == updated else body.count("ghcr.io/fetch-hive/mcp-gateway:")
    print(f"  {path.relative_to(root)}: {n} image pin(s)")

if dry:
    print("dry-run: no files written")
    raise SystemExit(0)

if new_toml != text:
    cargo_toml.write_text(new_toml)
if new_lock != lock_text:
    lock.write_text(new_lock)
for path, body, updated in image_updates:
    if updated != body:
        path.write_text(updated)
PY

if [[ "$DRY_RUN" -eq 1 ]]; then
  exit 0
fi

(cd "$ROOT" && cargo metadata --format-version 1 --locked --offline >/dev/null)

echo "bump-version: $VERSION"
echo "next: commit, push main, tag v$VERSION, publish ghcr.io/fetch-hive/mcp-gateway:$VERSION"
