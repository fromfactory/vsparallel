"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repository = path.resolve(__dirname, "../..");

function read(relativePath) {
  return fs.readFileSync(path.join(repository, relativePath), "utf8");
}

function readBuffer(relativePath) {
  return fs.readFileSync(path.join(repository, relativePath));
}

function pngMetadata(relativePath) {
  const png = readBuffer(relativePath);
  assert.deepEqual(
    png.subarray(0, 8),
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    `${relativePath} should be a PNG`,
  );
  assert.equal(png.subarray(12, 16).toString("ascii"), "IHDR");
  return {
    width: png.readUInt32BE(16),
    height: png.readUInt32BE(20),
    colorType: png[25],
  };
}

function sliceBetween(source, startPattern, endPattern, description) {
  const startMatch = source.match(startPattern);
  assert.ok(startMatch, `${description} should have a start marker`);
  const start = startMatch.index;
  const remainder = source.slice(start + startMatch[0].length);
  const endMatch = remainder.match(endPattern);
  assert.ok(endMatch, `${description} should have an end marker`);
  return source.slice(start, start + startMatch[0].length + endMatch.index);
}

function attribute(tag, name) {
  const match = tag.match(new RegExp(`\\b${name}\\s*=\\s*(["'])(.*?)\\1`, "i"));
  return match?.[2] ?? null;
}

function cssBlocksMatching(css, pattern) {
  return Array.from(css.matchAll(/([^{}]+)\{([^{}]*)\}/g))
    .filter((match) => pattern.test(match[1]))
    .map((match) => match[0])
    .join("\n");
}

test("the main chrome omits redundant labels while retaining an accessible workspace heading", () => {
  const html = read("ui/index.html");
  const titlebar = html.match(/<header\b(?=[^>]*id="appTitlebar")[\s\S]*?<\/header>/i)?.[0];
  assert.ok(titlebar, "the app titlebar should exist");
  assert.doesNotMatch(titlebar, />\s*Workspace monitor\s*</i);
  assert.doesNotMatch(html, />\s*VS Code workspaces\s*</i);

  assert.match(
    html,
    /<h2\b(?=[^>]*id="workspaceHeading")(?=[^>]*class="[^"]*\bsr-only\b[^"]*")[^>]*>\s*Workspaces\s*<\/h2>/i,
  );

  const connectionBar = html.match(
    /<div\b(?=[^>]*id="connectionBar")[^>]*>[\s\S]*?<\/div>/i,
  )?.[0];
  assert.ok(connectionBar, "the connection bar should exist");
  assert.match(connectionBar, /\bid="workspaceCount"/i);
  assert.equal(
    Array.from(html.matchAll(/\bid="workspaceCount"/gi)).length,
    1,
    "the workspace count should have one canonical location",
  );
});

test("the interface uses the reference icon on every in-app brand surface", () => {
  const html = read("ui/index.html");
  assert.match(html, /<link\b(?=[^>]*rel="icon")(?=[^>]*href="vsparallel-icon\.png")[^>]*>/i);
  assert.match(
    html,
    /<img\b(?=[^>]*class="brand-mark")(?=[^>]*src="vsparallel-icon\.png")(?=[^>]*alt="")[^>]*>/i,
  );
  assert.match(
    html,
    /<img\b(?=[^>]*class="launch-overlay__mark")(?=[^>]*src="vsparallel-icon\.png")(?=[^>]*alt="")[^>]*>/i,
  );
});

