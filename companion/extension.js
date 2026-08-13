"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const crypto = require("node:crypto");

const SCHEMA_VERSION = 1;
const HEARTBEAT_INTERVAL_MS = 3_000;
const EDITOR_KINDS = Object.freeze({
  vscode: "vscode",
  cursor: "cursor",
  antigravityIde: "antigravity_ide",
});
const AGENT_EXTENSION_IDS = Object.freeze({
  codex: "openai.chatgpt",
  claude: "anthropic.claude-code",
});

let activeReporter = null;

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
    ? value
    : null;
}

function environmentString(environment, property) {
  try {
    return nonEmptyString(environment && environment[property]);
  } catch {
    return null;
  }
}

function detectEditor(vscode) {
  let environment = null;
  try {
    environment = vscode && vscode.env;
  } catch {
    // Older or partially compatible hosts may not expose `env`. They retain the
    // historical VS Code classification so existing installations keep working.
  }

  const uriScheme = environmentString(environment, "uriScheme")?.toLowerCase();
  if (uriScheme === "cursor") {
    return EDITOR_KINDS.cursor;
  }
  if (uriScheme === "antigravity" || uriScheme === "antigravity-ide") {
    return EDITOR_KINDS.antigravityIde;
  }
  if (uriScheme === "vscode" || uriScheme === "vscode-insiders") {
    return EDITOR_KINDS.vscode;
  }

  const appName = environmentString(environment, "appName")?.toLowerCase();
  if (appName === "cursor") {
    return EDITOR_KINDS.cursor;
  }
  if (appName === "antigravity" || appName === "antigravity ide") {
    return EDITOR_KINDS.antigravityIde;
  }

  return EDITOR_KINDS.vscode;
}

function resolveStateDirectory(options = {}) {
  const env = options.env || process.env;
  const platform = options.platform || process.platform;
  const homeDirectory = options.homeDirectory || os.homedir();
  const pathApi =
    options.pathApi || (platform === "win32" ? path.win32 : path.posix);

  const override = nonEmptyString(env.VSPARALLEL_STATE_DIR);
  if (override) {
    if (!pathApi.isAbsolute(override)) {
      throw new Error("VSPARALLEL_STATE_DIR must be an absolute path");
    }
    return override;
  }

  if (platform === "win32") {
    const baseDirectory =
      nonEmptyString(env.LOCALAPPDATA) ||
      nonEmptyString(env.APPDATA) ||
      pathApi.join(
        nonEmptyString(env.USERPROFILE) || homeDirectory,
        "AppData",
        "Local",
      );
    return pathApi.join(baseDirectory, "VSParallel");
  }

  if (platform === "darwin") {
    return pathApi.join(
      nonEmptyString(env.HOME) || homeDirectory,
      "Library",
      "Application Support",
      "VSParallel",
    );
  }

  const stateHome = nonEmptyString(env.XDG_STATE_HOME);
  return stateHome
    ? pathApi.join(stateHome, "vsparallel")
    : pathApi.join(
        nonEmptyString(env.HOME) || homeDirectory,
        ".local",
        "state",
        "vsparallel",
      );
}

function safeInstanceFileName(instanceId) {
  const original = String(instanceId || "instance");
  const candidate = original.replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 96);

  if (candidate && candidate !== "." && candidate !== ".." && candidate === original) {
    return candidate;
  }

  const digest = crypto.createHash("sha256").update(original).digest("hex").slice(0, 12);
  const prefix = candidate && candidate !== "." && candidate !== ".."
    ? candidate
    : "instance";
  return `${prefix}-${digest}`;
}

function recordPathFor(stateDirectory, instanceId) {
  return path.join(
    stateDirectory,
    "instances",
    `${safeInstanceFileName(instanceId)}.json`,
  );
}

function localPathForUri(uri) {
  try {
    if (!uri || uri.scheme !== "file") {
      return null;
    }
    return nonEmptyString(uri.fsPath);
  } catch {
    return null;
  }
}

function folderMetadata(folder, fallbackIndex) {
  if (!folder || !folder.uri) {
    return null;
  }

  const index = Number.isInteger(folder.index) ? folder.index : fallbackIndex;
  const localPath = localPathForUri(folder.uri);
  const derivedName = localPath ? path.basename(localPath) : `Folder ${index + 1}`;

  return {
    name: nonEmptyString(folder.name) || derivedName,
    index,
    path: localPath,
  };
}

function workspaceFileMetadata(uri) {
  if (!uri) {
    return null;
  }

  return {
    path: localPathForUri(uri),
  };
}

