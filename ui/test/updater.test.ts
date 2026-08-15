"use strict";

import assert = require("node:assert/strict");
import fs = require("node:fs");
import path = require("node:path");
import test = require("node:test");
import vm = require("node:vm");

const repository = path.resolve(process.cwd());

function read(relativePath: string): string {
  return fs.readFileSync(path.join(repository, relativePath), "utf8");
}

function readJson(relativePath: string): unknown {
  return JSON.parse(read(relativePath));
}

function appFunction(source: string, name: string): string {
  const match = source.match(
    new RegExp(
      `^(?<indent>[ \\t]+)function ${name}\\([^\\n]*\\) \\{[\\s\\S]*?^\\k<indent>\\}`,
      "m",
    ),
  );
  assert.ok(match, `${name} should be independently testable`);
  return match[0];
}

test("the app exposes an accessible, dismissible update banner", () => {
  const html = read("ui/index.html");
  const banner = html.match(
    /<section\b(?=[^>]*id="updateBanner")[^>]*>[\s\S]*?<\/section>/i,
  )?.[0];
  assert.ok(banner, "the update banner should exist");
  assert.match(banner, /\bhidden\b/i);
  assert.match(banner, /\brole="status"/i);
  assert.match(banner, /\baria-live="polite"/i);
  assert.match(banner, /id="updateVersion"/i);
  assert.match(banner, /<progress\b(?=[^>]*id="updateProgress")/i);
  assert.match(banner, /<button\b(?=[^>]*id="updateNowButton")[^>]*>\s*Update now\s*<\/button>/i);
  assert.match(banner, /<button\b(?=[^>]*id="updateLaterButton")[^>]*>\s*Later\s*<\/button>/i);

  const settings = html.match(
    /<dialog\b(?=[^>]*id="settingsDialog")[^>]*>[\s\S]*?<\/dialog>/i,
  )?.[0];
  assert.ok(settings, "the existing settings dialog should contain update controls");
  assert.match(settings, />\s*Application updates\s*</i);
  assert.match(settings, /id="updateCheckStatus"[^>]*role="status"/i);
  assert.match(settings, /id="checkForUpdatesButton"[^>]*>[\s\S]*?Check for updates/i);
});

test("the updater uses Tauri's updater and process plugins without blocking startup", () => {
  const javascript = read("ui/generated/app.js");
  assert.match(javascript, /tauriApi\?\.updater/);
  assert.match(javascript, /tauriApi\?\.process/);
  assert.match(javascript, /tauriUpdater\.check\(\{ timeout: UPDATE_CHECK_TIMEOUT_MS \}\)/);
  assert.match(javascript, /invoke\("is_release_build", \{\}\)/);
  assert.match(javascript, /downloadAndInstall\(handleUpdateDownloadEvent/);
  assert.match(javascript, /tauriProcess\.relaunch\(\)/);
  assert.match(javascript, /event\.event === "Started"/);
  assert.match(javascript, /event\.event === "Progress"/);
  assert.match(javascript, /event\.event === "Finished"/);
  assert.match(
    javascript,
    /window\.setTimeout\(\(\) => \{\s*void checkForUpdates\(false\);\s*\}, UPDATE_CHECK_DELAY_MS\)/,
  );
  assert.doesNotMatch(
    javascript,
    /setInterval\([^)]*checkForUpdates/,
    "failed checks should not be retried on an interval",
  );
});

test("update progress formatting is bounded and handles unknown totals", () => {
  const javascript = read("ui/generated/app.js");
  const context = {} as {
    formatUpdateVersion(version: string): string;
    formatByteCount(bytes: number): string;
    updateProgressPercent(downloaded: number, contentLength: number | null): number | null;
  };
  vm.runInNewContext(
    ["formatUpdateVersion", "formatByteCount", "updateProgressPercent"]
      .map((name) => appFunction(javascript, name))
      .join("\n"),
    context,
  );

  assert.equal(context.formatUpdateVersion("1.2.3"), "v1.2.3");
  assert.equal(context.formatUpdateVersion("v1.2.3"), "v1.2.3");
  assert.equal(context.formatByteCount(1_572_864), "1.5 MB");
  assert.equal(context.updateProgressPercent(50, null), null);
  assert.equal(context.updateProgressPercent(50, 0), null);
  assert.equal(context.updateProgressPercent(25, 100), 25);
  assert.equal(context.updateProgressPercent(125, 100), 100);
});

test("Tauri updater configuration targets the GitHub release manifest", () => {
  const config = readJson("src-tauri/tauri.conf.json") as {
    plugins?: { updater?: { endpoints?: string[]; pubkey?: string } };
  };
  assert.deepEqual(config.plugins?.updater?.endpoints, [
    "https://github.com/fromfactory/vsparallel/releases/latest/download/latest.json",
  ]);
  assert.equal(
    typeof config.plugins?.updater?.pubkey,
    "string",
    "the plugin requires a public-key field even before release signing is configured",
  );

  const cargo = read("src-tauri/Cargo.toml");
  const library = read("src-tauri/src/lib.rs");
  assert.match(cargo, /tauri-plugin-updater\s*=\s*"2\.10\.1"/);
  assert.match(cargo, /tauri-plugin-process\s*=\s*"2\.3\.1"/);
  assert.match(library, /tauri_plugin_updater::Builder::new\(\)\.build\(\)/);
  assert.match(library, /tauri_plugin_process::init\(\)/);
  assert.match(library, /fn is_release_build\(\) -> bool \{\s*!cfg!\(debug_assertions\)/);
});

test("release builds opt into signed updater artifacts and prepare a gated draft", () => {
  const config = readJson("src-tauri/tauri.conf.json") as {
    bundle?: { targets?: string[] };
  };
  const releaseConfig = readJson("src-tauri/tauri.release.conf.json") as {
    bundle?: { createUpdaterArtifacts?: boolean };
  };
  assert.deepEqual(config.bundle?.targets, ["deb"]);
  assert.equal(releaseConfig.bundle?.createUpdaterArtifacts, true);

  const workflow = read(".github/workflows/release.yml");
  assert.match(workflow, /TAURI_SIGNING_PRIVATE_KEY:\s*\$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/);
  assert.match(workflow, /--config src-tauri\/tauri\.release\.conf\.json/);
  assert.match(workflow, /does not match the public key/);
  assert.match(workflow, /create-update-manifest\.py/);
  assert.match(workflow, /\.app\.tar\.gz\.sig/);
  assert.match(workflow, /latest\.json/);
  assert.match(workflow, /--draft/);
  assert.match(workflow, /gh release delete-asset/);
  assert.match(workflow, /Draft release assets do not exactly match/);
  assert.doesNotMatch(workflow, /--draft=false/);
  assert.doesNotMatch(workflow, /AppImage/);
});

test("the public Linux download resolves only the release-supported Debian package", () => {
  const html = read("website/index.html");
  const downloads = read("website/src/downloads.ts");

  assert.match(html, /data-download-kind="linux-deb"/);
  assert.match(html, /Ubuntu and Debian \(\.deb\)/);
  assert.doesNotMatch(html, /AppImage/i);
  assert.match(downloads, /linux: \["linux-deb"\]/);
  assert.doesNotMatch(downloads, /appimage/i);
});