test("the primary palette is VS Code blue in dark and light themes", () => {
  const css = read("ui/styles.css");
  assert.match(css, /:root\s*\{[\s\S]*?--accent\s*:\s*#3794ff\s*;/i);
  assert.match(
    css,
    /:root\[data-color-theme="light"\]\s*\{[\s\S]*?--accent\s*:\s*#0078d4\s*;/i,
  );
  assert.doesNotMatch(
    css,
    /#(?:a78bfa|7052c8|7559c4|33244f|866ce0|5e43af)|rgba\(\s*(?:167\s*,\s*139\s*,\s*250|112\s*,\s*82\s*,\s*200)/i,
  );
});

test("provider names stay complete and the status panel owns most row width", () => {
  const javascript = read("ui/app.js");
  const css = read("ui/styles.css");
  assert.match(
    javascript,
    /createProviderState\(\s*"Claude"\s*,\s*workspace\.claude\s*,\s*"Claude Code"\s*\)/,
  );
  assert.doesNotMatch(javascript, /createProviderState\(\s*"Claude Code"\s*,/);
  assert.match(
    css,
    /\.workspace-row\s*\{[^}]*grid-template-columns\s*:\s*28px\s+minmax\(160px,\s*0\.7fr\)\s+minmax\(300px,\s*1\.3fr\)/i,
  );
  assert.match(
    css,
    /\.provider-state\s*\{[^}]*grid-template-columns\s*:\s*48px\s+minmax\(0,\s*1fr\)/i,
  );
  const providerName = css.match(/\.provider-name\s*\{([^}]*)\}/i)?.[1];
  assert.ok(providerName, "provider names should have dedicated styling");
  assert.doesNotMatch(providerName, /text-overflow\s*:\s*ellipsis/i);
});

test("platform, tray, UI, and companion icon assets use the complete size set", () => {
  const expectedPngs = new Map([
    ["src-tauri/icons/16x16.png", 16],
    ["src-tauri/icons/24x24.png", 24],
    ["src-tauri/icons/32x32.png", 32],
    ["src-tauri/icons/48x48.png", 48],
    ["src-tauri/icons/64x64.png", 64],
    ["src-tauri/icons/128x128.png", 128],
    ["src-tauri/icons/128x128@2x.png", 256],
    ["src-tauri/icons/256x256.png", 256],
    ["src-tauri/icons/512x512.png", 512],
    ["src-tauri/icons/icon.png", 512],
    ["src-tauri/icons/tray-icon-linux.png", 64],
    ["src-tauri/icons/tray-icon-macos.png", 36],
    ["src-tauri/icons/tray-icon-windows.png", 64],
    ["ui/vsparallel-icon.png", 128],
    ["companion/icon.png", 128],
  ]);
  for (const [relativePath, expectedSize] of expectedPngs) {
    const metadata = pngMetadata(relativePath);
    assert.deepEqual([metadata.width, metadata.height], [expectedSize, expectedSize]);
    assert.equal(metadata.colorType, 6, `${relativePath} should be RGBA`);
  }

  const ico = readBuffer("src-tauri/icons/icon.ico");
  assert.deepEqual(Array.from(ico.subarray(0, 4)), [0, 0, 1, 0]);
  const icoCount = ico.readUInt16LE(4);
  const icoSizes = Array.from({ length: icoCount }, (_, index) => {
    const encodedSize = ico[6 + index * 16];
    return encodedSize === 0 ? 256 : encodedSize;
  });
  assert.equal(icoSizes[0], 32, "Tauri should decode a 32 px Windows runtime icon first");
  assert.deepEqual(
    new Set(icoSizes),
    new Set([32, 16, 20, 24, 30, 36, 40, 48, 60, 64, 72, 80, 96, 128, 256]),
  );

  const icns = readBuffer("src-tauri/icons/icon.icns");
  assert.equal(icns.subarray(0, 4).toString("ascii"), "icns");
  assert.equal(icns.readUInt32BE(4), icns.length, "the ICNS header should cover the full file");
  const icnsTypes = [];
  for (let offset = 8; offset < icns.length; ) {
    const chunkSize = icns.readUInt32BE(offset + 4);
    assert.ok(chunkSize >= 8, "each ICNS chunk should include its header");
    icnsTypes.push(icns.subarray(offset, offset + 4).toString("ascii"));
    offset += chunkSize;
  }
  for (const type of [
    "is32",
    "s8mk",
    "ic11",
    "il32",
    "l8mk",
    "ic12",
    "ic07",
    "ic13",
    "ic08",
    "ic14",
    "ic09",
    "ic10",
  ]) {
    assert.ok(icnsTypes.includes(type), `the macOS icon should include ${type}`);
  }

  const configuredIcons = JSON.parse(read("src-tauri/tauri.conf.json")).bundle.icon;
  assert.equal(
    configuredIcons[0],
    "icons/128x128.png",
    "GTK/X11 should embed a crisp taskbar icon small enough to publish as _NET_WM_ICON",
  );
  for (const icon of [
    "icons/icon.png",
    "icons/16x16.png",
    "icons/24x24.png",
    "icons/48x48.png",
    "icons/256x256.png",
    "icons/icon.icns",
    "icons/icon.ico",
  ]) {
    assert.ok(configuredIcons.includes(icon), `${icon} should be included in Tauri bundles`);
  }

  const macosBundle = JSON.parse(read("src-tauri/tauri.macos.conf.json")).bundle;
  assert.deepEqual(macosBundle.icon, ["icons/icon.icns"]);
  assert.deepEqual(macosBundle.targets, ["app", "dmg"]);
  assert.equal(macosBundle.macOS.minimumSystemVersion, "12.3");
  const windowsBundle = JSON.parse(read("src-tauri/tauri.windows.conf.json")).bundle;
  assert.deepEqual(windowsBundle.icon, ["icons/icon.ico"]);
  assert.deepEqual(windowsBundle.targets, ["nsis"]);
  assert.equal(windowsBundle.windows.allowDowngrades, false);

  const traySource = read("src-tauri/src/tray.rs");
  assert.doesNotMatch(traySource, /include_image!/);
  assert.doesNotMatch(traySource, /TRAY_ICON_REAPPLY_DELAY|tray\.set_icon\s*\(/);
  assert.match(traySource, /const TRAY_ID:\s*&str\s*=\s*"vsparallel-tray"/);
  assert.match(traySource, /TrayIconTempDirectory::create\(icon_variant\)/);
  assert.match(traySource, /\.permissions\(Permissions::from_mode\(0o700\)\)/);
  assert.match(traySource, /\.temp_dir_path\(temp_path\)/);
  for (const name of ["linux", "macos", "windows"]) {
    assert.match(traySource, new RegExp(`include_bytes!\\(\"\\.\\./icons/tray-icon-${name}\\.png\"\\)`));
  }
  const buildScript = read("src-tauri/build.rs");
  for (const name of ["linux", "macos", "windows"]) {
    assert.match(buildScript, new RegExp(`tray-icon-${name}\\.png`));
  }
  const desktopInstaller = read("scripts/install-desktop.sh");
  const desktopUninstaller = read("scripts/uninstall-desktop.sh");
  const desktopEntry = read("packaging/app.vsparallel.desktop.in");
  assert.match(desktopEntry, /^Icon=app\.vsparallel$/m);
  assert.match(desktopEntry, /^StartupWMClass=Vsparallel$/m);
  assert.match(desktopInstaller, /gtk-update-icon-cache\s+-f\s+-t/);
  assert.match(desktopInstaller, /update-desktop-database/);
  assert.match(desktopUninstaller, /gtk-update-icon-cache\s+-f\s+-t/);
  for (const size of [16, 24, 32, 48, 64, 128, 256, 512]) {
    assert.match(desktopInstaller, new RegExp(`hicolor/${size}x${size}/apps`));
    assert.match(desktopUninstaller, new RegExp(`hicolor/${size}x${size}/apps`));
  }
  assert.ok(!fs.existsSync(path.join(repository, "src-tauri/icons/tray-icon.png")));
  assert.ok(!fs.existsSync(path.join(repository, "src-tauri/icons/tray-icon-template.png")));
});

test("setup and diagnostics uses the compact neutral visual language of the main screen", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");

  assert.doesNotMatch(html, /settings-dialog__title-icon/);
  assert.doesNotMatch(html, /monitor-diagnostics__mark/);
  assert.match(
    css,
    /\.settings-dialog__header\s*\{[^}]*min-height:\s*52px[^}]*background:\s*var\(--panel\)[^}]*box-shadow:\s*none/s,
  );
  assert.match(
    css,
    /\.appearance-options\s*\{[^}]*grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)[^}]*background:\s*var\(--panel-deep\)/s,
  );
  assert.match(
    css,
    /\.privacy-note\s*\{[^}]*border:\s*0[^}]*background:\s*transparent/s,
  );
  assert.match(
    css,
    /\.integration-list\s*\{[^}]*grid-template-columns:\s*1fr[^}]*background:\s*var\(--panel-deep\)/s,
  );
  assert.match(
    css,
    /\.integration-card\s*\{[^}]*border:\s*0[^}]*background:\s*transparent[^}]*box-shadow:\s*none/s,
  );
  assert.match(
    css,
    /\.integration-status::before\s*\{[^}]*width:\s*6px[^}]*border-radius:\s*50%/s,
  );
  assert.match(
    css,
    /\.monitor-diagnostics\s*\{[^}]*border-top:\s*1px solid var\(--border\)[^}]*background:\s*transparent[^}]*box-shadow:\s*none/s,
  );
});

