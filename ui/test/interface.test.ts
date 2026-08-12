"use strict";

import assert = require("node:assert/strict");
import fs = require("node:fs");
import path = require("node:path");
import { test } from "node:test";
import vm = require("node:vm");

const repository = path.resolve(process.cwd());

function read(relativePath: string): string {
  return fs.readFileSync(path.join(repository, relativePath), "utf8");
}

function readBuffer(relativePath: string): Buffer {
  return fs.readFileSync(path.join(repository, relativePath));
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

function pngMetadata(relativePath: string): {
  width: number;
  height: number;
  colorType: number;
} {
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

function sliceBetween(
  source: string,
  startPattern: RegExp,
  endPattern: RegExp,
  description: string,
): string {
  const startMatch = source.match(startPattern);
  assert.ok(startMatch, `${description} should have a start marker`);
  const start = startMatch.index;
  assert.ok(start !== undefined, `${description} start marker should have an index`);
  const remainder = source.slice(start + startMatch[0].length);
  const endMatch = remainder.match(endPattern);
  assert.ok(endMatch, `${description} should have an end marker`);
  const end = endMatch.index;
  assert.ok(end !== undefined, `${description} end marker should have an index`);
  return source.slice(start, start + startMatch[0].length + end);
}

function attribute(tag: string, name: string): string | null {
  const match = tag.match(new RegExp(`\\b${name}\\s*=\\s*(["'])(.*?)\\1`, "i"));
  return match?.[2] ?? null;
}

function cssBlocksMatching(css: string, pattern: RegExp): string {
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

test("global Codex and Claude usage meters emphasize remaining capacity", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  const overview = html.match(
    /<section\b(?=[^>]*class="[^"]*\busage-overview\b[^"]*")[^>]*>[\s\S]*?<\/section>/i,
  )?.[0];
  assert.ok(overview, "a global usage overview should exist outside workspace rows");
  assert.match(overview, /id="usageHeading"[^>]*>\s*Usage remaining\s*</i);
  assert.match(overview, /\baria-busy="true"/i);
  assert.equal(Array.from(overview.matchAll(/\bdata-provider="(?:codex|claude)"/g)).length, 2);

  for (const [provider, label] of [["codex", "Codex"], ["claude", "Claude"]]) {
    const card = overview.match(
      new RegExp(`<article\\b(?=[^>]*data-provider="${provider}")[\\s\\S]*?<\\/article>`, "i"),
    )?.[0];
    assert.ok(card, `${label} should have one usage card`);
    assert.match(card, new RegExp(`>\\s*${label}\\s*<`, "i"));
    assert.match(card, /\bdata-state="checking"/i);
    assert.match(card, /\baria-describedby="(?:codex|claude)UsageDetail"/i);
    assert.match(card, /class="usage-card__state"[^>]*hidden[^>]*>\s*Stale\s*</i);
    assert.match(card, /class="usage-meter"[^>]*aria-hidden="true"/i);
    assert.doesNotMatch(card, /\brole="meter"/i);
  }

  assert.match(
    css,
    /\.usage-grid\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/s,
  );
  assert.match(css, /\.usage-card__value\s*\{[^}]*font-variant-numeric:\s*tabular-nums/s);
  assert.match(css, /\.usage-meter__fill\s*\{[^}]*width:\s*calc\(var\(--usage-remaining\)\s*\*\s*1%\)/s);
  assert.match(css, /\.usage-card\[data-level="warning"\][^{]*\{[^}]*var\(--amber\)/s);
  assert.match(css, /\.usage-card\[data-level="critical"\][^{]*\{[^}]*var\(--red\)/s);
  assert.match(css, /\.usage-card\[data-state="stale"\][^{}]*\.usage-meter__fill\s*\{/s);
  assert.match(css, /\.usage-card__state\s*\{[^}]*text-transform:\s*uppercase/s);
  assert.match(javascript, /invoke\("get_usage",\s*\{\}\)/);
  assert.match(javascript, /textContent\s*=\s*`\$\{roundedRemaining\}% left`/);
  assert.match(javascript, /setAttribute\("role",\s*"meter"\)/);
  assert.match(javascript, /removeAttribute\("role"\)/);
  assert.match(javascript, /removeAttribute\("aria-valuenow"\)/);
  assert.match(javascript, /remaining\$\{stale \? " \(last known\)" : ""\}/);
  assert.match(javascript, /USAGE_REFRESH_INTERVAL_MS\s*=\s*60_000/);

  const asFiniteNumber = javascript.match(
    /^([ \t]+)function asFiniteNumber\(value, fallback = null\) \{[\s\S]*?^\1\}/m,
  )?.[0];
  const asPercentage = javascript.match(
    /^([ \t]+)function asPercentage\(value\) \{[\s\S]*?^\1\}/m,
  )?.[0];
  assert.ok(asFiniteNumber && asPercentage, "percentage normalization should be independently testable");
  const context = {} as {
    asPercentage(value: unknown): number | null;
  };
  vm.runInNewContext(`${asFiniteNumber}\n${asPercentage}`, context);
  assert.equal(context.asPercentage(null), null);
  assert.equal(context.asPercentage("42"), null);
  assert.equal(context.asPercentage(-8), 0);
  assert.equal(context.asPercentage(108), 100);
  assert.equal(context.asPercentage(42.5), 42.5);
});

test("usage normalization selects the limiting window and bounds last-known fallback", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "isObject",
    "isUnknownArray",
    "asString",
    "asFiniteNumber",
    "asTimestamp",
    "asPercentage",
    "normalizeStateToken",
    "usageWindowLabel",
    "normalizeUsageWindow",
    "normalizeUsageProvider",
    "usageProviderWithFallback",
  ].map((name) => appFunction(javascript, name)).join("\n");
  interface UsageProvider {
    detail: string;
    remainingPercent: number | null;
    resetsAtMs: number | null;
    state: string;
    windowLabel: string | null;
    windows: unknown[];
  }
  const context = {} as {
    normalizeUsageProvider(value: unknown, providerName: string): UsageProvider;
    usageProviderWithFallback(
      current: UsageProvider,
      previous: UsageProvider,
      nowMs: number,
    ): UsageProvider;
  };
  vm.runInNewContext(
    `const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000;
     const USAGE_LAST_KNOWN_MAX_AGE_MS = 15 * 60_000;
     ${functions}`,
    context,
  );

  const previous = context.normalizeUsageProvider({
    state: "available",
    updatedAtMs: 1_000,
    windows: [
      {
        label: "5-hour limit",
        durationMinutes: 300,
        remainingPercent: 60,
        resetsAtMs: 200_000,
      },
      {
        label: "7-day limit",
        durationMinutes: 10_080,
        remainingPercent: 18,
        resetsAtMs: 400_000,
      },
    ],
  }, "Codex");
  assert.equal(previous.remainingPercent, 18);
  assert.equal(previous.windowLabel, "7-day limit");
  assert.equal(previous.resetsAtMs, 400_000);

  const unavailable = context.normalizeUsageProvider({
    state: "unavailable",
    detail: "Open Codex and sign in to view limits.",
  }, "Codex");
  const fallback = context.usageProviderWithFallback(unavailable, previous, 2_000);
  assert.equal(fallback.state, "stale");
  assert.equal(fallback.remainingPercent, 18);
  assert.equal(fallback.detail, "Open Codex and sign in to view limits.");

  const partiallyExpired = context.usageProviderWithFallback(unavailable, previous, 250_000);
  assert.equal(partiallyExpired.windows.length, 1);
  assert.equal(partiallyExpired.windowLabel, "7-day limit");

  const fullyExpired = context.usageProviderWithFallback(unavailable, previous, 500_000);
  assert.equal(fullyExpired.state, "unavailable");
  assert.equal(fullyExpired.remainingPercent, null);
});

test("workspace activity preserves the backend's distinct no-activity label", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "isObject",
    "asString",
    "asFiniteNumber",
    "asTimestamp",
    "asNullableBoolean",
    "normalizeStateToken",
    "describeActivityState",
    "normalizeAntigravityModelKind",
    "normalizeActivityView",
  ].map((name) => appFunction(javascript, name)).join("\n");
  const context = {} as {
    normalizeActivityView(value: unknown): { label: string; kind: string; modelKind: string | null };
  };
  vm.runInNewContext(
    `const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000; ${functions}`,
    context,
  );

  const initial = context.normalizeActivityView({
    state: "unknown",
    label: "No activity yet",
    detail: "Submit a prompt from this workspace.",
  });
  assert.equal(initial.kind, "unknown");
  assert.equal(initial.label, "No activity yet");
  assert.equal(context.normalizeActivityView({ state: "unknown" }).modelKind, null);
  assert.equal(context.normalizeActivityView({ state: "unknown" }).label, "Unknown");
});

test("workspace normalization preserves Antigravity source and recent hook activity", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "isObject",
    "asString",
    "asFiniteNumber",
    "asTimestamp",
    "asNullableBoolean",
    "normalizeStateToken",
    "describeActivityState",
    "normalizeAntigravityModelKind",
    "normalizeActivityView",
    "deriveName",
    "normalizeWorkspace",
  ].map((name) => appFunction(javascript, name)).join("\n");
  const context = {} as {
    normalizeWorkspace(value: unknown, index: number): {
      editor: string;
      editorName: string;
      recentlyActive: boolean;
      openable: boolean;
      antigravity: { kind: string; label: string; modelKind: string | null } | null;
    };
  };
  vm.runInNewContext(
    `const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000; ${functions}`,
    context,
  );

  const workspace = context.normalizeWorkspace({
    instanceId: "antigravity-2:opaque",
    editor: "antigravity_2",
    editorName: "Antigravity 2.0",
    name: "project",
    path: "/work/project",
    openable: false,
    active: false,
    focused: false,
    recentlyActive: true,
    antigravity: {
      state: "turn_finished",
      label: "Turn finished",
      changedAtMs: 123,
      modelKind: "gemini_3_6_flash_medium",
    },
  }, 0);

  assert.equal(workspace.editor, "antigravity_2");
  assert.equal(workspace.editorName, "Antigravity 2.0");
  assert.equal(workspace.recentlyActive, true);
  assert.equal(workspace.openable, false);
  assert.equal(workspace.antigravity?.kind, "finished");
  assert.equal(workspace.antigravity?.label, "Turn finished");
  assert.equal(workspace.antigravity?.modelKind, "gemini_3_6_flash_medium");
});

