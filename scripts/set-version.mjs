// Sets apps/desktop versions from a git tag like v2.0.3.
// Usage: node scripts/set-version.mjs v2.0.3
// Single source of truth for #10: tauri.conf.json + Cargo.toml stay in
// sync so the bundle, the updater manifest and the in-app version agree.
// Lockfiles carry the same version, so `npm ci` and `cargo build --locked`
// (if used) do not disagree with the bundle.
import { readFileSync, writeFileSync } from "node:fs";

const tag = process.argv[2] ?? "";
const version = tag.replace(/^refs\/tags\//, "").replace(/^[vV]/, "");
if (!/^\d+\.\d+\.\d+/.test(version)) {
  console.error(`set-version: refusing tag ${JSON.stringify(tag)}`);
  process.exit(1);
}

const confPath = new URL("../apps/desktop/src-tauri/tauri.conf.json", import.meta.url);
const conf = JSON.parse(readFileSync(confPath, "utf8"));
conf.version = version;
writeFileSync(confPath, `${JSON.stringify(conf, null, 2)}\n`);

const cargoPath = new URL("../apps/desktop/src-tauri/Cargo.toml", import.meta.url);
const cargo = readFileSync(cargoPath, "utf8").replace(
  /^(version = ")[^"]+(")/m,
  `$1${version}$2`,
);
writeFileSync(cargoPath, cargo);

const pkgPath = new URL("../apps/desktop/package.json", import.meta.url);
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
pkg.version = version;
writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`);

const lockPath = new URL("../apps/desktop/package-lock.json", import.meta.url);
const lock = JSON.parse(readFileSync(lockPath, "utf8"));
lock.version = version;
if (lock.packages?.[""]) lock.packages[""].version = version;
writeFileSync(lockPath, `${JSON.stringify(lock, null, 2)}\n`);

const cargoLockPath = new URL("../apps/desktop/src-tauri/Cargo.lock", import.meta.url);
const cargoLock = readFileSync(cargoLockPath, "utf8").replace(
  /(name = "appsdesktop"\nversion = ")[^"]+(")/,
  `$1${version}$2`,
);
writeFileSync(cargoLockPath, cargoLock);

console.log(`set-version: ${version}`);
