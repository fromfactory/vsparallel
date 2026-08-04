"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

function loadTestInterface() {
  const extensionPath = path.resolve(__dirname, "../extension.js");
  const source = fsSync.readFileSync(extensionPath, "utf8");
  const testModule = { exports: {} };
  const exposeTestInterface = `
    module.exports.__test = {
      AGENT_EXTENSION_IDS,
      SCHEMA_VERSION,
      HEARTBEAT_INTERVAL_MS,
      HeartbeatReporter,
      atomicWriteJson,
      createHeartbeatRecord,
      detectAgentExtensions,
      exactOpenTarget,
      recordPathFor,
      removeRecord,
      resolveStateDirectory,
      safeInstanceFileName,
    };
  `;
  const compile = new Function(
    "require",
    "module",
    "exports",
    "__filename",
    "__dirname",
    `${source}\n${exposeTestInterface}`,
  );
  compile(require, testModule, testModule.exports, extensionPath, path.dirname(extensionPath));
  return testModule.exports.__test;
}

const {
  AGENT_EXTENSION_IDS,
  SCHEMA_VERSION,
  HeartbeatReporter,
  atomicWriteJson,
  createHeartbeatRecord,
  recordPathFor,
  resolveStateDirectory,
  safeInstanceFileName,
} = loadTestInterface();
const manifest = require("../package.json");

function fileUri(filePath) {
  const normalized = filePath.replaceAll("\\", "/");
  return {
    scheme: "file",
    fsPath: filePath,
    toString() {
      return `file://${normalized.startsWith("/") ? "" : "/"}${normalized}`;
    },
  };
}

function otherUri(scheme, serialized) {
  return {
    scheme,
    fsPath: "/must/not/be/reported",
    toString() {
      return serialized;
    },
  };
}

function fakeVscode(overrides = {}) {
  return {
    version: "1.95.2",
    env: {
      sessionId: "session-1",
      remoteName: undefined,
      ...overrides.env,
    },
    window: {
      state: { focused: true, active: true },
      ...overrides.window,
    },
    workspace: {
      name: undefined,
      workspaceFolders: undefined,
      workspaceFile: undefined,
      ...overrides.workspace,
    },
    extensions: overrides.extensions === undefined
      ? { getExtension: () => undefined }
      : overrides.extensions,
  };
}

function record(vscode, options = {}) {
  return createHeartbeatRecord(vscode, {
    instanceId: "session-1",
    nowMs: 2_000,
    startedAtMs: 1_000,
    ...options,
  });
}

test("manifest activates as a dependency-free local UI extension", () => {
  assert.equal(manifest.main, "./extension.js");
  assert.deepEqual(manifest.activationEvents, ["onStartupFinished"]);
  assert.deepEqual(manifest.extensionKind, ["ui"]);
  assert.equal(manifest.dependencies, undefined);
  assert.equal(manifest.devDependencies, undefined);
});

test("each extension-host activation gets an opaque collision-resistant identity", () => {
  const first = new HeartbeatReporter(fakeVscode(), {
    stateDirectory: "/tmp/vsparallel-window-id-test",
  });
  const second = new HeartbeatReporter(fakeVscode(), {
    stateDirectory: "/tmp/vsparallel-window-id-test",
  });

  assert.match(first.instanceId, /^[0-9a-f-]{32,36}$/);
  assert.match(second.instanceId, /^[0-9a-f-]{32,36}$/);
  assert.notEqual(first.instanceId, second.instanceId);
  assert.equal(first.instanceId.includes("session-1"), false);
});

test("resolves one shared state root on Linux, macOS, and Windows", () => {
  assert.equal(
    resolveStateDirectory({
      env: { VSPARALLEL_STATE_DIR: "/tmp/custom-state" },
      platform: "linux",
      homeDirectory: "/home/tester",
    }),
    "/tmp/custom-state",
  );
  assert.equal(
    resolveStateDirectory({
      env: { XDG_STATE_HOME: "/var/state/tester" },
      platform: "linux",
      homeDirectory: "/home/tester",
    }),
    "/var/state/tester/vsparallel",
  );
  assert.equal(
    resolveStateDirectory({
      env: {},
      platform: "linux",
      homeDirectory: "/home/tester",
    }),
    "/home/tester/.local/state/vsparallel",
  );
  assert.equal(
    resolveStateDirectory({
      env: {},
      platform: "darwin",
      homeDirectory: "/Users/tester",
    }),
    "/Users/tester/Library/Application Support/VSParallel",
  );
  assert.equal(
    resolveStateDirectory({
      env: { LOCALAPPDATA: "C:\\Users\\tester\\AppData\\Local" },
      platform: "win32",
      homeDirectory: "C:\\Users\\tester",
    }),
    "C:\\Users\\tester\\AppData\\Local\\VSParallel",
  );
});