test("Antigravity model labels accept only the public closed model set", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "asString",
    "normalizeStateToken",
    "normalizeAntigravityModelKind",
    "antigravityModelLabel",
  ].map((name) => appFunction(javascript, name)).join("\n");
  const context = {} as {
    normalizeAntigravityModelKind(value: unknown): string | null;
    antigravityModelLabel(value: string | null): string;
  };
  vm.runInNewContext(functions, context);

  const gemini = context.normalizeAntigravityModelKind("gemini_3_6_flash_medium");
  assert.equal(context.antigravityModelLabel(gemini), "Gemini 3.6 Flash (Medium)");
  const automatic = context.normalizeAntigravityModelKind("automatic");
  assert.equal(context.antigravityModelLabel(automatic), "Auto model");
  assert.equal(context.normalizeAntigravityModelKind("private-future-model"), null);
  assert.equal(context.antigravityModelLabel(null), "");
});

test("legacy companion heartbeats give an actionable IDE extension status", () => {
  const javascript = read("ui/generated/app.js");
  const context = {} as {
    describeExtensionPresence(value: {
      extensionDetectionAvailable: boolean | null;
      extensionInstalled: boolean | null;
      extensionActive: boolean | null;
      extensionRemote: boolean | null;
    }, remoteWindow?: boolean): { state: string; label: string; title: string };
  };
  vm.runInNewContext(appFunction(javascript, "describeExtensionPresence"), context);

  const legacy = context.describeExtensionPresence({
    extensionDetectionAvailable: null,
    extensionInstalled: null,
    extensionActive: null,
    extensionRemote: null,
  });
  assert.equal(legacy.label, "Reload VS Code for IDE status");
  assert.match(legacy.title, /Developer: Reload Window/);

  const unavailable = context.describeExtensionPresence({
    extensionDetectionAvailable: false,
    extensionInstalled: false,
    extensionActive: false,
    extensionRemote: null,
  });
  assert.equal(unavailable.label, "IDE extension status unavailable");

  const missing = context.describeExtensionPresence({
    extensionDetectionAvailable: true,
    extensionInstalled: false,
    extensionActive: false,
    extensionRemote: null,
  });
  assert.equal(missing.label, "IDE extension not detected · this window/profile");

  const remoteActive = context.describeExtensionPresence({
    extensionDetectionAvailable: true,
    extensionInstalled: true,
    extensionActive: true,
    extensionRemote: true,
  }, true);
  assert.equal(remoteActive.label, "IDE extension active · remote");
  assert.match(remoteActive.title, /cannot cross the remote host boundary/);

  const remoteUnavailable = context.describeExtensionPresence({
    extensionDetectionAvailable: false,
    extensionInstalled: false,
    extensionActive: false,
    extensionRemote: null,
  }, true);
  assert.equal(remoteUnavailable.label, "IDE extension status unavailable · remote window");
  assert.match(remoteUnavailable.title, /window\/profile/);

  const remoteInactive = context.describeExtensionPresence({
    extensionDetectionAvailable: true,
    extensionInstalled: true,
    extensionActive: false,
    extensionRemote: true,
  }, true);
  assert.equal(remoteInactive.label, "IDE extension installed · remote · inactive");
});

