"use strict";

import assert = require("node:assert/strict");
import fs = require("node:fs");
import path = require("node:path");
import test = require("node:test");
import vm = require("node:vm");

interface WindowConfiguration {
  decorations: boolean;
  resizable: boolean;
  transparent: boolean;
  theme?: string;
  titleBarStyle?: string;
  hiddenTitle?: boolean;
  width: number;
  height: number;
  minWidth: number;
  minHeight: number;
}

interface MainTauriConfiguration {
  app: {
    windows: WindowConfiguration[];
    macOSPrivateApi: boolean;
    security: {
      csp: string;
    };
  };
  build: {
    beforeBuildCommand: string;
    beforeDevCommand: {
      script: string;
      wait: boolean;
    };
    frontendDist: string[];
    devUrl?: string;
  };
  bundle: {
    resources: Record<string, string>;
  };
}

interface MacOSTauriConfiguration {
  app: {
    windows: WindowConfiguration[];
  };
}

interface CapabilityConfiguration {
  permissions: string[];
}

interface WindowChromeVmContext {
  state: {
    windowChromeRequestId: number;
  };
  advanceWindowChromeRequestId?: () => number;
  isCurrentWindowChromeRequest?: (requestId: number) => boolean;
}

const repository = path.resolve(process.cwd());

function read(relativePath: string): string {
  return fs.readFileSync(path.join(repository, relativePath), "utf8");
}

function readJson<T>(relativePath: string): T {
  return JSON.parse(read(relativePath)) as T;
}

test("desktop platforms use custom chrome while macOS retains native chrome", () => {
  const desktop = readJson<MainTauriConfiguration>("src-tauri/tauri.conf.json").app.windows[0];
  const macos = readJson<MacOSTauriConfiguration>(
    "src-tauri/tauri.macos.conf.json",
  ).app.windows[0];
  assert.ok(desktop, "the desktop window configuration should exist");
  assert.ok(macos, "the macOS window configuration should exist");

  assert.equal(desktop.decorations, false);
  assert.equal(desktop.resizable, true);
  assert.equal(desktop.transparent, true);
  assert.equal(desktop.theme, undefined);
  assert.equal(macos.decorations, true);
  assert.equal(macos.transparent, true);
  assert.equal(macos.titleBarStyle, "Transparent");
  assert.equal(macos.hiddenTitle, true);
  assert.equal(macos.resizable, true);
  assert.deepEqual(
    [macos.width, macos.height, macos.minWidth, macos.minHeight],
    [desktop.width, desktop.height, desktop.minWidth, desktop.minHeight],
  );
  assert.equal(
    readJson<MainTauriConfiguration>("src-tauri/tauri.conf.json").app.macOSPrivateApi,
    true,
  );
  const cargo = read("src-tauri/Cargo.toml");
  assert.match(cargo, /"macos-private-api"/);
  assert.match(cargo, /objc2-app-kit/);
  assert.match(cargo, /Win32_UI_Shell/);
  assert.match(cargo, /Win32_System_Threading/);
});

test("the capability grants only the frontend operations used by the app", () => {
  const capability = readJson<CapabilityConfiguration>(
    "src-tauri/capabilities/default.json",
  );
  assert.deepEqual(capability.permissions, [
    "core:window:allow-start-dragging",
    "process:allow-restart",
    "updater:allow-check",
    "updater:allow-download-and-install",
  ]);
});