test("rejects a relative state-directory override instead of using VS Code cwd", () => {
  assert.throws(
    () => resolveStateDirectory({
      env: { VSPARALLEL_STATE_DIR: "relative/state" },
      platform: "linux",
      homeDirectory: "/home/tester",
    }),
    /VSPARALLEL_STATE_DIR must be an absolute path/,
  );
  assert.throws(
    () => resolveStateDirectory({
      env: { VSPARALLEL_STATE_DIR: "relative\\state" },
      platform: "win32",
      homeDirectory: "C:\\Users\\tester",
    }),
    /VSPARALLEL_STATE_DIR must be an absolute path/,
  );
});

test("record names cannot escape the instances directory", () => {
  assert.equal(safeInstanceFileName("normal-id_1.2"), "normal-id_1.2");
  assert.match(safeInstanceFileName("../../outside"), /^\.\._\.\._outside-/);

  const stateDirectory = path.join(path.sep, "state");
  const filePath = recordPathFor(stateDirectory, "../../outside");
  assert.equal(path.dirname(filePath), path.join(stateDirectory, "instances"));
  assert.equal(path.extname(filePath), ".json");
});

test("a single local folder has an exact folder open target", () => {
  const vscode = fakeVscode({
    workspace: {
      name: "example-workspace",
      workspaceFolders: [
        {
          name: "example-workspace",
          index: 0,
          uri: fileUri("/work/example-workspace"),
        },
      ],
    },
  });

  assert.deepEqual(record(vscode), {
    schemaVersion: SCHEMA_VERSION,
    instanceId: "session-1",
    workspaceName: "example-workspace",
    workspaceFolders: [
      {
        name: "example-workspace",
        index: 0,
        path: "/work/example-workspace",
      },
    ],
    workspaceFile: null,
    primaryPath: "/work/example-workspace",
    openTarget: "/work/example-workspace",
    focused: true,
    active: true,
    agentExtensions: {
      codex: { available: true, installed: false, active: false },
      claude: { available: true, installed: false, active: false },
    },
    lastSeenAtMs: 2_000,
    startedAtMs: 1_000,
  });
});

test("reports installed active and inactive agent extensions without activating them", () => {
  const lookups = [];
  const heartbeat = record(fakeVscode({
    extensions: {
      getExtension(extensionId) {
        lookups.push(extensionId);
        if (extensionId === AGENT_EXTENSION_IDS.codex) {
          return { id: extensionId, isActive: true };
        }
        if (extensionId === AGENT_EXTENSION_IDS.claude) {
          return { id: extensionId, isActive: false };
        }
        return undefined;
      },
    },
  }));

  assert.deepEqual(lookups, [
    "openai.chatgpt",
    "anthropic.claude-code",
  ]);
  assert.deepEqual(heartbeat.agentExtensions, {
    codex: { available: true, installed: true, active: true },
    claude: { available: true, installed: true, active: false },
  });
});

test("reports a confirmed absent extension separately from unavailable detection", () => {
  const heartbeat = record(fakeVscode({
    extensions: { getExtension: () => undefined },
  }));

  assert.deepEqual(heartbeat.agentExtensions, {
    codex: { available: true, installed: false, active: false },
    claude: { available: true, installed: false, active: false },
  });
});

test("contains an extension API failure to the affected provider", () => {
  const heartbeat = record(fakeVscode({
    extensions: {
      getExtension(extensionId) {
        if (extensionId === AGENT_EXTENSION_IDS.codex) {
          throw new Error("extension registry unavailable");
        }
        return undefined;
      },
    },
  }));

  assert.deepEqual(heartbeat.agentExtensions, {
    codex: { available: false, installed: false, active: false },
    claude: { available: true, installed: false, active: false },
  });
});

test("reports both providers unavailable when the extension API is unavailable", () => {
  const unavailable = record({
    ...fakeVscode(),
    extensions: null,
  });
  assert.deepEqual(unavailable.agentExtensions, {
    codex: { available: false, installed: false, active: false },
    claude: { available: false, installed: false, active: false },
  });
});