function extensionRunsRemote(vscode, extension) {
  const remoteWindow = Boolean(
    nonEmptyString(vscode && vscode.env && vscode.env.remoteName),
  );
  if (!remoteWindow) {
    return false;
  }

  const extensionKind = extension && extension.extensionKind;
  const kinds = vscode && vscode.ExtensionKind;
  if (!kinds || extensionKind === undefined || extensionKind === null) {
    return null;
  }
  if (extensionKind === kinds.Workspace) {
    return true;
  }
  if (extensionKind === kinds.UI) {
    return false;
  }
  return null;
}

function extensionPresence(vscode, extensionId) {
  try {
    const extensions = vscode && vscode.extensions;
    if (!extensions || typeof extensions.getExtension !== "function") {
      return { available: false, installed: false, active: false, remote: null };
    }

    const extension = extensions.getExtension(extensionId);
    if (!extension) {
      return { available: true, installed: false, active: false, remote: null };
    }

    return {
      available: true,
      installed: true,
      active: Boolean(extension.isActive),
      remote: extensionRunsRemote(vscode, extension),
    };
  } catch {
    // `available` distinguishes an API failure from a confirmed absence. The
    // status fields remain present to keep the on-disk shape predictable.
    return { available: false, installed: false, active: false, remote: null };
  }
}

function detectAgentExtensions(vscode) {
  return {
    codex: extensionPresence(vscode, AGENT_EXTENSION_IDS.codex),
    claude: extensionPresence(vscode, AGENT_EXTENSION_IDS.claude),
  };
}

function exactOpenTarget(workspaceFileUri, workspaceFile, folders) {
  // A saved workspace file is the authoritative target even if it currently has
  // one folder. Untitled workspaces do not have a stable target that another
  // VS Code process can reopen exactly.
  if (workspaceFileUri) {
    return workspaceFile && workspaceFile.path;
  }

  // With no workspace file, only a one-folder window has an exact target.
  if (folders.length === 1) {
    return folders[0].path;
  }

  return null;
}

function fallbackWorkspaceName(workspaceName, workspaceFile, folders) {
  const explicitName = nonEmptyString(workspaceName);
  if (explicitName) {
    return explicitName;
  }

  if (folders.length === 1) {
    return folders[0].name;
  }

  if (workspaceFile && workspaceFile.path) {
    const fileName = path.basename(workspaceFile.path);
    return fileName.endsWith(".code-workspace")
      ? fileName.slice(0, -".code-workspace".length)
      : fileName;
  }

  return folders.length > 1 ? "Untitled Workspace" : "Empty Window";
}

function createHeartbeatRecord(vscode, options) {
  const nowMs = options.nowMs;
  const workspaceFolders = Array.isArray(vscode.workspace.workspaceFolders)
    ? vscode.workspace.workspaceFolders
        .map((folder, index) => folderMetadata(folder, index))
        .filter(Boolean)
    : [];
  const workspaceFileUri = vscode.workspace.workspaceFile || null;
  const workspaceFile = workspaceFileMetadata(workspaceFileUri);
  const windowState = options.windowState || vscode.window.state || {};
  const focused = Boolean(windowState.focused);
  const active =
    typeof windowState.active === "boolean"
      ? windowState.active
      : focused;

  return {
    schemaVersion: SCHEMA_VERSION,
    instanceId: options.instanceId,
    editor: detectEditor(vscode),
    workspaceName: fallbackWorkspaceName(
      vscode.workspace.name,
      workspaceFile,
      workspaceFolders,
    ),
    workspaceFolders,
    workspaceFile,
    primaryPath:
      workspaceFolders.find((folder) => folder.path !== null)?.path || null,
    openTarget: exactOpenTarget(
      workspaceFileUri,
      workspaceFile,
      workspaceFolders,
    ),
    focused,
    active,
    remoteWindow: Boolean(
      nonEmptyString(vscode && vscode.env && vscode.env.remoteName),
    ),
    agentExtensions: detectAgentExtensions(vscode),
    lastSeenAtMs: nowMs,
    startedAtMs: options.startedAtMs,
  };
}

