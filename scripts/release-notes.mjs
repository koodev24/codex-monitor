// Prints the CHANGELOG section for a release tag, e.g. `node scripts/release-notes.mjs v2.6.0`.
// The generated text becomes the GitHub release body and the updater notes.
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const changelogPath = new URL("../CHANGELOG.md", import.meta.url);

export function releaseNotes(tag) {
  const version = tag.replace(/^refs\/tags\//, "").replace(/^[vV]/, "");
  if (!version) throw new Error("release-notes: missing tag");
  const changelog = readFileSync(changelogPath, "utf8");
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const heading = new RegExp(`^## (?:v)?${escaped}(?:\\s+-[^\\n]*)?\\s*$`, "m");
  const match = heading.exec(changelog);
  if (!match) throw new Error(`release-notes: no CHANGELOG section for ${tag}`);
  const start = match.index + match[0].length;
  const rest = changelog.slice(start);
  const next = rest.search(/^## /m);
  const body = (next === -1 ? rest : rest.slice(0, next)).trim();
  if (!body) throw new Error(`release-notes: empty CHANGELOG section for ${tag}`);
  return body;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const tag = process.argv[2];
  if (!tag) {
    console.error("usage: node scripts/release-notes.mjs <tag>");
    process.exit(1);
  }
  try {
    process.stdout.write(`${releaseNotes(tag)}\n`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