test("workspace rows use a transparent native full-card action without visible Open text", () => {
  const javascript = read("ui/app.js");
  const css = read("ui/styles.css");
  const createRow = sliceBetween(
    javascript,
    /function\s+createWorkspaceRow\s*\(/,
    /function\s+renderSnapshot\s*\(/,
    "createWorkspaceRow",
  );

  assert.match(
    createRow,
    /createElement\(\s*["']button["']\s*,\s*["']open-button["'](?:\s*,\s*(?:undefined|["']["']))?\s*\)/,
  );
  assert.doesNotMatch(
    createRow,
    /createElement\(\s*["']button["']\s*,\s*["']open-button["']\s*,\s*["']Open["']\s*\)/,
  );
  assert.doesNotMatch(createRow, /\.textContent\s*=\s*["']Open["']/);
  assert.match(createRow, /setAttribute\(\s*["']aria-label["']/);

  const openButtonCss = cssBlocksMatching(css, /\.open-button\b/);
  assert.match(openButtonCss, /position\s*:\s*absolute/i);
  assert.ok(
    /inset\s*:\s*0(?:\s+0){0,3}\s*;/i.test(openButtonCss) ||
      (/width\s*:\s*100%/i.test(openButtonCss) && /height\s*:\s*100%/i.test(openButtonCss)),
    "the native action should cover the complete workspace row",
  );
  assert.ok(
    /background(?:-color)?\s*:\s*transparent/i.test(openButtonCss) ||
      /opacity\s*:\s*0(?:\.0+)?\s*;/i.test(openButtonCss),
    "the full-card action should not render a separate button surface",
  );
});

test("appearance offers persisted System, Light, and Dark choices", () => {
  const html = read("ui/index.html");
  const javascript = `${read("ui/theme-init.js")}\n${read("ui/app.js")}`;
  const css = read("ui/styles.css");
  const inputs = Array.from(html.matchAll(/<input\b[^>]*>/gi), (match) => match[0]);
  const radios = inputs.filter((input) => attribute(input, "type")?.toLowerCase() === "radio");
  const appearanceRadios = radios.filter((input) =>
    ["system", "light", "dark"].includes(attribute(input, "value")?.toLowerCase()),
  );

  assert.deepEqual(
    new Set(appearanceRadios.map((input) => attribute(input, "value").toLowerCase())),
    new Set(["system", "light", "dark"]),
  );
  assert.equal(
    new Set(appearanceRadios.map((input) => attribute(input, "name"))).size,
    1,
    "appearance choices should form one radio group",
  );
  assert.ok(
    appearanceRadios.every((input) => attribute(input, "name")),
    "each appearance choice should name its radio group",
  );
  const system = appearanceRadios.find(
    (input) => attribute(input, "value")?.toLowerCase() === "system",
  );
  assert.match(system, /\schecked(?:\s|=|\/?>)/i);

  const themeScript = html.search(/<script\b[^>]*src=["']theme-init\.js["'][^>]*>/i);
  const stylesheet = html.search(/<link\b[^>]*href=["']styles\.css["'][^>]*>/i);
  assert.ok(themeScript >= 0, "the early theme initializer should be loaded");
  assert.ok(stylesheet >= 0 && themeScript < stylesheet, "theme initialization should precede CSS");
  assert.match(javascript, /localStorage\.getItem\s*\(/);
  assert.match(javascript, /localStorage\.setItem\s*\(/);
  assert.match(javascript, /prefers-color-scheme\s*:\s*light/);
  assert.match(javascript, /["']system["']/);
  assert.ok(
    /\[data-(?:color-)?theme\s*=\s*["']light["']\][^{]*\{/i.test(css),
    "CSS should expose a resolved, explicit light-theme palette",
  );
});

test("workspace launching exposes live glass feedback", () => {
  const html = read("ui/index.html");
  const javascript = read("ui/app.js");
  const css = read("ui/styles.css");
  const tags = Array.from(html.matchAll(/<[^!/][^>]*>/g), (match) => match[0]);
  const launchTags = tags.filter((tag) =>
    /(?:id|class)=["'][^"']*(?:launch|opening)[^"']*["']/i.test(tag),
  );

  assert.ok(launchTags.length > 0, "launch feedback should have dedicated markup");
  assert.ok(
    launchTags.some(
      (tag) => /aria-live=["']polite["']/i.test(tag) || /role=["']status["']/i.test(tag),
    ),
    "launch feedback should be announced without interrupting the user",
  );
  assert.match(javascript, /openingInstanceId/);
  assert.match(javascript, /(?:launch(?:Overlay|Status)|workspaceOpening)/i);

  const launchCss = cssBlocksMatching(css, /(?:launch|opening)/i);
  assert.match(launchCss, /backdrop-filter\s*:/i);
  assert.ok(
    /rgba?\s*\(|hsla?\s*\(|color-mix\s*\(|opacity\s*:\s*0?\.[0-9]|background(?:-color)?\s*:\s*var\(/i.test(
      launchCss,
    ),
    "launch feedback should use a translucent glass treatment",
  );
});

test("workspace launches never auto-minimize VSParallel", () => {
  const library = read("src-tauri/src/lib.rs");
  const tray = read("src-tauri/src/tray.rs");
  const openWorkspace = sliceBetween(
    library,
    /fn\s+open_workspace\s*\(/,
    /fn\s+activate_tray_workspace\s*\(/,
    "open_workspace",
  );
  const hideWindow = sliceBetween(
    library,
    /fn\s+hide_window\s*\(/,
    /fn\s+show_main_window\s*\(/,
    "hide_window",
  );
  const menuHandler = sliceBetween(
    tray,
    /fn\s+handle_menu_event\s*\(/,
    /fn\s+resolve_menu_action\s*\(/,
    "tray menu handler",
  );
  const trayWorkspaceStart = menuHandler.indexOf("TrayMenuAction::OpenWorkspace");
  const trayWorkspaceEnd = menuHandler.indexOf("TrayMenuAction::None", trayWorkspaceStart);

  assert.ok(trayWorkspaceStart >= 0 && trayWorkspaceEnd > trayWorkspaceStart);
  const trayWorkspace = menuHandler.slice(trayWorkspaceStart, trayWorkspaceEnd);
  assert.match(openWorkspace, /open_with\s*\(/);
  assert.doesNotMatch(openWorkspace, /\.minimize\s*\(/);
  assert.doesNotMatch(trayWorkspace, /\.minimize\s*\(/);
  assert.match(hideWindow, /\.minimize\s*\(/);
});