test("the production frontend contains only explicit runtime assets", () => {
  const config = readJson<MainTauriConfiguration>("src-tauri/tauri.conf.json");
  assert.deepEqual(config.build.frontendDist, [
    "../ui/index.html",
    "../ui/styles.css",
    "../ui/generated/app.js",
    "../ui/generated/theme-init.js",
    "../ui/vsparallel-icon.png",
  ]);
  assert.equal(config.build.beforeBuildCommand, "npm run build:ui");
  assert.deepEqual(config.build.beforeDevCommand, {
    script: "npm run build:ui",
    wait: true,
  });
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

test("floating panel controls are universal, accessible, and initially hidden", () => {
  const html = read("ui/index.html");
  const titlebar = html.match(/<header[\s\S]*?id="appTitlebar"[\s\S]*?<\/header>/)?.[0];
  assert.ok(titlebar, "the integrated titlebar should exist");

  for (const [id, label] of [
    ["restoreFullButton", "Restore full VSParallel window"],
    ["hidePanelButton", "Hide floating panel"],
  ]) {
    const button = titlebar.match(
      new RegExp(`<button(?=[^>]*id="${id}")[^>]*>[\\s\\S]*?<\\/button>`),
    )?.[0];
    assert.ok(button, `${id} should be available on every platform`);
    assert.match(button, /type="button"/);
    assert.match(button, new RegExp(`aria-label="${label}"`));
    assert.match(button, /data-tauri-drag-region="false"/);
    assert.match(button, /\bhidden\b/);
    assert.match(button, /<svg[\s\S]*?aria-hidden="true"/);
  }
});

test("window behavior updates maximize accessibility through the native bridge", () => {
  const javascript = read("ui/generated/app.js");
  assert.match(javascript, /invoke\("get_window_chrome_state"/);
  assert.match(javascript, /invoke\("toggle_window_maximize"/);
  assert.match(javascript, /invoke\("close_window"/);
  assert.match(javascript, /restore \? "Restore VSParallel" : "Maximize VSParallel"/);
  assert.match(javascript, /window\.addEventListener\("resize", scheduleWindowChromeRefresh\)/);
  assert.match(javascript, /Boolean\(tauriInvoke\) && !isMac/);
  assert.doesNotMatch(javascript, /mockRequested|mockFailure|mockPlatform/);
});

test("workspace launches enter a rendered, draggable floating mode that can be restored", () => {
  const javascript = read("ui/generated/app.js");
  assert.match(javascript, /floating:\s*raw\.floating === true/);
  assert.match(
    javascript,
    /dataset\.windowMode\s*=\s*chrome\.floating\s*\?\s*"floating"\s*:\s*"full"/,
  );
  assert.match(javascript, /restoreFullButton\.hidden\s*=\s*!chrome\.floating/);
  assert.match(javascript, /hidePanelButton\.hidden\s*=\s*!chrome\.floating/);
  assert.match(javascript, /chrome\.customControls \|\| chrome\.floating/);
  assert.match(javascript, /invoke\("restore_full_window"/);
  assert.match(
    javascript,
    /Could not restore the full VSParallel window[\s\S]*?refreshWindowChromeState\(\)/,
  );
  assert.match(
    javascript,
    /invoke\("open_workspace"[\s\S]*?commitWindowChromeState\(result\)/,
  );
  assert.match(javascript, /windowChromeRequestId/);
  assert.match(
    javascript,
    /isCurrentWindowChromeRequest\(requestId\)[\s\S]*?renderWindowChromeState/,
  );
});

test("chrome request ordering rejects responses superseded by a newer command", () => {
  const javascript = read("ui/generated/app.js");
  const advance = javascript.match(
    /^([ \t]+)function advanceWindowChromeRequestId\(\) \{[\s\S]*?^\1\}/m,
  )?.[0];
  const isCurrent = javascript.match(
    /^([ \t]+)function isCurrentWindowChromeRequest\(requestId\) \{[\s\S]*?^\1\}/m,
  )?.[0];
  assert.ok(
    advance !== undefined && isCurrent !== undefined,
    "the latest-response gate should remain independently testable",
  );

  const context: WindowChromeVmContext = { state: { windowChromeRequestId: 0 } };
  vm.runInNewContext(`${advance}\n${isCurrent}`, context);
  const advanceWindowChromeRequestId = context.advanceWindowChromeRequestId;
  const isCurrentWindowChromeRequest = context.isCurrentWindowChromeRequest;
  assert.ok(
    advanceWindowChromeRequestId !== undefined && isCurrentWindowChromeRequest !== undefined,
    "the request-ordering helpers should be installed in the VM context",
  );
  const preLaunchRefresh = advanceWindowChromeRequestId();
  assert.equal(isCurrentWindowChromeRequest(preLaunchRefresh), true);

  const floatingCommit = advanceWindowChromeRequestId();
  assert.equal(isCurrentWindowChromeRequest(preLaunchRefresh), false);
  assert.equal(isCurrentWindowChromeRequest(floatingCommit), true);

  const newerRefresh = advanceWindowChromeRequestId();
  assert.equal(isCurrentWindowChromeRequest(floatingCommit), false);
  assert.equal(isCurrentWindowChromeRequest(newerRefresh), true);
});

test("native panel recovery follows delayed desktop activation without taking editor focus", () => {
  const library = read("src-tauri/src/lib.rs");

  assert.match(library, /enum WindowPresentationMode[\s\S]*?EnteringFloating[\s\S]*?Restoring/);
  assert.match(library, /wait_for_floating_panel_ready/);
  assert.match(library, /schedule_floating_panel_watchdog/);
  assert.match(library, /show_floating_panel_without_focus/);
  assert.match(library, /FullScreenAuxiliary/);
  assert.match(library, /IVirtualDesktopManager/);
  assert.match(library, /foreground_window_matches_editor/);
  assert.match(library, /QueryFullProcessImageNameW/);
  assert.match(library, /SWP_NOACTIVATE/);
  const reconciliation = library.match(
    /fn reconcile_floating_panel[\s\S]*?\nfn schedule_floating_panel_watchdog/,
  )?.[0];
  assert.ok(reconciliation, "the native reconciliation path should remain inspectable");
  assert.match(reconciliation, /window_presentation[\s\S]*?set_always_on_top/);
  assert.doesNotMatch(
    reconciliation,
    /run_on_main_thread/,
    "Linux window repairs must be queued from the worker instead of nesting Xlib access",
  );
  assert.match(
    library,
    /CloseRequested[\s\S]*?floating[\s\S]*?prevent_close\(\)[\s\S]*?minimize\(\)/,
  );
});

test("macOS compact chrome preserves native minimization", () => {
  const library = read("src-tauri/src/lib.rs");
  const floatingPresentation = library.match(
    /fn apply_floating_panel_presentation[\s\S]*?\nasync fn enter_floating_panel/,
  )?.[0];
  assert.ok(floatingPresentation, "the floating presentation should remain inspectable");

  const decorations = floatingPresentation.indexOf("set_decorations(false)");
  const resizing = floatingPresentation.indexOf("set_resizable(false)");
  const minimization = floatingPresentation.indexOf("set_minimizable(true)");
  assert.ok(decorations >= 0, "the compact panel should remove native decorations");
  assert.ok(
    resizing > decorations,
    "the compact panel should fix its size after going borderless",
  );
  assert.ok(
    minimization > resizing,
    "macOS minimizability must be restored after Tao replaces the borderless style mask",
  );

  const readiness = library.match(
    /async fn wait_for_floating_panel_ready[\s\S]*?\nfn apply_floating_panel_presentation/,
  )?.[0];
  assert.ok(readiness, "the floating readiness check should remain inspectable");
  assert.match(readiness, /is_minimizable\(\)/);
  assert.match(readiness, /if !minimizable[\s\S]*?set_minimizable\(true\)/);

  assert.match(
    library,
    /struct NormalWindowState[\s\S]*?minimizable:\s*bool[\s\S]*?macos_behavior/,
  );
  assert.match(
    library,
    /could not restore VSParallel minimization[\s\S]*?set_minimizable\(normal\.minimizable\)/,
  );

  const reconciliation = library.match(
    /fn reconcile_floating_panel[\s\S]*?\nfn schedule_floating_panel_watchdog/,
  )?.[0];
  assert.ok(reconciliation, "the native reconciliation path should remain inspectable");
  assert.match(
    reconciliation,
    /cfg\(target_os = "macos"\)[\s\S]*?is_minimized\(\)[\s\S]*?panel_hidden = true[\s\S]*?advance_window_generation/,
  );
  assert.match(
    reconciliation,
    /cfg\(not\(target_os = "macos"\)\)[\s\S]*?window[\s\S]*?\.unminimize\(\)/,
  );
});

test("GTK monitor geometry is captured on the native main thread", () => {
  const library = read("src-tauri/src/lib.rs");
  const placementCapture = library.match(
    /fn capture_floating_panel_placement[\s\S]*?\nfn floating_window_effects/,
  )?.[0];
  assert.ok(placementCapture, "the native monitor-placement helper should remain inspectable");
  assert.match(placementCapture, /run_on_main_thread/);
  assert.match(placementCapture, /current_monitor\(\)/);
  assert.match(placementCapture, /primary_monitor\(\)/);
  assert.match(placementCapture, /sender\.send\(placement\)/);

  const floatingTransition = library.match(
    /async fn enter_floating_panel[\s\S]*?\nfn record_first_window_error/,
  )?.[0];
  assert.ok(floatingTransition, "the initial floating transition should remain inspectable");
  assert.match(floatingTransition, /capture_floating_panel_placement\(window\)/);
  assert.doesNotMatch(
    floatingTransition,
    /window\.current_monitor\(\)/,
    "GTK-backed monitor handles must not be converted on the async command worker",
  );
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

test("floating mode is compact translucent glass with an opaque fallback", () => {
  const css = read("ui/styles.css");
  assert.match(
    css,
    /:root\[data-window-mode="floating"\][\s\S]*?background:\s*transparent/,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.app-shell\s*\{[^}]*max-width:\s*400px[^}]*backdrop-filter:\s*blur\(/,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.workspace-list\s*\{[^}]*overflow-y:\s*auto/,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.activity-providers\s*\{[^}]*display:\s*none/,
  );
  assert.match(css, /\.workspace-compact-status\s*\{[^}]*display:\s*none/s);
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.workspace-row\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)\s+auto/,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.workspace-compact-status\s*\{[^}]*display:\s*inline-flex[^}]*justify-self:\s*end/s,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.workspace-compact-status\[data-state="activity"\]\s*\{[^}]*color:\s*var\(--accent-text\)/s,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.workspace-compact-status\[data-state="finished"\]\s*\{[^}]*color:\s*var\(--green-text\)/s,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.usage-overview\s*\{[^}]*padding:\s*5px 6px 6px/,
  );
  assert.doesNotMatch(
    css,
    /data-window-mode="floating"\][^{}]*\.usage-overview\s*\{[^}]*display:\s*none/,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.usage-card__state\s*\{[^}]*font-size:\s*6px/,
  );
  assert.doesNotMatch(
    css,
    /data-window-mode="floating"\][^{}]*\.usage-card__state\s*\{[^}]*display:\s*none/,
  );
  assert.doesNotMatch(
    css,
    /data-window-mode="floating"\][^{}]*\.usage-card__detail\s*\{[^}]*display:\s*none/,
  );
  assert.match(
    css,
    /@media \(max-height:\s*300px\)[\s\S]*?data-window-mode="floating"\][^{}]*\.empty-state__mark,[\s\S]*?data-window-mode="floating"\][^{}]*\.empty-state p\s*\{[^}]*display:\s*none/,
  );
  assert.match(css, /@supports not \(\(backdrop-filter:/);
  assert.match(css, /@media \(prefers-reduced-transparency: reduce\)/);
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*#connectionText\s*\{[^}]*position:\s*absolute/,
  );
  assert.doesNotMatch(
    css,
    /data-window-mode="floating"\][^{}]*\.app-shell\s*\{[^}]*opacity:\s*0?\.[0-9]/,
  );
});
