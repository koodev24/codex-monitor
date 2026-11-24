// Finalizes a draft GitHub release after every platform has uploaded:
// - renames Tauri's raw bundle assets to the public naming scheme
// - regenerates latest.json from the uploaded updater artifacts
// - keeps the draft unpublished; the workflow publishes it afterwards
import { releaseNotes } from "./release-notes.mjs";

const tag = process.env.TAG ?? process.argv[2] ?? "";
const repo = process.env.GITHUB_REPOSITORY ?? "";
const token = process.env.GITHUB_TOKEN ?? "";
const releaseId = process.env.RELEASE_ID ?? "";
const apiBase = process.env.GITHUB_API_URL ?? "https://api.github.com";
const uploadsBase =
  apiBase === "https://api.github.com"
    ? "https://uploads.github.com"
    : apiBase.replace(/\/api\/v3$/, "/api/uploads");

if (!tag || !repo || !token || !releaseId) {
  console.error("finalize-release: TAG, GITHUB_REPOSITORY, GITHUB_TOKEN, RELEASE_ID required");
  process.exit(1);
}

const version = tag.replace(/^refs\/tags\//, "").replace(/^[vV]/, "");
const downloadBase = `https://github.com/${repo}/releases/download/${tag}`;

async function api(path, init = {}) {
  const response = await fetch(`${apiBase}${path}`, {
    ...init,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
      ...init.headers,
    },
  });
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(`${init.method ?? "GET"} ${path} failed: ${response.status} ${detail}`);
  }
  return response.status === 204 ? null : response.json();
}

async function downloadSig(asset) {
  const headers = {
    Accept: "application/octet-stream",
    Authorization: `Bearer ${token}`,
    "X-GitHub-Api-Version": "2022-11-28",
  };
  const first = await fetch(asset.url, { headers, redirect: "manual" });
  const location = first.headers.get("location");
  if (location) {
    const second = await fetch(location, { headers: { Accept: "application/octet-stream" } });
    if (!second.ok) {
      throw new Error(`signature download failed: ${second.status} ${asset.name}`);
    }
    return (await second.text()).trim();
  }
  if (!first.ok) {
    throw new Error(`signature download failed: ${first.status} ${asset.name}`);
  }
  return (await first.text()).trim();
}

function canonicalName(raw) {
  const isSig = raw.endsWith(".sig");
  const name = isSig ? raw.slice(0, -4) : raw;
  let target = null;

  if (/^CodexMonitor_.+_aarch64\.dmg$/.test(name)) {
    target = `CodexMonitor-${version}-macos-arm64.dmg`;
  } else if (/^CodexMonitor_.+_(?:x64|x86_64)\.dmg$/.test(name)) {
    target = `CodexMonitor-${version}-macos-x64.dmg`;
  } else if (/^CodexMonitor_aarch64\.app\.tar\.gz$/.test(name)) {
    target = `CodexMonitor-${version}-macos-arm64.app.tar.gz`;
  } else if (/^CodexMonitor_(?:x64|x86_64)\.app\.tar\.gz$/.test(name)) {
    target = `CodexMonitor-${version}-macos-x64.app.tar.gz`;
  } else if (/^CodexMonitor_.+_(?:x64|x86_64)-setup\.exe$/.test(name)) {
    target = `CodexMonitor-${version}-windows-x64-setup.exe`;
  } else if (/^CodexMonitor_.+_(?:x64|x86_64)_[a-z]{2,3}(?:-[A-Za-z]{2,4})?\.msi$/.test(name)) {
    target = `CodexMonitor-${version}-windows-x64.msi`;
  } else if (/^CodexMonitor_.+_amd64\.AppImage$/.test(name)) {
    target = `CodexMonitor-${version}-linux-x64.AppImage`;
  } else if (/^CodexMonitor_.+_amd64\.deb$/.test(name)) {
    target = `CodexMonitor-${version}-linux-x64.deb`;
  } else if (/^CodexMonitor-.+-1\.x86_64\.rpm$/.test(name)) {
    target = `CodexMonitor-${version}-linux-x64.rpm`;
  }

  return target ? `${target}${isSig ? ".sig" : ""}` : null;
}