async function atomicWriteJson(filePath, value, fileSystem = fs.promises) {
  const directory = path.dirname(filePath);
  await fileSystem.mkdir(directory, { recursive: true, mode: 0o700 });

  const temporaryPath = path.join(
    directory,
    `.${path.basename(filePath)}.${process.pid}.${crypto.randomBytes(6).toString("hex")}.tmp`,
  );
  let handle = null;

  try {
    handle = await fileSystem.open(temporaryPath, "wx", 0o600);
    await handle.writeFile(`${JSON.stringify(value)}\n`, "utf8");
    if (typeof handle.sync === "function") {
      await handle.sync();
    }
    await handle.close();
    handle = null;
    await fileSystem.rename(temporaryPath, filePath);
  } catch (error) {
    if (handle) {
      try {
        await handle.close();
      } catch {
        // The original write error is more useful.
      }
    }
    try {
      await fileSystem.unlink(temporaryPath);
    } catch {
      // A failed or interrupted write may already have removed the temp file.
    }
    throw error;
  }
}

async function removeRecord(filePath, fileSystem = fs.promises) {
  try {
    await fileSystem.unlink(filePath);
  } catch (error) {
    if (!error || error.code !== "ENOENT") {
      throw error;
    }
  }
}

function randomInstanceId() {
  return typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : crypto.randomBytes(16).toString("hex");
}

class HeartbeatReporter {
  constructor(vscode, options = {}) {
    this.vscode = vscode;
    this.stateDirectory =
      options.stateDirectory || resolveStateDirectory();
    this.instanceId =
      nonEmptyString(options.instanceId) || randomInstanceId();
    this.startedAtMs = options.startedAtMs || Date.now();
    this.now = options.now || Date.now;
    this.intervalMs = options.intervalMs || HEARTBEAT_INTERVAL_MS;
    this.fileSystem = options.fileSystem || fs.promises;
    this.logError = options.logError || ((error) => {
      const message = error instanceof Error ? error.message : String(error);
      console.warn(`[VSParallel] ${message}`);
    });
    this.filePath = recordPathFor(this.stateDirectory, this.instanceId);
    this.windowState = vscode.window.state || {};
    this.queue = Promise.resolve();
    this.publishInFlight = false;
    this.publishDirty = false;
    this.timer = null;
    this.disposables = [];
    this.stopped = false;
  }

  publish() {
    if (this.stopped) {
      return this.queue;
    }

    this.publishDirty = true;
    if (this.publishInFlight) {
      return this.queue;
    }

    this.publishInFlight = true;
    this.queue = this.drainPublishes();
    return this.queue;
  }

  async drainPublishes() {
    try {
      while (this.publishDirty && !this.stopped) {
        this.publishDirty = false;
        try {
          this.windowState = this.vscode.window.state || this.windowState;
          const record = createHeartbeatRecord(this.vscode, {
            instanceId: this.instanceId,
            nowMs: this.now(),
            startedAtMs: this.startedAtMs,
            windowState: this.windowState,
          });
          await atomicWriteJson(this.filePath, record, this.fileSystem);
        } catch (error) {
          this.logError(error);
        }
      }
    } finally {
      this.publishInFlight = false;
    }
  }

  async start(context) {
    if (typeof this.vscode.window.onDidChangeWindowState === "function") {
      const disposable = this.vscode.window.onDidChangeWindowState((state) => {
        this.windowState = state || this.windowState;
        void this.publish();
      });
      this.disposables.push(disposable);
    }

    if (typeof this.vscode.workspace.onDidChangeWorkspaceFolders === "function") {
      const disposable = this.vscode.workspace.onDidChangeWorkspaceFolders(() => {
        void this.publish();
      });
      this.disposables.push(disposable);
    }

    if (context && Array.isArray(context.subscriptions)) {
      context.subscriptions.push(...this.disposables);
    }

    this.timer = setInterval(() => {
      void this.publish();
    }, this.intervalMs);
    if (typeof this.timer.unref === "function") {
      this.timer.unref();
    }

    await this.publish();
  }

  async stop() {
    if (this.stopped) {
      return;
    }

    this.stopped = true;
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
    for (const disposable of this.disposables.splice(0)) {
      try {
        disposable.dispose();
      } catch {
        // VS Code may already have disposed extension subscriptions.
      }
    }

    await this.queue;
    try {
      await removeRecord(this.filePath, this.fileSystem);
    } catch (error) {
      this.logError(error);
    }
  }
}

async function activate(context) {
  const vscode = require("vscode");

  if (activeReporter) {
    await activeReporter.stop();
  }

  activeReporter = new HeartbeatReporter(vscode);
  await activeReporter.start(context);
}

async function deactivate() {
  const reporter = activeReporter;
  activeReporter = null;
  if (reporter) {
    await reporter.stop();
  }
}

module.exports = {
  activate,
  deactivate,
};
