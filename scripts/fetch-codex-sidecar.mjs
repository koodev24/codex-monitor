// Stages the Codex CLI sidecar binary for the Tauri build.
// Downloads @openai/codex@<version>-<platform>-<arch> from npm, extracts
// package/vendor/<rust-triple>/bin/codex, and places it at
// apps/desktop/src-tauri/binaries/codex-<rust-triple>[.exe] so
// tauri.conf.json > bundle > externalBin picks it up.
//
// Version: CODEX_SIDECAR_VERSION env, else latest published @openai/codex.
// Idempotent: skips when the staged binary already exists (pass --force to
// re-download). Run automatically via beforeBuildCommand; safe in CI.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync, copyFileSync, chmodSync } from "node:fs";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const binariesDir = join(root, "apps", "desktop", "src-tauri", "binaries");

const TRIPLES = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
  "win32-arm64": "aarch64-pc-windows-msvc",
};

function run(cmd, args, opts = {}) {
  return execFileSync(cmd, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...opts }).trim();
}

const hostKey = `${process.platform}-${process.arch}`;
const triple = TRIPLES[hostKey];
if (!triple) {
  console.error(`fetch-codex-sidecar: unsupported host ${hostKey}`);
  process.exit(1);
}
const exe = process.platform === "win32" ? ".exe" : "";
const target = join(binariesDir, `codex-${triple}${exe}`);
const force = process.argv.includes("--force");

if (existsSync(target) && !force) {
  console.log(`fetch-codex-sidecar: already staged ${target}`);
  process.exit(0);
}

const version =
  process.env.CODEX_SIDECAR_VERSION || run("npm", ["view", "@openai/codex", "version"]);
const spec = `@openai/codex@${version}-${hostKey}`;
console.log(`fetch-codex-sidecar: fetching ${spec}`);

const tmp = mkdtempSync(join(tmpdir(), "codex-sidecar-"));
try {
  const tgz = run("npm", ["pack", spec, "--pack-destination", tmp]).split("\n").pop();
  run("tar", ["-xzf", join(tmp, tgz), "-C", tmp, `package/vendor/${triple}/bin/codex${exe}`]);
  mkdirSync(binariesDir, { recursive: true });
  copyFileSync(join(tmp, `package/vendor/${triple}/bin/codex${exe}`), target);
  if (exe === "") chmodSync(target, 0o755);
  console.log(`fetch-codex-sidecar: staged ${target}`);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