const platforms = {
  "darwin-aarch64": `CodexMonitor-${version}-macos-arm64.app.tar.gz`,
  "darwin-aarch64-app": `CodexMonitor-${version}-macos-arm64.app.tar.gz`,
  "darwin-x86_64": `CodexMonitor-${version}-macos-x64.app.tar.gz`,
  "darwin-x86_64-app": `CodexMonitor-${version}-macos-x64.app.tar.gz`,
  "windows-x86_64": `CodexMonitor-${version}-windows-x64-setup.exe`,
  "windows-x86_64-nsis": `CodexMonitor-${version}-windows-x64-setup.exe`,
  "windows-x86_64-msi": `CodexMonitor-${version}-windows-x64.msi`,
  "linux-x86_64": `CodexMonitor-${version}-linux-x64.AppImage`,
  "linux-x86_64-appimage": `CodexMonitor-${version}-linux-x64.AppImage`,
  "linux-x86_64-deb": `CodexMonitor-${version}-linux-x64.deb`,
  "linux-x86_64-rpm": `CodexMonitor-${version}-linux-x64.rpm`,
};

async function listAssets() {
  const assets = [];
  for (let page = 1; ; page += 1) {
    const chunk = await api(`/repos/${repo}/releases/${releaseId}/assets?per_page=100&page=${page}`);
    assets.push(...chunk);
    if (chunk.length < 100) return assets;
  }
}

async function deleteAsset(id) {
  await api(`/repos/${repo}/releases/assets/${id}`, { method: "DELETE" });
}

async function renameAsset(id, name) {
  await api(`/repos/${repo}/releases/assets/${id}`, {
    method: "PATCH",
    body: JSON.stringify({ name }),
    headers: { "Content-Type": "application/json" },
  });
}

async function main() {
  const release = await api(`/repos/${repo}/releases/${releaseId}`);
  if (!release.draft) {
    throw new Error(`finalize-release: ${tag} is already published; refusing to mutate it`);
  }

  const assets = await listAssets();
  const byName = new Map(assets.map((asset) => [asset.name, asset]));

  const renames = [];
  for (const asset of assets) {
    const target = canonicalName(asset.name);
    if (!target || target === asset.name) continue;
    renames.push({ asset, target });
  }

  for (const { asset, target } of renames) {
    const existing = byName.get(target);
    if (existing && existing.id !== asset.id) {
      await deleteAsset(existing.id);
    }
    await renameAsset(asset.id, target);
  }

  const renamed = await listAssets();
  const finalByName = new Map(renamed.map((asset) => [asset.name, asset]));

  const platformEntries = {};
  for (const [platform, assetName] of Object.entries(platforms)) {
    const asset = finalByName.get(assetName);
    if (!asset) throw new Error(`finalize-release: missing updater asset ${assetName}`);
    const sig = finalByName.get(`${assetName}.sig`);
    if (!sig) throw new Error(`finalize-release: missing signature for ${assetName}`);
    platformEntries[platform] = {
      signature: await downloadSig(sig),
      url: `${downloadBase}/${encodeURIComponent(assetName)}`,
    };
  }

  for (const installer of installers) {
    if (!finalByName.has(installer)) {
      throw new Error(`finalize-release: missing installer ${installer}`);
    }
  }

  const latest = {
    version,
    notes: releaseNotes(tag),
    pub_date: new Date().toISOString(),
    platforms: platformEntries,
  };

  const oldLatest = finalByName.get("latest.json");
  if (oldLatest) await deleteAsset(oldLatest.id);

  const uploadUrl = `${uploadsBase}/repos/${repo}/releases/${releaseId}/assets?name=latest.json`;
  const uploaded = await fetch(uploadUrl, {
    method: "POST",
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "X-GitHub-Api-Version": "2022-11-28",
    },
    body: JSON.stringify(latest, null, 2),
  });
  if (!uploaded.ok) {
    throw new Error(`latest.json upload failed: ${uploaded.status} ${await uploaded.text()}`);
  }

  // Remove bundle leftovers such as an unversioned compatibility zip or
  // arch-less updater archive. Only canonical assets stay public.
  const installers = [
    `CodexMonitor-${version}-macos-arm64.dmg`,
    `CodexMonitor-${version}-macos-x64.dmg`,
  ];
  const expected = new Set([
    "latest.json",
    ...installers,
    ...Object.values(platforms),
    ...Object.values(platforms).map((name) => `${name}.sig`),
  ]);
  const leftovers = (await listAssets()).filter(
    (asset) =>
      asset.name.startsWith("CodexMonitor") &&
      !expected.has(asset.name) &&
      !asset.name.endsWith("-SHA256SUMS.txt"),
  );
  for (const asset of leftovers) {
    await deleteAsset(asset.id);
    console.log(`finalize-release: removed stale asset ${asset.name}`);
  }

  console.log(`finalize-release: renamed ${renames.length} assets`);
  console.log(`finalize-release: latest.json lists ${Object.keys(platformEntries).length} platforms`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