test("Codex setup keeps review guidance separate from installed status", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "isObject",
    "asString",
    "asNullableBoolean",
    "normalizeStateToken",
    "normalizeIntegrationComponent",
  ].map((name) => appFunction(javascript, name)).join("\n");
  const context = {} as {
    normalizeIntegrationComponent(value: unknown, kind: string): {
      installed: boolean;
      reviewRequired: boolean | null;
    };
  };
  vm.runInNewContext(functions, context);

  const trusted = context.normalizeIntegrationComponent({
    state: "installed",
    reviewRequired: false,
  }, "codex");
  assert.equal(trusted.installed, true);
  assert.equal(trusted.reviewRequired, false);
  assert.equal(context.normalizeIntegrationComponent({
    state: "installed",
    reviewRequired: true,
  }, "codex").reviewRequired, true);
});

test("setup summary treats VS Code and Antigravity IDE as peer editor choices", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "summarizeEditorCompanions",
    "describeSetupSummary",
  ].map((name) => appFunction(javascript, name)).join("\n");
  const context = {} as {
    describeSetupSummary(
      status: unknown,
      diagnosticsLoaded: boolean,
      diagnosticsUnavailable: boolean,
      diagnosticWarningCount: number,
    ): { summary: string; attention: boolean };
  };
  vm.runInNewContext(functions, context);

  const component = (
    kind: string,
    visualState: "missing" | "ready" | "warning" | "error",
    installed: boolean,
    token: string = visualState,
  ) => ({ kind, visualState, installed, token });
  const status = (
    vscode: ReturnType<typeof component>,
    antigravityIde: ReturnType<typeof component>,
    hookState: "missing" | "ready" = "ready",
  ) => ({
    companion: vscode,
    antigravityIde,
    antigravity: component("antigravity", hookState, hookState === "ready"),
    codex: component("codex", hookState, hookState === "ready"),
    claude: component("claude", hookState, hookState === "ready"),
  });

  const vscodeReady = context.describeSetupSummary(status(
    component("companion", "ready", true, "installed"),
    component("antigravityIde", "missing", false, "not_installed"),
  ), false, false, 0);
  assert.equal(vscodeReady.summary, "Integrations ready");
  assert.equal(vscodeReady.attention, false);

  const antigravityReady = context.describeSetupSummary(status(
    component("companion", "error", false, "unavailable"),
    component("antigravityIde", "ready", true, "installed"),
  ), true, false, 0);
  assert.equal(antigravityReady.summary, "Ready");
  assert.equal(antigravityReady.attention, false);

  const neitherReady = context.describeSetupSummary(status(
    component("companion", "missing", false, "not_installed"),
    component("antigravityIde", "error", false, "unavailable"),
  ), true, false, 0);
  assert.equal(neitherReady.summary, "Editor setup needed");
  assert.equal(neitherReady.attention, true);

  const optionalHooksMissing = context.describeSetupSummary(status(
    component("companion", "ready", true, "installed"),
    component("antigravityIde", "missing", false, "not_installed"),
    "missing",
  ), true, false, 0);
  assert.equal(optionalHooksMissing.summary, "Optional setup");
  assert.equal(optionalHooksMissing.attention, false);
});

