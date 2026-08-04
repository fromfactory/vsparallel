"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repository = path.resolve(__dirname, "../..");

function read(relativePath) {
  return fs.readFileSync(path.join(repository, relativePath), "utf8");
}

function readJson(relativePath) {
  return JSON.parse(read(relativePath));
}

test("desktop platforms use custom chrome while macOS retains native chrome", () => {
  const desktop = readJson("src-tauri/tauri.conf.json").app.windows[0];
  const macos = readJson("src-tauri/tauri.macos.conf.json").app.windows[0];

  assert.equal(desktop.decorations, false);
  assert.equal(desktop.resizable, true);
  assert.equal(desktop.theme, undefined);
  assert.equal(macos.decorations, true);
  assert.equal(macos.titleBarStyle, "Transparent");
  assert.equal(macos.hiddenTitle, true);
  assert.equal(macos.resizable, true);
  assert.deepEqual(
    [macos.width, macos.height, macos.minWidth, macos.minHeight],
    [desktop.width, desktop.height, desktop.minWidth, desktop.minHeight],
  );
});

test("the capability grants only the supported drag operation to the frontend", () => {
  const capability = readJson("src-tauri/capabilities/default.json");
  assert.deepEqual(capability.permissions, ["core:window:allow-start-dragging"]);
});

test("the production frontend contains only explicit runtime assets", () => {
  const config = readJson("src-tauri/tauri.conf.json");
  assert.deepEqual(config.build.frontendDist, [
    "../ui/index.html",
    "../ui/styles.css",
    "../ui/app.js",
    "../ui/theme-init.js",
    "../ui/vsparallel-icon.png",
  ]);
  assert.equal(config.build.devUrl, undefined);
  assert.match(config.app.security.csp, /base-uri 'none'/);
  assert.match(config.app.security.csp, /object-src 'none'/);
  assert.deepEqual(config.bundle.resources, {
    "../LICENSE": "LICENSE",
    "../PRIVACY.md": "PRIVACY.md",
  });
});

test("titlebar controls are accessible, initially guarded, and excluded from dragging", () => {
  const html = read("ui/index.html");
  const titlebar = html.match(/<header[\s\S]*?id="appTitlebar"[\s\S]*?<\/header>/)?.[0];
  assert.ok(titlebar, "the integrated titlebar should exist");
  assert.match(titlebar, /id="titlebarDragRegion"/);
  assert.match(titlebar, /id="windowControls"[^>]*aria-label="Window controls"[^>]*hidden/);

  for (const id of ["refreshButton", "settingsButton", "hideButton", "maximizeButton", "closeButton"]) {
    const button = titlebar.match(
      new RegExp(`<button(?=[^>]*id="${id}")[^>]*>[\\s\\S]*?<\\/button>`),
    )?.[0];
    assert.ok(button, `${id} should be a native button`);
    assert.match(button, /type="button"/);
    assert.match(button, /data-tauri-drag-region="false"/);
    if (["hideButton", "maximizeButton", "closeButton"].includes(id)) {
      assert.match(button, /aria-label="[^"]+"/);
      assert.match(button, /<svg[\s\S]*?aria-hidden="true"/);
    }
  }
});

test("window behavior updates maximize accessibility through the native bridge", () => {
  const javascript = read("ui/app.js");
  assert.match(javascript, /invoke\("get_window_chrome_state"/);
  assert.match(javascript, /invoke\("toggle_window_maximize"/);
  assert.match(javascript, /invoke\("close_window"/);
  assert.match(javascript, /restore \? "Restore VSParallel" : "Maximize VSParallel"/);
  assert.match(javascript, /window\.addEventListener\("resize", scheduleWindowChromeRefresh\)/);
  assert.match(javascript, /Boolean\(tauriInvoke\) && !isMac/);
  assert.doesNotMatch(javascript, /mockRequested|mockFailure|mockPlatform/);
});

test("chrome has complete interaction states and a resolved light palette", () => {
  const css = read("ui/styles.css");
  assert.match(css, /\[data-color-theme="light"\]/);
  assert.match(css, /\.window-control:hover/);
  assert.match(css, /\.window-control:active/);
  assert.match(css, /\.window-control:focus-visible/);
  assert.match(css, /\.window-control--close:hover/);
  assert.match(css, /data-window-focused="false"/);
  assert.match(css, /data-window-maximized="true"/);
});