test("a saved multi-root window opens its workspace file, not one folder", () => {
  const vscode = fakeVscode({
    env: { remoteName: "ssh-remote" },
    workspace: {
      name: "Example suite",
      workspaceFolders: [
        { name: "api", index: 0, uri: fileUri("/work/example-suite/api") },
        { name: "web", index: 1, uri: fileUri("/work/example-suite/web") },
      ],
      workspaceFile: fileUri(
        "/work/example-suite/example-suite.code-workspace",
      ),
    },
    window: {
      state: { focused: false, active: true },
    },
  });

  const heartbeat = record(vscode);
  assert.equal(
    heartbeat.openTarget,
    "/work/example-suite/example-suite.code-workspace",
  );
  assert.equal(heartbeat.primaryPath, "/work/example-suite/api");
  assert.equal(heartbeat.workspaceFolders.length, 2);
  assert.equal(heartbeat.focused, false);
  assert.equal(heartbeat.active, true);
  assert.equal("remoteName" in heartbeat, false);
  assert.equal("vscodeVersion" in heartbeat, false);
});

test("untitled and inaccessible workspace state remains safe and non-openable", () => {
  const brokenUri = {
    scheme: "file",
    get fsPath() {
      throw new Error("bad URI");
    },
    toString() {
      throw new Error("bad URI");
    },
  };
  const vscode = fakeVscode({
    workspace: {
      name: "Scratch set",
      workspaceFolders: [
        null,
        { name: "broken", index: 0, uri: brokenUri },
        {
          name: "remote",
          index: 1,
          uri: otherUri("vscode-remote", "vscode-remote://ssh-host/project"),
        },
      ],
      workspaceFile: otherUri("untitled", "untitled:Untitled-1"),
    },
  });

  const heartbeat = record(vscode);
  assert.equal(heartbeat.openTarget, null);
  assert.equal(heartbeat.primaryPath, null);
  assert.deepEqual(heartbeat.workspaceFolders, [
    {
      name: "broken",
      index: 0,
      path: null,
    },
    {
      name: "remote",
      index: 1,
      path: null,
    },
  ]);
  assert.deepEqual(heartbeat.workspaceFile, {
    path: null,
  });
});

test("a virtual single-folder window omits remote identifiers and has no open target", () => {
  const vscode = fakeVscode({
    workspace: {
      workspaceFolders: [
        {
          name: "remote-project",
          index: 0,
          uri: otherUri(
            "vscode-remote",
            "vscode-remote://ssh-remote+dev/home/me/project",
          ),
        },
      ],
    },
    window: {
      state: { focused: true },
    },
  });

  const heartbeat = record(vscode);
  assert.equal(heartbeat.openTarget, null);
  assert.equal(heartbeat.primaryPath, null);
  assert.equal(heartbeat.active, true, "focused is the compatibility fallback");
  assert.equal(JSON.stringify(heartbeat).includes("ssh-remote+dev"), false);
});

test("heartbeat serialization excludes prompts, answers, source, and terminal data", () => {
  const vscode = fakeVscode({
    env: {
      prompt: "SECRET PROMPT",
      machineId: "PRIVATE MACHINE ID",
    },
    workspace: {
      name: "example-idle",
      workspaceFolders: [
        {
          name: "example-idle",
          index: 0,
          uri: fileUri("/work/example-idle"),
          sourceText: "SECRET SOURCE",
        },
      ],
      transcript: "SECRET TRANSCRIPT",
    },
    window: {
      state: { focused: false, active: false },
      terminalContents: "SECRET TERMINAL",
      codexAnswer: "SECRET ANSWER",
    },
    extensions: {
      getExtension(extensionId) {
        return {
          id: extensionId,
          isActive: true,
          exports: {
            prompt: "SECRET EXTENSION PROMPT",
            transcript: "SECRET EXTENSION TRANSCRIPT",
          },
        };
      },
    },
  });

  const serialized = JSON.stringify(record(vscode));
  for (const forbidden of [
    "SECRET PROMPT",
    "PRIVATE MACHINE ID",
    "SECRET SOURCE",
    "SECRET TRANSCRIPT",
    "SECRET TERMINAL",
    "SECRET ANSWER",
    "SECRET EXTENSION PROMPT",
    "SECRET EXTENSION TRANSCRIPT",
  ]) {
    assert.equal(serialized.includes(forbidden), false);
  }
});

test("atomic writes replace complete JSON and leave no temporary files", async (t) => {
  const temporaryDirectory = await fs.mkdtemp(
    path.join(os.tmpdir(), "vsparallel-companion-atomic-"),
  );
  t.after(() => fs.rm(temporaryDirectory, { recursive: true, force: true }));
  const filePath = path.join(temporaryDirectory, "instances", "one.json");

  await atomicWriteJson(filePath, { schemaVersion: 1, value: "first" });
  await atomicWriteJson(filePath, { schemaVersion: 1, value: "second" });

  assert.deepEqual(JSON.parse(await fs.readFile(filePath, "utf8")), {
    schemaVersion: 1,
    value: "second",
  });
  assert.deepEqual(await fs.readdir(path.dirname(filePath)), ["one.json"]);
});