test("setup-all skips either editor whose CLI is unavailable", () => {
  const javascript = read("ui/generated/app.js");
  const context = {} as {
    availableEditorSetupKinds(status: unknown): string[];
  };
  vm.runInNewContext(appFunction(javascript, "availableEditorSetupKinds"), context);

  const kinds = (companion: string, antigravityIde: string) => Array.from(
    context.availableEditorSetupKinds({
      companion: { token: companion },
      antigravityIde: { token: antigravityIde },
    }),
  );
  assert.deepEqual(kinds("not_installed", "not_installed"), [
    "companion",
    "antigravityIde",
  ]);
  assert.deepEqual(kinds("unavailable", "not_installed"), ["antigravityIde"]);
  assert.deepEqual(kinds("not_installed", "unavailable"), ["companion"]);
  assert.deepEqual(kinds("unavailable", "unavailable"), []);
});

test("setup separates activity hooks from provider usage requirements", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  const integrationSection = html.match(
    /<section\b(?=[^>]*class="[^"]*\bintegration-section\b[^"]*")[^>]*>[\s\S]*?<\/section>/i,
  )?.[0];
  assert.ok(integrationSection, "the integrations section should exist");
  const integrationText = integrationSection.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

  assert.match(integrationText, /Activity hooks and usage are separate\./i);
  assert.match(integrationText, /Hooks report lifecycle status/i);
  assert.doesNotMatch(integrationText, /VS Code extension is required/i);
  assert.match(integrationText, /Codex activity hooks/i);
  assert.match(integrationText, /Claude Code activity hooks/i);
  assert.match(integrationText, /Antigravity IDE companion/i);
  assert.match(integrationText, /Antigravity activity hooks/i);
  assert.match(integrationText, /Recent activity only/i);
  assert.match(integrationText, /Start an agent turn after installation/i);
  assert.match(integrationText, /Project-level \.agents\/hooks\.json can override/i);
  assert.match(
    integrationText,
    /Install at least one editor companion—VS Code or Antigravity IDE—to track live workspaces/i,
  );
  const antigravityIdeCard = integrationSection.match(
    /<article\b(?=[^>]*id="antigravityIdeCard")[^>]*>[\s\S]*?<\/article>/i,
  );
  assert.ok(antigravityIdeCard, "the Antigravity IDE companion card should exist");
  assert.doesNotMatch(antigravityIdeCard[0], /optional-label|>\s*Optional\s*</i);
  assert.match(
    integrationText,
    /compatible, signed-in Codex CLI available to this app, either from a standalone installation or the locally installed Codex extension in VS Code or Antigravity IDE/i,
  );
  assert.match(
    integrationText,
    /compatible, signed-in Claude Code CLI available to this app, either from a standalone installation or the locally installed Claude Code extension in VS Code or Antigravity IDE/i,
  );
  assert.match(integrationText, /recent terminal status-line capture can also provide fallback usage/i);
  assert.equal(
    Array.from(integrationSection.matchAll(/<p\b(?=[^>]*class="integration-usage-requirement")(?=[^>]*role="note")[^>]*>/gi)).length,
    2,
    "each provider should have a separate usage requirement",
  );
  assert.match(css, /\.integration-usage-requirement[\s,]/);
  assert.match(css, /\.integration-activity-limitation\s*\{/);
  assert.match(integrationText, /Set up monitoring/i);
  assert.doesNotMatch(integrationText, /Set up all/i);
  assert.match(javascript, /:\s*"Set up monitoring"/);
  assert.match(javascript, /Codex activity hooks installed\. Usage remaining is separate/);
  assert.match(javascript, /Claude Code activity hooks installed\. Usage remaining is separate/);
  assert.match(javascript, /Antigravity 2\.0 hook execution/);
  assert.match(javascript, /Opening an Antigravity 2\.0 Project does not fire hooks/);
  assert.match(javascript, /No available editor companion CLI was detected/);
  assert.doesNotMatch(javascript, /All integrations are installed|still needs/i);
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

test("provider model names remain discoverable and the status panel owns most row width", () => {
  const javascript = read("ui/generated/app.js");
  const css = read("ui/styles.css");
  assert.match(
    javascript,
    /createProviderState\(\s*"Claude"\s*,\s*workspace\.claude\s*,\s*"Claude Code"\s*,\s*workspace\.editorName\s*,\s*workspace\.remoteWindow\s*\)/,
  );
  assert.match(javascript, /antigravityModelLabel\(workspace\.antigravity\.modelKind\)/);
  assert.match(javascript, /modelLabel\s*\|\|\s*"Antigravity"/);
  assert.match(javascript, /latest model reported by Antigravity/);
  assert.doesNotMatch(javascript, /createProviderState\(\s*"Claude Code"\s*,/);
  assert.match(
    css,
    /\.workspace-row\s*\{[^}]*grid-template-columns\s*:\s*28px\s+minmax\(160px,\s*0\.7fr\)\s+minmax\(300px,\s*1\.3fr\)/i,
  );
  assert.match(
    css,
    /\.provider-state\s*\{[^}]*grid-template-columns\s*:\s*112px\s+minmax\(0,\s*1fr\)/i,
  );
  const providerName = css.match(/\.provider-name\s*\{([^}]*)\}/i)?.[1];
  assert.ok(providerName, "provider names should have dedicated styling");
  assert.match(providerName, /text-overflow\s*:\s*ellipsis/i);
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
  const javascript = read("ui/generated/app.js");
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
  assert.match(javascript, /status\.codex\.reviewRequired === true/);
  assert.doesNotMatch(javascript, /codexTrustGuidance\.hidden = !status\.codex\.installed/);
  assert.match(
    javascript,
    /addEventListener\("focus",\s*\(\) => \{[\s\S]*?isDialogOpen\(elements\.settingsDialog\)[\s\S]*?refreshIntegrationStatus\(\)/,
  );
  assert.match(html, /review and trust or re-enable the\s+VSParallel handlers/i);
});

test("workspace rows use a transparent native full-card action without visible Open text", () => {
  const javascript = read("ui/generated/app.js");
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
  assert.match(createRow, /workspace\.active \? "Switch to" : "Open"/);
  assert.match(
    createRow,
    /`\$\{actionLabel\} \$\{workspace\.name\} in \$\{workspace\.editorName\}`/,
  );
  assert.match(createRow, /workspace\.recentlyActive && !workspace\.openable/);
  assert.match(createRow, /hook activity does not identify a live window or exact open target/);

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
  const javascript = `${read("ui/generated/theme-init.js")}\n${read("ui/generated/app.js")}`;
  const css = read("ui/styles.css");
  const inputs = Array.from(html.matchAll(/<input\b[^>]*>/gi), (match) => match[0]);
  const radios = inputs.filter((input) => attribute(input, "type")?.toLowerCase() === "radio");
  const appearanceRadios = radios.filter((input) =>
    ["system", "light", "dark"].includes(attribute(input, "value")?.toLowerCase() ?? ""),
  );

  const appearanceValues = appearanceRadios.map((input) => {
    const value = attribute(input, "value");
    assert.ok(value, "each appearance choice should have a value");
    return value.toLowerCase();
  });
  assert.deepEqual(new Set(appearanceValues), new Set(["system", "light", "dark"]));
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
  assert.ok(system, "the System appearance choice should exist");
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
  const javascript = read("ui/generated/app.js");
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

test("workspace switches compact VSParallel without minimizing it", () => {
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
  assert.match(openWorkspace, /open_editor_with\s*\(/);
  assert.match(openWorkspace, /enter_floating_panel\s*\(/);
  assert.match(openWorkspace, /find_active_workspace_open_target\s*\(/);
  assert.match(openWorkspace, /WorkspaceLaunchMode::PreferExisting/);
  assert.match(openWorkspace, /WorkspaceLaunchMode::NewWindow/);
  assert.match(library, /wait_for_restored_window_state\s*\(/);
  assert.match(library, /wait_for_floating_panel_ready\s*\(/);
  assert.match(library, /set_visible_on_all_workspaces\(true\)/);
  assert.match(openWorkspace, /schedule_floating_panel_watchdog\s*\(/);
  assert.ok(
    openWorkspace.indexOf("enter_floating_panel") < openWorkspace.indexOf("open_editor_with"),
    "the native panel must be ready before the selected editor can switch desktops",
  );
  assert.doesNotMatch(openWorkspace, /\.minimize\s*\(/);
  assert.doesNotMatch(trayWorkspace, /\.minimize\s*\(/);
  assert.match(hideWindow, /\.minimize\s*\(/);
});
