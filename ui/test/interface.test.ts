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
  assert.doesNotMatch(titlebar, />\s*VS Code workspaces\s*</i);

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

test("the workspace empty state stays concise and explains visibility filtering", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  const emptyState = html.match(
    /<div\b(?=[^>]*id="emptyState")[^>]*>[\s\S]*?<\/div>/i,
  )?.[0];
  assert.ok(emptyState, "the workspace empty state should exist");
  assert.match(emptyState, /id="emptySetupButton"[^>]*>[\s\S]*?Open setup/i);
  assert.match(emptyState, /Zed for automatic read-only monitoring from local metadata/i);
  assert.doesNotMatch(emptyState, /experimental Cursor desktop bridge/i);
  assert.match(css, /\.empty-state__actions\s*\{[^}]*display:\s*flex[^}]*justify-content:\s*center/s);
  assert.match(
    javascript,
    /emptySetupButton:\s*requiredElement\("#emptySetupButton"\)/,
  );
  assert.match(
    javascript,
    /elements\.emptySetupButton\.addEventListener\("click",\s*openSettingsDialog\)/,
  );
  assert.match(javascript, /"No visible workspaces"/);
  assert.match(javascript, /Settings › Visibility/);
});

test("the six-provider dashboard renders quota, context, and token usage", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  const overview = html.match(
    /<section\b(?=[^>]*class="[^"]*\busage-overview\b[^"]*")[^>]*>[\s\S]*?<\/section>/i,
  )?.[0];
  assert.ok(overview, "a global usage overview should exist outside workspace rows");
  assert.match(overview, /id="usageHeading"[^>]*>\s*Provider usage\s*</i);
  assert.match(overview, /\baria-busy="true"/i);
  assert.equal(
    Array.from(
      overview.matchAll(
        /\bdata-provider="(?:codex|claude|gemini|antigravity|zed|cursor)"/g,
      ),
    ).length,
    6,
  );

  for (const [provider, label] of [
    ["codex", "Codex"],
    ["claude", "Claude"],
    ["gemini", "Gemini"],
    ["antigravity", "Antigravity"],
    ["zed", "Zed Agent"],
    ["cursor", "Cursor"],
  ]) {
    const card = overview.match(
      new RegExp(`<article\\b(?=[^>]*data-provider="${provider}")[\\s\\S]*?<\\/article>`, "i"),
    )?.[0];
    assert.ok(card, `${label} should have one usage card`);
    assert.match(card, new RegExp(`>\\s*${label}\\s*<`, "i"));
    assert.match(card, /\bdata-state="checking"/i);
    assert.match(card, new RegExp(`\\baria-describedby="${provider}UsageDetail"`, "i"));
    assert.match(card, /class="usage-card__state"[^>]*hidden[^>]*>\s*Stale\s*</i);
    assert.match(card, /class="usage-meter"[^>]*aria-hidden="true"/i);
    assert.doesNotMatch(card, /\brole="meter"/i);
  }

  assert.match(
    css,
    /\.usage-grid\s*\{[^}]*grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)/s,
  );
  assert.match(css, /\.usage-card__value\s*\{[^}]*font-variant-numeric:\s*tabular-nums/s);
  assert.match(css, /\.usage-meter__fill\s*\{[^}]*width:\s*calc\(var\(--usage-remaining\)\s*\*\s*1%\)/s);
  assert.match(css, /\.usage-card\[data-level="warning"\][^{]*\{[^}]*var\(--amber\)/s);
  assert.match(css, /\.usage-card\[data-level="critical"\][^{]*\{[^}]*var\(--red\)/s);
  assert.match(css, /\.usage-card\[data-state="stale"\][^{}]*\.usage-meter__fill\s*\{/s);
  assert.match(css, /\.usage-card__state\s*\{[^}]*text-transform:\s*uppercase/s);
  assert.match(javascript, /invoke\("get_usage",\s*\{\}\)/);
  assert.match(javascript, /`\$\{roundedRemaining\}% left`/);
  assert.match(javascript, /`\$\{roundedRemaining\}% context left`/);
  assert.match(javascript, /textContent\s*=\s*`\$\{formattedTokens\} tokens`/);
  assert.match(javascript, /resetUsageMeter\(target\.meter, true\)/);
  assert.match(css, /\.usage-meter\[hidden\]\s*\{[^}]*display:\s*none/s);
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

test("visibility settings default on and synchronize editor and usage preferences", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  const visibility = html.match(
    /<details\b(?=[^>]*class="[^"]*\bvisibility-section\b[^"]*")[^>]*>[\s\S]*?<\/details>/i,
  )?.[0];
  assert.ok(visibility, "visibility should be a compact settings disclosure");
  for (const editor of ["vscode", "cursor", "antigravity", "zed"]) {
    assert.match(
      visibility,
      new RegExp(`<input\\b(?=[^>]*data-editor-visibility="${editor}")(?=[^>]*checked)[^>]*>`, "i"),
    );
  }
  assert.match(
    visibility,
    /<input\b(?=[^>]*id="usageVisibilityInput")(?=[^>]*checked)[^>]*>/i,
  );
  assert.match(visibility, />\s*Provider usage\s*</i);
  assert.match(visibility, /available quota, context, and token information/i);
  assert.doesNotMatch(visibility, /Show Codex and Claude/i);
  assert.match(javascript, /invoke\("get_display_preferences",\s*\{\}\)/);
  assert.match(javascript, /invoke\("set_editor_visibility",\s*\{[\s\S]*editor:\s*kind,[\s\S]*visible:/);
  assert.match(javascript, /invoke\("set_usage_limit_percentage_visible",\s*\{\s*visible\s*\}\)/);
  assert.match(javascript, /await refreshSnapshot\(\)/);
  assert.match(javascript, /localStorage\.setItem\(\s*VISIBILITY_STORAGE_KEY/);
  assert.match(javascript, /function visibleWorkspaces\(/);
  assert.match(javascript, /if \(!state\.usageVisible\)/);
  assert.match(javascript, /if \(state\.usageRefreshPromise\)/);
  assert.match(css, /\.usage-overview\[hidden\]\s*\{[^}]*display:\s*none/s);
});

test("setup warnings expose specific hover and keyboard-accessible details", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  assert.match(
    html,
    /<button\b(?=[^>]*id="diagnosticsSummary")(?=[^>]*class="[^"]*help-popover__trigger)(?=[^>]*aria-describedby="diagnosticsSummaryDetail")[^>]*>/i,
  );
  assert.match(
    html,
    /<span\b(?=[^>]*id="diagnosticsSummaryDetail")(?=[^>]*role="tooltip")(?=[^>]*popover="manual")[^>]*>/i,
  );
  assert.match(javascript, /details\.push\(\.\.\.state\.diagnosticWarnings\)/);
  assert.match(javascript, /integrationComponentName\(component\.kind\)/);
  assert.match(javascript, /diagnosticsSummaryDetail\.textContent = details\.length/);
  assert.match(css, /\.diagnostics-summary\[data-attention="true"\]/);
  assert.match(css, /\.help-popover__trigger:focus-visible\s*\{[^}]*outline:/s);
});

test("advanced diagnostics keeps actionable paths and health while hiding raw record counts", () => {
  const html = read("ui/index.html");
  const javascript = read("ui/generated/app.js");
  const renderDiagnostics = sliceBetween(
    javascript,
    /function\s+renderDiagnostics\s*\(/,
    /async function\s+refreshDiagnostics\s*\(/,
    "renderDiagnostics",
  );
  assert.match(renderDiagnostics, /appendDiagnostic\("State directory"/);
  assert.match(renderDiagnostics, /appendDiagnostic\("VS Code command"/);
  assert.match(renderDiagnostics, /appendDiagnostic\("Cursor command"/);
  assert.doesNotMatch(renderDiagnostics, /appendDiagnostic\("Workspace records"/);
  assert.doesNotMatch(renderDiagnostics, /appendDiagnostic\("(?:Codex|Claude Code|Cursor) activity records"/);
  assert.match(html, /id="experimentalIntegrations"/);
  assert.match(javascript, /experimentalIntegrations\.append\(elements\.cursorAgentsBridgeCard\)/);
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
    "normalizeUsageMetricKind",
    "normalizeUsageProvider",
    "usageProviderHasMetric",
    "usageProviderWithFallback",
  ].map((name) => appFunction(javascript, name)).join("\n");
  interface UsageProvider {
    detail: string;
    metricLabel: string;
    metricKind: string;
    remainingPercent: number | null;
    resetsAtMs: number | null;
    state: string;
    tokenCount: number | null;
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
  assert.equal(previous.metricKind, "quota");
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

  const gemini = context.normalizeUsageProvider({
    state: "available",
    metricKind: "tokens",
    tokenCount: 12_345,
    metricLabel: "Latest model call",
    updatedAtMs: 1_000,
  }, "Gemini");
  assert.equal(gemini.metricKind, "tokens");
  assert.equal(gemini.tokenCount, 12_345);
  assert.equal(gemini.remainingPercent, null);
  const tokenFallback = context.usageProviderWithFallback(
    context.normalizeUsageProvider({ state: "unavailable", detail: "Refresh failed." }, "Gemini"),
    gemini,
    2_000,
  );
  assert.equal(tokenFallback.state, "stale");
  assert.equal(tokenFallback.tokenCount, 12_345);
  const disabledCapture = context.usageProviderWithFallback(
    context.normalizeUsageProvider({
      state: "unavailable",
      detail: "Gemini token capture is disabled.",
    }, "Gemini"),
    gemini,
    2_000,
  );
  assert.equal(disabledCapture.state, "unavailable");
  assert.equal(disabledCapture.tokenCount, null);

  for (const detail of [
    "Gemini usage capture is installed, but Gemini CLI settings disable hooks.",
    "Gemini usage capture is not installed. Open Setup & diagnostics.",
    "Gemini usage capture needs repair in Setup & diagnostics.",
    "The Gemini usage capture is incompatible. Repair it in Setup & diagnostics.",
    "Gemini usage hook conflicts with an existing hook.",
  ]) {
    const setupRequired = context.usageProviderWithFallback(
      context.normalizeUsageProvider({ state: "unavailable", detail }, "Gemini"),
      gemini,
      2_000,
    );
    assert.equal(setupRequired.state, "unavailable", detail);
    assert.equal(setupRequired.tokenCount, null, detail);
  }

  const cursor = context.normalizeUsageProvider({
    state: "available",
    metricKind: "context",
    remainingPercent: 72.5,
    metricLabel: "Latest CLI context",
    updatedAtMs: 1_000,
  }, "Cursor");
  assert.equal(cursor.metricKind, "context");
  assert.equal(cursor.remainingPercent, 72.5);
  assert.equal(cursor.tokenCount, null);

  const cursorTurn = context.normalizeUsageProvider({
    state: "available",
    metricKind: "tokens",
    tokenCount: 9_876,
    metricLabel: "Latest Cursor turn",
    updatedAtMs: 2_000,
  }, "Cursor");
  assert.equal(cursorTurn.metricKind, "tokens");
  assert.equal(cursorTurn.tokenCount, 9_876);
  assert.equal(cursorTurn.remainingPercent, null);
  assert.equal(cursorTurn.metricLabel, "Latest Cursor turn");

  const futureMetric = context.normalizeUsageProvider({
    state: "available",
    metricKind: "future-private-metric",
    remainingPercent: 90,
  }, "Provider");
  assert.equal(futureMetric.metricKind, "none");
  assert.equal(futureMetric.state, "unavailable");
});

test("a forced usage refresh supersedes an in-flight pre-uninstall response", async () => {
  const javascript = read("ui/generated/app.js");
  const refreshUsage = sliceBetween(
    javascript,
    /async function\s+refreshUsage\s*\(/,
    /function\s+refreshUsageIfDue\s*\(/,
    "refreshUsage",
  );
  const resolvers: Array<(value: unknown) => void> = [];
  const rendered: unknown[] = [];
  const state = {
    usageVisible: true,
    usagePending: false,
    usageRefreshPromise: null as Promise<void> | null,
    usageRefreshGeneration: 0,
    lastUsageAttemptAtMs: null as number | null,
    lastUsage: null as unknown,
  };
  const context = {
    state,
    updateRefreshControl() {},
    invoke() {
      return new Promise((resolve) => resolvers.push(resolve));
    },
    normalizeUsageSnapshot(value: unknown) {
      return value;
    },
    unavailableUsageSnapshot(detail: string) {
      return { detail };
    },
    resolveUsageSnapshot(current: unknown) {
      return current;
    },
    renderUsageSnapshot(snapshot: unknown) {
      rendered.push(snapshot);
    },
  } as unknown as {
    refreshUsage(forceAfterPending?: boolean): Promise<void>;
  };
  vm.runInNewContext(refreshUsage, context);

  const first = context.refreshUsage();
  assert.equal(resolvers.length, 1);
  const forced = context.refreshUsage(true);
  assert.equal(state.usageRefreshGeneration, 1);

  resolvers[0]({ source: "before uninstall" });
  await first;
  await Promise.resolve();
  assert.equal(rendered.length, 0, "the invalidated response must never render");
  assert.equal(resolvers.length, 2, "a fresh request must be queued after the pending one");

  resolvers[1]({ source: "after uninstall" });
  await forced;
  assert.deepEqual(rendered, [{ source: "after uninstall" }]);
  assert.deepEqual(state.lastUsage, { source: "after uninstall" });
  assert.equal(state.usagePending, false);
  assert.equal(state.usageRefreshPromise, null);
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
    normalizeActivityView(value: unknown): {
      label: string;
      kind: string;
      modelKind: string | null;
      modelName: string;
      agentKind: string;
    };
  };
  vm.runInNewContext(
    `const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000; ${functions}`,
    context,
  );

  const initial = context.normalizeActivityView({
    state: "unknown",
    label: "No activity yet",
    detail: "Submit a prompt from this workspace.",
    modelName: "gpt-5.6-codex",
    agentKind: "Agent",
  });
  assert.equal(initial.kind, "unknown");
  assert.equal(initial.label, "No activity yet");
  assert.equal(initial.modelName, "gpt-5.6-codex");
  assert.equal(initial.agentKind, "Agent");
  assert.equal(context.normalizeActivityView({ state: "unknown" }).modelKind, null);
  assert.equal(context.normalizeActivityView({ state: "unknown" }).label, "Unknown");
});

test("workspace normalization preserves Cursor IDE and agent metadata", () => {
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
      surface: string;
      cursor: {
        kind: string;
        label: string;
        modelName: string;
        agentKind: string;
      } | null;
    };
  };
  vm.runInNewContext(
    `const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000; ${functions}`,
    context,
  );

  const workspace = context.normalizeWorkspace({
    instanceId: "cursor-window",
    editor: "cursor",
    name: "project",
    path: "/work/project",
    cursor: {
      state: "turn_finished",
      modelName: "claude-4-sonnet",
      agentKind: "Background agent",
    },
  }, 0);

  assert.equal(workspace.editor, "cursor");
  assert.equal(workspace.editorName, "Cursor");
  assert.equal(workspace.surface, "editor_workspace");
  assert.equal(workspace.cursor?.kind, "finished");
  assert.equal(workspace.cursor?.label, "Turn finished");
  assert.equal(workspace.cursor?.modelName, "claude-4-sonnet");
  assert.equal(workspace.cursor?.agentKind, "Background agent");
});

test("workspace normalization preserves Zed metadata and keeps unknown editors on VS Code", () => {
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
      active: boolean;
      recentlyActive: boolean;
      openable: boolean;
      zed: {
        kind: string;
        label: string;
        changedAtMs: number | null;
        modelName: string;
        agentKind: string;
      } | null;
    };
  };
  vm.runInNewContext(
    `const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000; ${functions}`,
    context,
  );

  const workspace = context.normalizeWorkspace({
    instanceId: "zed-window",
    editor: "zed",
    name: "project",
    path: "/work/project",
    openable: true,
    active: false,
    recentlyActive: true,
    zed: {
      state: "recent_activity",
      label: "Recent agent activity",
      changedAtMs: 123,
      modelName: "claude-sonnet-4",
      agentKind: "Agent panel",
    },
  }, 0);

  assert.equal(workspace.editor, "zed");
  assert.equal(workspace.editorName, "Zed");
  assert.equal(workspace.active, false);
  assert.equal(workspace.recentlyActive, true);
  assert.equal(workspace.openable, true);
  assert.equal(workspace.zed?.kind, "recent");
  assert.equal(workspace.zed?.label, "Recent agent activity");
  assert.equal(workspace.zed?.changedAtMs, 123);
  assert.equal(workspace.zed?.modelName, "claude-sonnet-4");
  assert.equal(workspace.zed?.agentKind, "Agent panel");

  const active = context.normalizeWorkspace({
    instanceId: "zed-active",
    editor: "zed",
    zed: {
      state: "activity_detected",
      changedAtMs: 456,
    },
  }, 1);
  assert.equal(active.zed?.kind, "activity");
  assert.equal(active.zed?.label, "Activity detected");

  const finished = context.normalizeWorkspace({
    instanceId: "zed-finished",
    editor: "zed",
    zed: {
      state: "turn_finished",
      changedAtMs: 789,
    },
  }, 2);
  assert.equal(finished.zed?.kind, "finished");
  assert.equal(finished.zed?.label, "Turn finished");

  const fallback = context.normalizeWorkspace({
    instanceId: "unknown-window",
    editor: "future_editor",
    path: "/work/fallback",
  }, 3);
  assert.equal(fallback.editor, "vscode");
  assert.equal(fallback.editorName, "VS Code");
  assert.equal(fallback.zed, null);
});

test("Zed monitoring is described as automatic and has no installable integration card", () => {
  const html = read("ui/index.html");
  const typescript = read("ui/app.ts");
  const integrationKind = typescript.match(
    /type IntegrationKind\s*=([\s\S]*?);/,
  )?.[1];
  assert.ok(integrationKind, "the integration kind union should exist");

  assert.match(html, /Zed monitoring is\s+automatic and read-only/i);
  assert.match(html, /uses local workspace and agent metadata/i);
  assert.doesNotMatch(html, /id="zed(?:Card|InstallButton|UninstallButton)"/i);
  assert.doesNotMatch(integrationKind, /["']zed["']/i);
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
      surface: string;
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
    surface: "hook_only",
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
  assert.equal(workspace.surface, "hook_only");
  assert.equal(workspace.recentlyActive, true);
  assert.equal(workspace.openable, false);
  assert.equal(workspace.antigravity?.kind, "finished");
  assert.equal(workspace.antigravity?.label, "Turn finished");
  assert.equal(workspace.antigravity?.modelKind, "gemini_3_6_flash_medium");
});

test("workspace normalization keeps experimental Cursor agent rows non-openable", () => {
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
      surface: string;
      openable: boolean;
    };
  };
  vm.runInNewContext(
    `const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000; ${functions}`,
    context,
  );

  const workspace = context.normalizeWorkspace({
    instanceId: "cursor-agent-thread:opaque",
    editor: "cursor",
    surface: "cursor_agent_thread",
    name: "project",
    path: "/work/project",
    openable: true,
  }, 0);

  assert.equal(workspace.editor, "cursor");
  assert.equal(workspace.editorName, "Cursor agent thread (experimental)");
  assert.equal(workspace.surface, "cursor_agent_thread");
  assert.equal(workspace.openable, false, "the private bridge must not create an open target");
});

test("workspace activity aggregation keeps the highest-priority lifecycle state", () => {
  const javascript = read("ui/generated/app.js");
  const context = {} as {
    aggregateActivity(workspace: {
      codex: Activity;
      claude: Activity;
      antigravity: Activity | null;
      cursor: Activity | null;
      zed: Activity | null;
    }): Activity;
  };
  interface Activity {
    kind: "activity" | "finished" | "failure" | "recent" | "unknown";
    label: string;
    modelKind?: string | null;
  }
  const activity = (
    kind: Activity["kind"],
    label: string,
    modelKind: string | null = null,
  ): Activity => ({ kind, label, modelKind });
  vm.runInNewContext(appFunction(javascript, "aggregateActivity"), context);

  const active = context.aggregateActivity({
    codex: activity("finished", "Turn finished"),
    claude: activity("activity", "Activity detected"),
    antigravity: activity("failure", "Failed/interrupted"),
    cursor: null,
    zed: null,
  });
  assert.equal(active.kind, "activity");
  assert.equal(active.label, "Activity detected");

  const finished = context.aggregateActivity({
    codex: activity("unknown", "No activity yet"),
    claude: activity("finished", "Turn finished"),
    antigravity: null,
    cursor: null,
    zed: null,
  });
  assert.equal(finished.kind, "finished");
  assert.equal(finished.label, "Turn finished");

  const antigravity = activity(
    "activity",
    "Activity detected",
    "gemini_3_6_flash_medium",
  );
  assert.equal(
    context.aggregateActivity({
      codex: activity("finished", "Turn finished"),
      claude: activity("unknown", "No activity yet"),
      antigravity,
      cursor: null,
      zed: null,
    }),
    antigravity,
    "Antigravity lifecycle activity should participate without synthesizing a model label",
  );

  const failure = context.aggregateActivity({
    codex: activity("finished", "Turn finished"),
    claude: activity("failure", "Failed/interrupted"),
    antigravity: activity("unknown", "Unknown"),
    cursor: null,
    zed: null,
  });
  assert.equal(failure.kind, "failure");
  assert.equal(failure.label, "Failed/interrupted");

  const cursor = activity("activity", "Activity detected");
  assert.equal(context.aggregateActivity({
    codex: activity("finished", "Turn finished"),
    claude: activity("unknown", "No activity yet"),
    antigravity: null,
    cursor,
    zed: null,
  }), cursor);

  const zed = activity("recent", "Recent agent activity");
  assert.equal(context.aggregateActivity({
    codex: activity("unknown", "No activity yet"),
    claude: activity("unknown", "No activity yet"),
    antigravity: null,
    cursor: null,
    zed,
  }), zed);
});

test("workspaces render as cards in single Open and Recent sections", () => {
  const html = read("ui/index.html");
  const javascript = read("ui/generated/app.js");
  const css = read("ui/styles.css");
  const grouping = appFunction(javascript, "groupWorkspaces");
  const context = {} as {
    groupWorkspaces(
      workspaces: Array<{ instanceId: string; active: boolean }>,
    ): Array<{
      kind: string;
      label: string;
      workspaces: Array<{ instanceId: string; active: boolean }>;
    }>;
  };
  vm.runInNewContext(grouping, context);

  const groups = context.groupWorkspaces([
    { instanceId: "recent-a", active: false },
    { instanceId: "open-a", active: true },
    { instanceId: "recent-b", active: false },
    { instanceId: "open-b", active: true },
  ]);
  assert.deepEqual(Array.from(groups, (group) => group.kind), ["open", "recent"]);
  assert.deepEqual(Array.from(groups, (group) => group.label), ["Open", "Recent"]);
  assert.deepEqual(
    Array.from(groups[0].workspaces, (workspace) => workspace.instanceId),
    ["open-a", "open-b"],
  );
  assert.deepEqual(
    Array.from(groups[1].workspaces, (workspace) => workspace.instanceId),
    ["recent-a", "recent-b"],
  );
  assert.deepEqual(
    Array.from(
      context.groupWorkspaces([{ instanceId: "recent-only", active: false }]),
      (group) => group.kind,
    ),
    ["recent"],
  );
  assert.deepEqual(
    Array.from(
      context.groupWorkspaces([{ instanceId: "open-only", active: true }]),
      (group) => group.kind,
    ),
    ["open"],
  );
  assert.equal(context.groupWorkspaces([]).length, 0);

  assert.match(
    html,
    /<div\b(?=[^>]*id="workspaceList")(?=[^>]*class="[^"]*\bworkspace-list\b)[^>]*>/i,
  );
  assert.match(
    javascript,
    /createElement\(\s*["']section["']\s*,\s*["']workspace-group["']\s*\)/,
  );
  assert.match(
    javascript,
    /createElement\(\s*["']h3["']\s*,\s*["']workspace-group__heading["']\s*,\s*group\.label\s*\)/,
  );
  assert.match(
    javascript,
    /createElement\(\s*["']ul["']\s*,\s*["']workspace-group__cards["']\s*\)/,
  );
  assert.match(javascript, /section\.setAttribute\(\s*["']aria-labelledby["']/);
  assert.match(
    javascript,
    /group\.workspaces\.forEach\(\s*\(workspace\)\s*=>\s*\{\s*cards\.append\(createWorkspaceRow\(workspace\)\)/,
  );
  assert.match(javascript, /section\.append\(heading,\s*cards\)/);
  assert.match(javascript, /groupWorkspaces\(displayedWorkspaces\)/);
  assert.match(javascript, /const displayedWorkspaces = visibleWorkspaces\(snapshot\.workspaces\)/);
  assert.doesNotMatch(javascript, /window-badge/);
  assert.doesNotMatch(css, /\.window-badge\b/);
  assert.match(css, /\.workspace-group__cards\s*\{[^}]*list-style:\s*none/s);
  assert.match(
    css,
    /\.workspace-row\s*\{[^}]*border:\s*1px solid var\(--border\)[^}]*background:\s*var\(--panel-deep\)/s,
  );
  assert.doesNotMatch(
    css,
    /\.workspace-row\.is-inactive[^{}]*(?:workspace-title-line|workspace-meta|activity-providers)[^{]*\{[^}]*opacity/s,
  );
});

test("workspace names lead the application label hierarchy without theming the entire card", () => {
  const javascript = read("ui/generated/app.js");
  const css = read("ui/styles.css");
  const createRow = sliceBetween(
    javascript,
    /function\s+createWorkspaceRow\s*\(/,
    /function\s+groupWorkspaces\s*\(/,
    "createWorkspaceRow",
  );

  assert.doesNotMatch(createRow, /row\.dataset\.editor/);
  assert.match(
    createRow,
    /createElement\(\s*["']span["']\s*,\s*["']workspace-application["']\s*,\s*workspace\.editorName\s*\)/,
  );
  assert.match(createRow, /application\.dataset\.editor\s*=\s*workspace\.editor/);
  assert.match(createRow, /primary\.append\(application,\s*titleLine,\s*metaLine\)/);
  assert.doesNotMatch(createRow, /createElement\(\s*["'](?:img|svg)["']/i);
  assert.doesNotMatch(css, /\.workspace-application(?:::before|::after)\s*\{/i);
  assert.doesNotMatch(
    cssBlocksMatching(css, /\.workspace-application/),
    /(?:^|[;{])\s*(?:-webkit-)?mask(?:-image)?\s*:|url\s*\(/i,
  );

  const applicationLabel = css.match(/\.workspace-application\s*\{([^}]*)\}/i)?.[1];
  assert.ok(applicationLabel, "application labels should have dedicated styling");
  assert.match(applicationLabel, /font-size\s*:\s*9px/i);
  assert.match(applicationLabel, /font-weight\s*:\s*680/i);
  assert.match(applicationLabel, /line-height\s*:\s*14px/i);

  const workspaceName = css.match(/\.workspace-name\s*\{([^}]*)\}/i)?.[1];
  assert.ok(workspaceName, "workspace names should have dedicated styling");
  assert.match(workspaceName, /font-size\s*:\s*15px/i);
  assert.match(workspaceName, /font-weight\s*:\s*680/i);
  assert.match(workspaceName, /line-height\s*:\s*20px/i);
  assert.match(css, /\.workspace-row\.is-inactive\s*\{[^}]*opacity\s*:\s*1\s*;/i);
  assert.doesNotMatch(
    css,
    /\.workspace-row\.is-inactive\s+\.workspace-application/,
  );
  assert.match(
    css,
    /data-window-mode="floating"\][^{}]*\.workspace-application\s*\{[^}]*font-size\s*:\s*9px/i,
  );

  assert.doesNotMatch(css, /\.workspace-row\[data-editor=/i);
  assert.match(applicationLabel, /border:\s*1px solid var\(--border-strong\)/i);
  assert.match(applicationLabel, /background:\s*var\(--panel-deep\)/i);
  assert.match(applicationLabel, /color:\s*var\(--text-subtle\)/i);
  assert.doesNotMatch(css, /\.workspace-application\[data-editor=/i);
  assert.doesNotMatch(css, /antigravity-rainbow/i);
});

test("Antigravity model labels accept only the public closed model set", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "asString",
    "normalizeStateToken",
    "normalizeAntigravityModelKind",
    "antigravityModelLabel",
    "antigravityModelFamilyLabel",
  ].map((name) => appFunction(javascript, name)).join("\n");
  const context = {} as {
    normalizeAntigravityModelKind(value: unknown): string | null;
    antigravityModelLabel(value: string | null): string;
    antigravityModelFamilyLabel(value: string | null): string;
  };
  vm.runInNewContext(functions, context);

  const gemini = context.normalizeAntigravityModelKind("gemini_3_6_flash_medium");
  assert.equal(context.antigravityModelLabel(gemini), "Gemini 3.6 Flash (Medium)");
  const geminiHigh = context.normalizeAntigravityModelKind("gemini_3_6_flash_high");
  assert.equal(context.antigravityModelLabel(geminiHigh), "Gemini 3.6 Flash (High)");
  assert.equal(context.antigravityModelFamilyLabel(geminiHigh), "Gemini");
  const claude = context.normalizeAntigravityModelKind("claude_sonnet_4_6_thinking");
  assert.equal(context.antigravityModelLabel(claude), "Claude Sonnet 4.6 (Thinking)");
  assert.equal(context.antigravityModelFamilyLabel(claude), "Claude");
  const gptOss = context.normalizeAntigravityModelKind("gpt_oss_120b_medium");
  assert.equal(context.antigravityModelLabel(gptOss), "GPT-OSS 120B (Medium)");
  assert.equal(context.antigravityModelFamilyLabel(gptOss), "GPT-OSS");
  const automatic = context.normalizeAntigravityModelKind("automatic");
  assert.equal(context.antigravityModelLabel(automatic), "Auto model");
  assert.equal(context.normalizeAntigravityModelKind("private-future-model"), null);
  assert.equal(context.antigravityModelLabel(null), "");
  assert.equal(context.antigravityModelFamilyLabel(null), "");
});

test("an unreadable Antigravity health receipt is not treated as an observed hook", () => {
  const javascript = read("ui/generated/app.js");
  const context = {} as {
    antigravityHookWasObserved(outcome: string): boolean;
  };
  vm.runInNewContext(appFunction(javascript, "antigravityHookWasObserved"), context);

  assert.equal(context.antigravityHookWasObserved("recorded"), true);
  assert.equal(context.antigravityHookWasObserved("no_workspace"), true);
  assert.equal(context.antigravityHookWasObserved("not_observed"), false);
  assert.equal(context.antigravityHookWasObserved("health_unreadable"), false);
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

test("provider setup preserves review and manual-action states", () => {
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
      token: string;
      visualState: string;
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

  const manuallyDisabled = context.normalizeIntegrationComponent({
    state: "manual_action_required",
    label: "Installed · enable hooks manually",
  }, "gemini");
  assert.equal(manuallyDisabled.installed, true);
  assert.equal(manuallyDisabled.token, "manual_action_required");
  assert.equal(manuallyDisabled.visualState, "warning");
});

test("editor setup combines required companion and hook components", () => {
  const javascript = read("ui/generated/app.js");
  const html = read("ui/index.html");
  const context = {} as {
    integrationInstallButtonLabel(component: {
      kind: string;
      actionLabel: string;
    }): string;
    combineIntegrationComponents(kind: string, components: unknown[]): {
      visualState: string;
      installed: boolean;
      actionLabel: string;
      token: string;
      label: string;
    };
  };
  vm.runInNewContext([
    appFunction(javascript, "integrationComponentName"),
    appFunction(javascript, "combineIntegrationComponents"),
    appFunction(javascript, "integrationInstallButtonLabel"),
  ].join("\n"), context);

  assert.equal(context.integrationInstallButtonLabel({
    kind: "cursorCompanion",
    actionLabel: "Install",
  }), "Install");

  const component = (
    kind: string,
    visualState: "missing" | "ready" | "warning" | "error",
    installed: boolean,
    token: string = visualState,
  ) => ({
    kind,
    visualState,
    installed,
    token,
    label: token === "manual_action_required"
      ? "Installed · custom status line kept"
      : `${kind} ${visualState}`,
    actionLabel: visualState === "missing" ? "Install" : "Repair",
    detail: `${kind} ${visualState}`,
  });
  const combine = (
    first: ReturnType<typeof component>,
    second: ReturnType<typeof component>,
  ) => context.combineIntegrationComponents("cursorCompanion", [first, second]);
  assert.equal(combine(
    component("cursorCompanion", "ready", true),
    component("cursor", "ready", true),
  ).visualState, "ready");
  assert.equal(combine(
    component("cursorCompanion", "missing", false),
    component("cursor", "missing", false),
  ).visualState, "missing");
  assert.equal(combine(
    component("cursorCompanion", "ready", true),
    component("cursor", "missing", false),
  ).visualState, "warning");
  assert.equal(combine(
    component("cursorCompanion", "error", false, "unavailable"),
    component("cursor", "ready", true),
  ).visualState, "warning");
  assert.equal(combine(
    component("cursorCompanion", "error", false, "unavailable"),
    component("cursor", "error", false, "unavailable"),
  ).visualState, "error");
  const customStatusLine = combine(
    component("cursorCompanion", "ready", true),
    component("cursor", "warning", true, "manual_action_required"),
  );
  assert.equal(customStatusLine.visualState, "warning");
  assert.equal(customStatusLine.token, "manual_action_required");
  assert.equal(customStatusLine.label, "Installed · custom status line kept");
  assert.notEqual(combine(
    component("cursorCompanion", "missing", false),
    component("cursor", "warning", true, "manual_action_required"),
  ).token, "manual_action_required");

  assert.doesNotMatch(html, /\bid="cursorCard"/i);
  assert.doesNotMatch(html, /\bid="antigravityCard"/i);

  const runIntegrationAction = sliceBetween(
    javascript,
    /async function\s+runIntegrationAction\s*\(/,
    /function\s+formatNaturalList\s*\(/,
    "runIntegrationAction",
  );
  assert.match(
    runIntegrationAction,
    /cursorCompanion:\s*\{\s*install:\s*["']install_cursor_monitoring["'],\s*uninstall:\s*["']uninstall_cursor_monitoring["']/s,
  );
  assert.match(
    runIntegrationAction,
    /antigravityIde:\s*\{\s*install:\s*["']install_antigravity_monitoring["'],\s*uninstall:\s*["']uninstall_antigravity_monitoring["']/s,
  );
  assert.match(
    runIntegrationAction,
    /catch\s*\([^)]*\)\s*\{[\s\S]*invoke\(["']get_integration_status["']/,
    "a partially applied setup action should refresh its visible status",
  );
  assert.match(
    runIntegrationAction,
    /resultComponent\.token === ["']manual_action_required["'][\s\S]*setIntegrationMessage\([\s\S]*["']warning["']/,
    "a structurally installed integration that still needs manual action must not report success",
  );
});

test("Cursor agent-thread monitoring is an explicit experimental opt-in", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  const cursorCompanionIndex = html.indexOf('id="cursorCompanionCard"');
  const cursorAgentsIndex = html.indexOf('id="cursorAgentsBridgeCard"');
  const antigravityIdeIndex = html.indexOf('id="antigravityIdeCard"');
  assert.ok(
    cursorCompanionIndex >= 0
      && cursorAgentsIndex > cursorCompanionIndex
      && antigravityIdeIndex > cursorAgentsIndex,
    "the experimental card should follow the Cursor companion",
  );

  const card = html.match(
    /<article\b(?=[^>]*id="cursorAgentsBridgeCard")[^>]*>[\s\S]*?<\/article>/i,
  )?.[0];
  assert.ok(card, "the Cursor agent-thread monitoring card should exist");
  assert.match(card, /^<article\b[^>]*\bhidden\b/i);
  assert.match(html, /<div\s+id="experimentalIntegrations"><\/div>/i);
  assert.match(
    javascript,
    /experimentalIntegrations\.append\(elements\.cursorAgentsBridgeCard\)[\s\S]*cursorAgentsBridgeCard\.hidden = false/,
  );
  assert.match(card, /Cursor Agents Window/i);
  assert.match(card, />\s*Experimental\s*</i);
  assert.match(card, /id="cursorAgentsBridgeDetail"[^>]*>\s*Optional live thread status/i);
  assert.match(
    card,
    /<button\b(?=[^>]*class="help-popover__trigger")(?=[^>]*type="button")(?=[^>]*aria-label="About experimental Cursor agent-thread monitoring")(?=[^>]*aria-describedby="cursorAgentsBridgePrivacy")[^>]*>/i,
  );
  const tooltip = card.match(
    /<span\b(?=[^>]*class="help-popover__content")(?=[^>]*id="cursorAgentsBridgePrivacy")(?=[^>]*role="tooltip")(?=[^>]*popover="manual")[^>]*>[\s\S]*?<\/span>\s*<\/span>/i,
  )?.[0];
  assert.ok(tooltip, "the detailed Cursor bridge guidance should be a help tooltip");
  assert.match(card, /id="cursorAgentsMonitoringEnabled"[^>]*type="checkbox"[^>]*role="switch"/i);
  const switchInput = card.match(
    /<input\b(?=[^>]*id="cursorAgentsMonitoringEnabled")[^>]*>/i,
  )?.[0] ?? "";
  assert.doesNotMatch(switchInput, /\bchecked\b/i);
  assert.match(switchInput, /aria-describedby="cursorAgentsBridgeDetail cursorAgentsMonitoringHint"/i);
  assert.doesNotMatch(switchInput, /cursorAgentsBridgePrivacy/i);
  assert.match(tooltip, /Settings &gt; Beta &gt; Desktop Bridge &gt; Allow CLI to access\s+desktop agents/i);
  assert.match(tooltip, /limited server-controlled rollout/i);
  assert.match(tooltip, /If that section is absent,[\s\S]*live agent-thread monitoring is unavailable/i);
  assert.match(tooltip, /Cursor hooks still provide\s+recent activity/i);
  assert.match(tooltip, /exact hook match/i);
  assert.match(tooltip, /never keeps\s+prompts, responses, credentials, or raw thread and window identifiers/i);
  assert.match(tooltip, /Cursor changes may break this experimental integration/i);
  assert.match(card, /Off by default/i);

  assert.match(css, /\.setting-switch input:checked \+ \.setting-switch__track\s*\{/);
  assert.match(css, /\.setting-switch input:focus-visible \+ \.setting-switch__track\s*\{/);
  assert.match(css, /\.help-popover__trigger:focus-visible\s*\{[^}]*outline:/s);
  assert.match(css, /\.help-popover__content\s*\{[^}]*position:\s*fixed[^}]*max-width:/s);
  assert.match(javascript, /function initializeHelpPopovers\(\)/);
  assert.match(javascript, /addEventListener\("pointerenter"/);
  assert.match(javascript, /addEventListener\("focus"/);
  assert.match(javascript, /showPopover\(\)/);
  assert.match(javascript, /hidePopover\(\)/);
  assert.match(javascript, /"get_cursor_agents_bridge_status"/);
  assert.match(javascript, /"set_cursor_agents_monitoring_enabled"/);

  const setupAll = sliceBetween(
    javascript,
    /async function\s+setupAllIntegrations\s*\(/,
    /function\s+openSettingsDialog\s*\(/,
    "setupAllIntegrations",
  );
  assert.doesNotMatch(setupAll, /set_cursor_agents_monitoring_enabled/);
  const setupSummary = appFunction(javascript, "describeSetupSummary");
  assert.doesNotMatch(setupSummary, /cursorAgents|bridge/i);
});

test("Cursor bridge status is closed, bounded, and rendered independently", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "isObject",
    "asString",
    "asFiniteNumber",
    "asNonNegativeInteger",
    "asTimestamp",
    "parseBridgeValue",
    "normalizeStateToken",
    "normalizeCursorAgentsBridgeStatus",
    "formatRelativeTime",
    "describeCursorAgentsBridgeStatus",
  ].map((name) => appFunction(javascript, name)).join("\n");
  const context = {} as {
    normalizeCursorAgentsBridgeStatus(value: unknown): {
      enabled: boolean;
      availability: string;
      connected: boolean;
      instanceCount: number;
      threadCount: number;
      errorCode: string;
    };
    describeCursorAgentsBridgeStatus(status: unknown): {
      state: string;
      label: string;
      detail: string;
    };
  };
  vm.runInNewContext(
    `const SCHEMA_VERSION = 1;
     const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000;
     ${functions}`,
    context,
  );

  const connected = context.normalizeCursorAgentsBridgeStatus({
    schemaVersion: 1,
    enabled: true,
    availability: "connected",
    connected: true,
    instanceCount: 1,
    threadCount: 2,
    lastCheckedAtMs: Date.now(),
    errorCode: null,
    detail: "Connected read-only.",
    token: "must-not-be-selected",
  });
  assert.equal(connected.enabled, true);
  assert.equal(connected.availability, "connected");
  assert.equal(connected.connected, true);
  assert.equal(connected.instanceCount, 1);
  assert.equal(connected.threadCount, 2);
  assert.equal(connected.errorCode, "");
  const connectedView = context.describeCursorAgentsBridgeStatus(connected);
  assert.equal(connectedView.state, "ready");
  assert.equal(connectedView.label, "Connected");
  assert.match(connectedView.detail, /1 instance · 2 threads/);

  const waiting = context.normalizeCursorAgentsBridgeStatus({
    schemaVersion: 1,
    enabled: true,
    availability: "waiting",
    connected: false,
    instanceCount: 0,
    threadCount: 0,
    lastCheckedAtMs: Date.now(),
    errorCode: "bridge_not_found",
    detail: "",
  });
  const waitingView = context.describeCursorAgentsBridgeStatus(waiting);
  assert.equal(waitingView.label, "Not connected");
  assert.equal(waitingView.detail, "Desktop Bridge is not available");
  assert.match(javascript, /cursorAgentsBridgeHelpStatus\.textContent = status\.detail/);

  const future = context.normalizeCursorAgentsBridgeStatus({
    schemaVersion: 1,
    enabled: true,
    availability: "future-private-state",
    connected: true,
    instanceCount: -2,
    threadCount: -4,
  });
  assert.equal(future.availability, "error");
  assert.equal(future.connected, false);
  assert.equal(future.instanceCount, 0);
  assert.equal(future.threadCount, 0);
  assert.equal(context.describeCursorAgentsBridgeStatus(future).label, "Check failed");
  assert.throws(() => context.normalizeCursorAgentsBridgeStatus({ schemaVersion: 2 }));

  const setter = sliceBetween(
    javascript,
    /async function\s+setCursorAgentsMonitoringEnabled\s*\(/,
    /function\s+setIntegrationMessage\s*\(/,
    "setCursorAgentsMonitoringEnabled",
  );
  assert.match(setter, /invoke\("set_cursor_agents_monitoring_enabled",\s*\{\s*enabled\s*\}\)/);
  assert.match(setter, /const status = normalizeCursorAgentsBridgeStatus\(raw\)/);
  assert.match(setter, /renderCursorAgentsBridgeStatus\(status\)/);
  assert.match(setter, /status\.enabled/);
  assert.match(setter, /await refreshSnapshot\(\)/);
  assert.match(setter, /renderCursorAgentsBridgeStatus\(previous\)/);

  const refreshSetup = sliceBetween(
    javascript,
    /async function\s+refreshSetup\s*\(/,
    /async function\s+hideWindow\s*\(/,
    "refreshSetup",
  );
  assert.match(refreshSetup, /refreshCursorAgentsBridgeStatus\(\)/);
  assert.match(
    javascript,
    /elements\.cursorAgentsMonitoringEnabled\.addEventListener\("change"/,
  );
});

test("setup summary treats VS Code, Cursor, and Antigravity IDE as peer editor choices", () => {
  const javascript = read("ui/generated/app.js");
  const functions = [
    "integrationComponentName",
    "combineIntegrationComponents",
    "visibleIntegrationComponent",
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
  ) => ({
    kind,
    visualState,
    installed,
    token,
    actionLabel: visualState === "missing" ? "Install" : "Repair",
    detail: `${kind} ${visualState}`,
  });
  const missing = (kind: string) => component(kind, "missing", false, "not_installed");
  const ready = (kind: string) => component(kind, "ready", true, "installed");
  const baseStatus = () => ({
    companion: ready("companion"),
    cursorCompanion: missing("cursorCompanion"),
    antigravityIde: missing("antigravityIde"),
    cursor: missing("cursor"),
    antigravity: missing("antigravity"),
    gemini: ready("gemini"),
    codex: ready("codex"),
    claude: ready("claude"),
  });

  const vscodeReady = context.describeSetupSummary(baseStatus(), false, false, 0);
  assert.equal(vscodeReady.summary, "Integrations ready");
  assert.equal(vscodeReady.attention, false);

  const antigravityStatus = baseStatus();
  antigravityStatus.companion = missing("companion");
  antigravityStatus.antigravityIde = ready("antigravityIde");
  antigravityStatus.antigravity = ready("antigravity");
  const antigravityReady = context.describeSetupSummary(
    antigravityStatus,
    true,
    false,
    0,
  );
  assert.equal(antigravityReady.summary, "Ready");
  assert.equal(antigravityReady.attention, false);

  const cursorStatus = baseStatus();
  cursorStatus.companion = missing("companion");
  cursorStatus.cursorCompanion = ready("cursorCompanion");
  cursorStatus.cursor = ready("cursor");
  const cursorReady = context.describeSetupSummary(cursorStatus, true, false, 0);
  assert.equal(cursorReady.summary, "Ready");
  assert.equal(cursorReady.attention, false);

  const neitherStatus = baseStatus();
  neitherStatus.companion = missing("companion");
  const neitherReady = context.describeSetupSummary(neitherStatus, true, false, 0);
  assert.equal(neitherReady.summary, "Editor setup needed");
  assert.equal(neitherReady.attention, true);

  const optionalStatus = baseStatus();
  optionalStatus.gemini = missing("gemini");
  optionalStatus.codex = missing("codex");
  optionalStatus.claude = missing("claude");
  const optionalHooksMissing = context.describeSetupSummary(optionalStatus, true, false, 0);
  assert.equal(optionalHooksMissing.summary, "Optional setup");
  assert.equal(optionalHooksMissing.attention, false);

  const partialStatus = baseStatus();
  partialStatus.cursorCompanion = component("cursorCompanion", "error", false, "unavailable");
  partialStatus.cursor = ready("cursor");
  const partialCursor = context.describeSetupSummary(partialStatus, true, false, 0);
  assert.equal(partialCursor.summary, "1 warning");
  assert.equal(partialCursor.attention, true);

  const diagnosticFailure = context.describeSetupSummary(baseStatus(), true, true, 0);
  assert.equal(diagnosticFailure.summary, "1 warning");
  assert.equal(diagnosticFailure.attention, true);
});

test("setup-all skips each editor whose CLI is unavailable", () => {
  const javascript = read("ui/generated/app.js");
  const context = {} as {
    availableEditorSetupKinds(status: unknown): string[];
  };
  vm.runInNewContext(appFunction(javascript, "availableEditorSetupKinds"), context);

  const kinds = (
    companion: string,
    cursorCompanion: string,
    cursor: string,
    antigravityIde: string,
    antigravity: string,
  ) => Array.from(
    context.availableEditorSetupKinds({
      companion: { token: companion },
      cursorCompanion: { token: cursorCompanion },
      cursor: { token: cursor },
      antigravityIde: { token: antigravityIde },
      antigravity: { token: antigravity },
    }),
  );
  assert.deepEqual(kinds("not_installed", "not_installed", "not_installed", "not_installed", "not_installed"), [
    "companion",
    "cursorCompanion",
    "antigravityIde",
  ]);
  assert.deepEqual(kinds("unavailable", "not_installed", "not_installed", "not_installed", "not_installed"), [
    "cursorCompanion",
    "antigravityIde",
  ]);
  assert.deepEqual(kinds("not_installed", "unavailable", "unavailable", "unavailable", "unavailable"), ["companion"]);
  assert.deepEqual(
    kinds("unavailable", "unavailable", "not_installed", "unavailable", "not_installed"),
    [],
  );
  assert.deepEqual(kinds("unavailable", "unavailable", "unavailable", "unavailable", "unavailable"), []);

  const setupAll = sliceBetween(
    javascript,
    /async function\s+setupAllIntegrations\s*\(/,
    /function\s+openSettingsDialog\s*\(/,
    "setupAllIntegrations",
  );
  assert.match(
    setupAll,
    /command:\s*["']install_cursor_monitoring["']/,
  );
  assert.match(
    setupAll,
    /command:\s*["']install_antigravity_monitoring["']/,
  );
  assert.match(
    setupAll,
    /kind:\s*["']gemini["'][\s\S]*?command:\s*["']install_gemini_usage["']/,
  );
  assert.doesNotMatch(setupAll, /install_cursor_hooks|install_antigravity_hooks/);
  assert.match(
    setupAll,
    /Monitoring installed\. Reload affected editors, restart provider sessions, and review \/hooks in Codex\./,
  );
  assert.match(
    setupAll,
    /resultComponent\.token === ["']manual_action_required["'][\s\S]*manualActions\.push/,
  );
  assert.match(
    setupAll,
    /unconfirmed\.length === 0 && manualActions\.length === 0/,
    "the all-installed success banner must exclude unresolved manual actions",
  );
  assert.match(setupAll, /Setup needs attention\.[\s\S]*Manual action remains:/);
});

test("settings keep integration summaries compact and expose details through accessible help", () => {
  const html = read("ui/index.html");
  const css = read("ui/styles.css");
  const javascript = read("ui/generated/app.js");
  const integrationSection = html.match(
    /<section\b(?=[^>]*class="[^"]*\bintegration-section\b[^"]*")[^>]*>[\s\S]*?<\/section>/i,
  )?.[0];
  assert.ok(integrationSection, "the integrations section should exist");
  const cards = Array.from(integrationSection.matchAll(
    /<article\b([^>]*)>[\s\S]*?<\/article>/gi,
  ));
  const normalCards = cards.filter((match) => !/\bhidden\b/i.test(match[1]));
  assert.equal(normalCards.length, 6, "the normal list should have six unified rows");
  const normalMarkup = normalCards.map((match) => match[0]).join("\n");
  const integrationText = normalMarkup.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

  assert.match(integrationSection, /Install one complete integration per editor/i);
  assert.match(integrationSection, /Zed monitoring is automatic and read-only/i);
  for (const id of [
    "companionCard",
    "cursorCompanionCard",
    "antigravityIdeCard",
    "geminiCard",
    "codexCard",
    "claudeCard",
  ]) {
    assert.match(normalMarkup, new RegExp(`\\bid="${id}"`, "i"));
  }
  assert.doesNotMatch(normalMarkup, /Cursor hooks only|Antigravity activity hooks/i);

  const visibleDescriptions = Array.from(normalMarkup.matchAll(
    /<p\b(?=[^>]*class="[^"]*\bintegration-detail\b[^"]*")[^>]*>([\s\S]*?)<\/p>/gi,
  )).map((match) => match[1].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim());
  assert.deepEqual(visibleDescriptions, [
    "Live workspace detection",
    "Live workspaces and agent activity",
    "Live workspaces and agent activity",
    "Local model-call token totals",
    "Agent activity hooks",
    "Agent activity hooks",
  ]);

  const helpTriggers = Array.from(normalMarkup.matchAll(
    /<button\b(?=[^>]*class="help-popover__trigger")(?=[^>]*type="button")(?=[^>]*aria-label="[^"]+")(?=[^>]*aria-describedby="([^"]+)")[^>]*>/gi,
  ));
  assert.equal(helpTriggers.length, 6, "each integration should have a labeled help button");
  const helpIds = helpTriggers.map((match) => match[1]);
  assert.equal(new Set(helpIds).size, 6, "each help button should describe a unique tooltip");
  for (const helpId of helpIds) {
    assert.match(
      normalMarkup,
      new RegExp(
        '<span\\b(?=[^>]*class="help-popover__content")(?=[^>]*id="' +
          helpId +
          '")(?=[^>]*role="tooltip")(?=[^>]*popover="manual")[^>]*>',
        "i",
      ),
    );
  }

  const antigravityIdeCard = integrationSection.match(
    /<article\b(?=[^>]*id="antigravityIdeCard")[^>]*>[\s\S]*?<\/article>/i,
  );
  assert.ok(antigravityIdeCard, "the Antigravity integration row should exist");
  assert.doesNotMatch(antigravityIdeCard[0], /optional-label|>\s*Optional\s*</i);
  assert.match(antigravityIdeCard[0], /companion and activity hooks\s+together/i);
  const cursorCompanionCard = integrationSection.match(
    /<article\b(?=[^>]*id="cursorCompanionCard")[^>]*>[\s\S]*?<\/article>/i,
  );
  assert.ok(cursorCompanionCard, "the Cursor integration row should exist");
  assert.doesNotMatch(cursorCompanionCard[0], /optional-label|>\s*Optional\s*</i);
  assert.match(
    cursorCompanionCard[0],
    /reload Cursor IDE or open a new Cursor Agent CLI session[\s\S]*local Cursor agent turn in IDE Composer or the CLI[\s\S]*richer[\s\S]*context-left percentage/i,
  );
  assert.match(cursorCompanionCard[0], /never reads Cursor plan or billing quota/i);
  assert.match(cursorCompanionCard[0], />\s*Install\s*</i);
  assert.match(cursorCompanionCard[0], />\s*Uninstall\s*</i);
  assert.match(
    integrationText,
    /compatible, signed-in Codex CLI available to VSParallel, either standalone or from a local Codex extension in VS Code, Cursor, or Antigravity IDE/i,
  );
  assert.match(
    integrationText,
    /compatible, signed-in Claude Code CLI available to VSParallel, either standalone or from a local Claude Code extension in VS Code, Cursor, or Antigravity IDE/i,
  );
  assert.match(integrationText, /recent terminal status-line capture can provide fallback usage/i);
  assert.match(integrationText, /Gemini CLI AfterModel hook/i);
  assert.match(integrationText, /discards prompt and response content/i);
  assert.match(integrationText, /not Gemini subscription quota/i);
  assert.doesNotMatch(integrationSection, /integration-usage-requirement|integration-activity-limitation/i);
  assert.match(css, /\.help-popover__trigger\s*\{[\s\S]*?width:\s*24px/i);
  assert.match(css, /\.help-popover__trigger:focus-visible\s*\{[\s\S]*?outline:/i);
  assert.match(css, /\.help-popover__content\s*\{[\s\S]*?position:\s*fixed/i);
  assert.match(css, /\.help-popover__content\s*\{[\s\S]*?max-width:/i);
  assert.match(css, /\.help-popover__content:popover-open[\s\S]*?data-fallback-open="true"/i);
  assert.doesNotMatch(css, /\.integration-usage-requirement[\s,{]|\.integration-activity-limitation[\s,{]/);
  assert.match(integrationSection, /Set up monitoring/i);
  assert.match(integrationSection, /id="uninstallAllButton"[\s\S]*?Uninstall all/i);
  assert.match(javascript, /uninstallAllButton\.disabled = busy \|\| !status/);
  const uninstallAll = sliceBetween(
    javascript,
    /async function uninstallAllIntegrations\(\)/,
    /function openSettingsDialog\(\)/,
    "uninstall-all action",
  );
  const uninstallRefreshes = uninstallAll.match(
    /Promise\.all\(\[[\s\S]*?refreshSnapshot\(\),[\s\S]*?refreshUsage\(true\),[\s\S]*?refreshCursorAgentsBridgeStatus\(\),?[\s\S]*?\]\)/g,
  ) ?? [];
  assert.equal(
    uninstallRefreshes.length,
    2,
    "both successful and partial/error uninstalls should refresh visible state",
  );
  assert.match(
    uninstallAll,
    /catch \(error\)[\s\S]*?setIntegrationMessage\(message, "error"\);[\s\S]*?Promise\.all\(\[[\s\S]*?refreshSnapshot\(\),[\s\S]*?refreshUsage\(true\),[\s\S]*?refreshCursorAgentsBridgeStatus\(\),?[\s\S]*?\]\)/,
  );
  const individualUninstall = sliceBetween(
    javascript,
    /async function\s+runIntegrationAction\s*\(/,
    /function\s+formatNaturalList\s*\(/,
    "individual integration action",
  );
  assert.equal(
    individualUninstall.match(
      /operation === "uninstall"[\s\S]*?Promise\.all\(\[refreshSnapshot\(\), refreshUsage\(true\)\]\)/g,
    )?.length,
    2,
    "both successful and partial/error individual uninstalls should force a fresh usage snapshot",
  );
  assert.match(
    uninstallAll,
    /Integration-backed editor monitoring was disabled[\s\S]*Automatic Zed discovery and supported provider quota checks remain available\./,
  );
  assert.match(
    uninstallAll,
    /component\.token === "unavailable"[\s\S]*Physical companion removal could not be verified[\s\S]*"warning"/,
  );
  assert.match(css, /\.setup-message\.has-warning\s*\{[^}]*color:\s*var\(--amber-text\)/s);
  assert.match(
    javascript,
    /will stop integration-backed editor monitoring[\s\S]*Automatic Zed discovery and supported provider quota checks remain available\./,
  );
  assert.doesNotMatch(javascript, /(?:All local tracking|stop all local tracking)/i);
  assert.match(javascript, /:\s*"Set up monitoring"/);
  assert.match(javascript, /componentElements\.detail\.textContent = integrationPurpose\(component\.kind\)/);
  assert.match(javascript, /componentElements\.meta\.hidden = true/);
  assert.match(
    javascript,
    /installButton\.hidden = component\.visualState === "ready"[\s\S]*?component\.token === "manual_action_required"/,
  );
  assert.match(javascript, /componentElements\.helpDetail\.textContent = helpDetails\.length/);
  assert.match(javascript, /Current status details are unavailable\./);
  assert.match(javascript, /function integrationPurpose\(/);
  assert.match(javascript, /querySelectorAll\("\.help-popover"\)/);
  assert.match(javascript, /addEventListener\("pointerenter"/);
  assert.match(javascript, /addEventListener\("focus"/);
  assert.match(javascript, /addEventListener\("click"/);
  assert.match(javascript, /\.showPopover\(\)/);
  assert.match(javascript, /\.hidePopover\(\)/);
  assert.match(javascript, /closeActiveHelpPopover\(\)/);
  assert.match(javascript, /Codex hooks installed\. Review \/hooks in Codex\./);
  assert.match(javascript, /Claude Code hooks installed\. Restart active sessions\./);
  assert.match(
    integrationText,
    /Gemini card has no capture[\s\S]*Install or Repair[\s\S]*new Gemini CLI session/i,
  );
  assert.match(
    javascript,
    /Gemini usage hook installed\. Open a new Gemini CLI session and start a turn\./,
  );
  assert.match(javascript, /install:\s*"install_gemini_usage"/);
  assert.match(javascript, /uninstall:\s*"uninstall_gemini_usage"/);
  assert.match(javascript, /gemini:\s*normalizeIntegrationComponent\(raw\.gemini, "gemini"\)/);
  assert.match(javascript, /renderIntegrationComponent\(status\.gemini\)/);
  assert.match(
    javascript,
    /Cursor monitoring installed\. Reload open Cursor IDE windows or open a new Cursor Agent CLI session, then start a turn\./,
  );
  assert.match(javascript, /Antigravity integration installed\. Reload open Antigravity IDE windows/);
  assert.match(javascript, /Antigravity 2\.0 hook execution/);
  assert.match(javascript, /Antigravity IDE hook execution/);
  assert.match(javascript, /Cursor live heartbeat/);
  assert.match(javascript, /Cursor workspace hook/);
  assert.match(javascript, /Cursor command/);
  assert.match(javascript, /No editor companion was available/);
  assert.doesNotMatch(javascript, /All integrations are installed|still needs/i);
});

test("Cursor heartbeat diagnostics keep hook-only Agents Window evidence informational", () => {
  const javascript = read("ui/generated/app.js");
  const context = {} as {
    describeCursorHeartbeatDiagnostic(
      activeRecords: number,
      retainedRecords: number,
      latestDescription: string,
      recentWorkspaceOpens: number,
    ): string;
  };
  vm.runInNewContext(
    appFunction(javascript, "describeCursorHeartbeatDiagnostic"),
    context,
  );

  assert.equal(
    context.describeCursorHeartbeatDiagnostic(2, 3, "just now", 1),
    "Active · latest just now",
  );
  assert.match(
    context.describeCursorHeartbeatDiagnostic(0, 0, "unavailable", 1),
    /Hook activity observed.*exact experimental bridge match is required for live thread status/,
  );
  assert.match(
    context.describeCursorHeartbeatDiagnostic(0, 1, "5 minutes ago", 0),
    /Inactive.*unmatched Agents Window activity is hook-only/,
  );
  assert.doesNotMatch(
    context.describeCursorHeartbeatDiagnostic(2, 3, "just now", 1),
    /\d+ active|retained|records?/i,
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

test("workspace rows omit redundant leading lifecycle icons while keeping provider details", () => {
  const javascript = read("ui/generated/app.js");
  const css = read("ui/styles.css");
  const createRow = sliceBetween(
    javascript,
    /function\s+createWorkspaceRow\s*\(/,
    /function\s+groupWorkspaces\s*\(/,
    "createWorkspaceRow",
  );
  assert.match(
    javascript,
    /createProviderState\(\s*"Claude"\s*,\s*workspace\.claude\s*,\s*"Claude Code"\s*,\s*workspace\.editorName\s*,\s*workspace\.remoteWindow\s*,/,
  );
  assert.match(javascript, /antigravityModelLabel\(workspace\.antigravity\.modelKind\)/);
  assert.match(javascript, /antigravityModelFamilyLabel\(workspace\.antigravity\.modelKind\)/);
  assert.match(javascript, /createProviderState\(\s*"Antigravity",\s*workspace\.antigravity,/);
  assert.match(javascript, /createProviderState\(\s*"Cursor Agent",\s*workspace\.cursor,/);
  assert.match(javascript, /workspace\.cursor\.agentKind/);
  assert.match(javascript, /workspace\.cursor\.modelName/);
  assert.match(javascript, /createProviderState\(\s*"Zed Agent",\s*workspace\.zed,/);
  assert.match(
    createRow,
    /workspace\.zed\.agentKind === "Agent panel"\s*\? ""\s*:\s*workspace\.zed\.agentKind/,
  );
  assert.match(javascript, /workspace\.zed\.modelName/);
  assert.match(javascript, /"Zed local metadata",\s*zedDetails/);
  assert.match(
    createRow,
    /workspace\.editorName,\s*false,\s*false,\s*"Zed local metadata"/,
  );
  assert.match(createRow, /workspace\.editor === "zed"\s*\? "Agent lifecycle and local metadata"/);
  assert.match(javascript, /Coarse persisted Zed Agent turn boundaries and model information reported by Zed's read-only local metadata/);
  assert.match(createRow, /nativeReadOnlyEditor\s*=\s*workspace\.editor\s*===\s*"zed"/);
  assert.match(
    createRow,
    /nativeReadOnlyEditor\s*\?\s*"Workspace-matched lifecycle records"/,
  );
  assert.match(javascript, /`Antigravity \(\$\{modelLabel\}\), latest model reported by Antigravity`/);
  assert.match(javascript, /"Antigravity built-in model",\s*modelFamily,\s*modelLabel/);
  assert.match(javascript, /latest model reported by Antigravity/);
  assert.match(javascript, /Antigravity built-in model/);
  assert.match(css, /\.provider-name-detail\s*\{[^}]*display:\s*block/s);
  assert.match(css, /\.provider-name-detail\s*\{[^}]*text-overflow:\s*ellipsis/s);
  assert.match(css, /\.provider-name-detail\s*\{[^}]*text-transform:\s*none/s);
  assert.doesNotMatch(javascript, /createProviderState\(\s*"Claude Code"\s*,/);
  assert.doesNotMatch(createRow, /status-mark/);
  assert.doesNotMatch(css, /\.status-mark\b/);
  assert.match(
    css,
    /\.workspace-row\s*\{[^}]*grid-template-columns\s*:\s*minmax\(160px,\s*0\.7fr\)\s+minmax\(300px,\s*1\.3fr\)/i,
  );
  assert.match(
    css,
    /\.provider-state\s*\{[^}]*grid-template-columns\s*:\s*112px\s+minmax\(0,\s*1fr\)/i,
  );
  const providerName = css.match(/\.provider-name\s*\{([^}]*)\}/i)?.[1];
  assert.ok(providerName, "provider names should have dedicated styling");
  assert.match(providerName, /text-overflow\s*:\s*ellipsis/i);
});

test("focused workspaces use a compact illuminated indicator beside the title", () => {
  const javascript = read("ui/generated/app.js");
  const css = read("ui/styles.css");
  const createRow = sliceBetween(
    javascript,
    /function\s+createWorkspaceRow\s*\(/,
    /function\s+groupWorkspaces\s*\(/,
    "createWorkspaceRow",
  );

  assert.match(
    createRow,
    /if\s*\(workspace\.focused\)\s*\{\s*const\s+focused\s*=\s*createElement\(\s*["']span["']\s*,\s*["']workspace-focus["']\s*\)/,
  );
  assert.match(createRow, /focused\.setAttribute\(\s*["']aria-hidden["']\s*,\s*["']true["']\s*\)/);
  assert.doesNotMatch(
    createRow,
    /createElement\(\s*["']span["']\s*,\s*["']workspace-focus["']\s*,\s*["']Focused["']\s*\)/,
  );

  const indicatorAppend = createRow.indexOf("titleLine.append(focused)");
  const nameAppend = createRow.indexOf("titleLine.append(name)");
  assert.ok(indicatorAppend >= 0, "the focus indicator should be added to the title line");
  assert.ok(
    indicatorAppend < nameAppend,
    "the focus indicator should sit consistently before the workspace name",
  );

  const focusLight = css.match(/\.workspace-focus\s*\{([^}]*)\}/i)?.[1];
  assert.ok(focusLight, "the focus indicator should have dedicated styling");
  assert.match(focusLight, /width\s*:\s*7px/i);
  assert.match(focusLight, /height\s*:\s*7px/i);
  assert.match(focusLight, /border-radius\s*:\s*50%/i);
  assert.match(focusLight, /background\s*:\s*var\(--accent\)/i);
  assert.match(focusLight, /box-shadow\s*:/i);
  assert.doesNotMatch(focusLight, /font-|letter-spacing|text-transform|padding/i);
});

test("workspace rows render one model-free status for the compact panel", () => {
  const javascript = read("ui/generated/app.js");
  const createRow = sliceBetween(
    javascript,
    /function\s+createWorkspaceRow\s*\(/,
    /function\s+groupWorkspaces\s*\(/,
    "createWorkspaceRow",
  );
  const compactStatus = sliceBetween(
    createRow,
    /const\s+aggregate\s*=\s*aggregateActivity\(workspace\)\s*;/,
    /const\s+providers\s*=/,
    "compact workspace status",
  );

  assert.equal(
    Array.from(createRow.matchAll(/["']workspace-compact-status["']/g)).length,
    1,
    "each workspace row should contain one compact status",
  );
  assert.match(
    compactStatus,
    /createElement\(\s*["']span["']\s*,\s*["']workspace-compact-status["']\s*,\s*aggregate\.label\s*\)/,
  );
  assert.match(compactStatus, /compactStatus\.dataset\.state\s*=\s*aggregate\.kind/);
  assert.match(compactStatus, /row\.append\(compactStatus\)/);
  assert.doesNotMatch(compactStatus, /modelKind|modelLabel|providerName/i);
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
  assert.match(javascript, /"Zed local metadata"/);
  assert.match(javascript, /raw\.validZedWorkspaceRecords/);
  assert.match(javascript, /raw\.ambiguousZedLiveChannels/);
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
    /function\s+groupWorkspaces\s*\(/,
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
  assert.match(createRow, /workspace\.focused \? ", currently focused" : ""/);
  assert.match(
    createRow,
    /openable[\s\S]*?`\$\{actionLabel\} \$\{workspace\.name\} in \$\{workspace\.editorName\}\$\{focusContext\}`[\s\S]*?cannot currently be opened/,
  );
  assert.match(
    createRow,
    /openButton\.title\s*=\s*workspace\.surface\s*===\s*["']cursor_agent_thread["'][\s\S]*workspace\.recentlyActive/,
  );
  assert.match(createRow, /does not provide a safe window activation target/);
  assert.match(createRow, /hook activity does not identify a live window or exact open target/);
  assert.match(
    createRow,
    /workspace\.surface\s*===\s*["']cursor_agent_thread["'][\s\S]*["']Cursor desktop bridge["']/,
  );
  assert.match(
    createRow,
    /workspace\.surface\s*!==\s*["']cursor_agent_thread["'][\s\S]*createProviderState\(\s*["']Codex["']/,
    "Cursor agent-thread rows should omit unrelated Codex and Claude cards",
  );

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
  assert.match(openWorkspace, /open_editor_targets_with\s*\(/);
  assert.match(openWorkspace, /enter_floating_panel\s*\(/);
  assert.match(openWorkspace, /find_active_workspace_open_target(?:_with_zed)?\s*\(/);
  assert.match(openWorkspace, /WorkspaceLaunchMode::PreferExisting/);
  assert.match(openWorkspace, /WorkspaceLaunchMode::NewWindow/);
  assert.match(library, /wait_for_restored_window_state\s*\(/);
  assert.match(library, /wait_for_floating_panel_ready\s*\(/);
  assert.match(library, /set_visible_on_all_workspaces\(true\)/);
  assert.match(openWorkspace, /schedule_floating_panel_watchdog\s*\(/);
  assert.ok(
    openWorkspace.indexOf("enter_floating_panel")
      < openWorkspace.indexOf("open_editor_targets_with"),
    "the native panel must be ready before the selected editor can switch desktops",
  );
  assert.doesNotMatch(openWorkspace, /\.minimize\s*\(/);
  assert.doesNotMatch(trayWorkspace, /\.minimize\s*\(/);
  assert.match(hideWindow, /\.minimize\s*\(/);
});