test("reporter coalesces a publish burst into one in-flight write and one rerun", async () => {
  let releaseFirstWrite;
  let markFirstWriteStarted;
  const firstWriteStarted = new Promise((resolve) => {
    markFirstWriteStarted = resolve;
  });
  const firstWriteBlocked = new Promise((resolve) => {
    releaseFirstWrite = resolve;
  });
  const writes = [];
  const fileSystem = {
    async mkdir() {},
    async open() {
      return {
        async writeFile(serialized) {
          writes.push(JSON.parse(serialized));
          if (writes.length === 1) {
            markFirstWriteStarted();
            await firstWriteBlocked;
          }
        },
        async sync() {},
        async close() {},
      };
    },
    async rename() {},
    async unlink() {},
  };
  const vscode = fakeVscode({
    workspace: {
      name: "initial",
      workspaceFolders: [
        { name: "initial", index: 0, uri: fileUri("/work/initial") },
      ],
    },
  });
  let clock = 20_000;
  const reporter = new HeartbeatReporter(vscode, {
    stateDirectory: "/state",
    instanceId: "coalesced-session",
    startedAtMs: 19_000,
    now: () => ++clock,
    fileSystem,
  });

  const firstPublish = reporter.publish();
  await firstWriteStarted;
  const queuedPublishes = [];
  for (let index = 0; index < 50; index += 1) {
    vscode.workspace.name = `workspace-${index}`;
    vscode.workspace.workspaceFolders = [
      {
        name: `workspace-${index}`,
        index: 0,
        uri: fileUri(`/work/workspace-${index}`),
      },
    ];
    queuedPublishes.push(reporter.publish());
  }

  assert.equal(writes.length, 1, "no second write may overlap the blocked write");
  assert.ok(queuedPublishes.every((publish) => publish === firstPublish));
  releaseFirstWrite();
  await firstPublish;

  assert.equal(writes.length, 2, "the burst should cause exactly one dirty rerun");
  assert.equal(writes[1].workspaceName, "workspace-49");
  assert.equal(writes[1].openTarget, "/work/workspace-49");
  await reporter.stop();
});

test("reporter refreshes on focus/workspace events and removes its record", async (t) => {
  const temporaryDirectory = await fs.mkdtemp(
    path.join(os.tmpdir(), "vsparallel-companion-reporter-"),
  );
  t.after(() => fs.rm(temporaryDirectory, { recursive: true, force: true }));

  const listeners = {};
  let disposedCount = 0;
  const vscode = fakeVscode({
    workspace: {
      name: "first",
      workspaceFolders: [
        { name: "first", index: 0, uri: fileUri("/work/first") },
      ],
      onDidChangeWorkspaceFolders(callback) {
        listeners.workspace = callback;
        return { dispose: () => { disposedCount += 1; } };
      },
    },
    window: {
      state: { focused: true, active: true },
      onDidChangeWindowState(callback) {
        listeners.window = callback;
        return { dispose: () => { disposedCount += 1; } };
      },
    },
  });
  let clock = 10_000;
  const reporter = new HeartbeatReporter(vscode, {
    stateDirectory: temporaryDirectory,
    instanceId: "event-session",
    startedAtMs: 9_000,
    now: () => ++clock,
    intervalMs: 60_000,
  });
  const context = { subscriptions: [] };

  await reporter.start(context);
  assert.equal(context.subscriptions.length, 2);
  let heartbeat = JSON.parse(await fs.readFile(reporter.filePath, "utf8"));
  assert.equal(heartbeat.focused, true);
  assert.equal(heartbeat.openTarget, "/work/first");

  vscode.window.state = { focused: false, active: true };
  listeners.window(vscode.window.state);
  await reporter.queue;
  heartbeat = JSON.parse(await fs.readFile(reporter.filePath, "utf8"));
  assert.equal(heartbeat.focused, false);

  vscode.workspace.name = "two projects";
  vscode.workspace.workspaceFolders = [
    { name: "first", index: 0, uri: fileUri("/work/first") },
    { name: "second", index: 1, uri: fileUri("/work/second") },
  ];
  listeners.workspace({ added: [vscode.workspace.workspaceFolders[1]], removed: [] });
  await reporter.queue;
  heartbeat = JSON.parse(await fs.readFile(reporter.filePath, "utf8"));
  assert.equal(heartbeat.workspaceName, "two projects");
  assert.equal(heartbeat.workspaceFolders.length, 2);
  assert.equal(heartbeat.openTarget, null);

  await reporter.stop();
  assert.equal(disposedCount, 2);
  await assert.rejects(fs.stat(reporter.filePath), { code: "ENOENT" });
});
