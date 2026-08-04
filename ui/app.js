(function () {
  "use strict";

  const SCHEMA_VERSION = 1;
  const REFRESH_INTERVAL_MS = 3_000;
  const LAUNCH_TRANSITION_MIN_MS = 750;
  const THEME_STORAGE_KEY = "vsparallel.appearance";
  const THEME_PREFERENCES = new Set(["system", "light", "dark"]);
  const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000;
  const tauriInvoke = window.__TAURI__?.core?.invoke;
  const lightThemeQuery = window.matchMedia("(prefers-color-scheme: light)");

  const elements = {
    connectionBar: document.querySelector("#connectionBar"),
    connectionText: document.querySelector("#connectionText"),
    updatedAt: document.querySelector("#updatedAt"),
    refreshButton: document.querySelector("#refreshButton"),
    hideButton: document.querySelector("#hideButton"),
    appTitlebar: document.querySelector("#appTitlebar"),
    titlebarDragRegion: document.querySelector("#titlebarDragRegion"),
    windowControls: document.querySelector("#windowControls"),
    maximizeButton: document.querySelector("#maximizeButton"),
    closeButton: document.querySelector("#closeButton"),
    workspaceCount: document.querySelector("#workspaceCount"),
    workspaceList: document.querySelector("#workspaceList"),
    errorBanner: document.querySelector("#errorBanner"),
    errorText: document.querySelector("#errorText"),
    emptyState: document.querySelector("#emptyState"),
    emptyRefreshButton: document.querySelector("#emptyRefreshButton"),
    launchOverlay: document.querySelector("#launchOverlay"),
    launchStatus: document.querySelector("#launchStatus"),
    settingsButton: document.querySelector("#settingsButton"),
    settingsDialog: document.querySelector("#settingsDialog"),
    settingsCloseButton: document.querySelector("#settingsCloseButton"),
    diagnosticsSummary: document.querySelector("#diagnosticsSummary"),
    diagnosticsList: document.querySelector("#diagnosticsList"),
    diagnosticsStatus: document.querySelector("#diagnosticsStatus"),
    diagnosticsRefreshButton: document.querySelector("#diagnosticsRefreshButton"),
    setupAllButton: document.querySelector("#setupAllButton"),
    integrationList: document.querySelector("#integrationList"),
    integrationMessage: document.querySelector("#integrationMessage"),
    companionCard: document.querySelector("#companionCard"),
    companionStatus: document.querySelector("#companionStatus"),
    companionDetail: document.querySelector("#companionDetail"),
    companionMeta: document.querySelector("#companionMeta"),
    companionInstallButton: document.querySelector("#companionInstallButton"),
    companionUninstallButton: document.querySelector("#companionUninstallButton"),
    codexCard: document.querySelector("#codexCard"),
    codexStatus: document.querySelector("#codexStatus"),
    codexDetail: document.querySelector("#codexDetail"),
    codexMeta: document.querySelector("#codexMeta"),
    codexInstallButton: document.querySelector("#codexInstallButton"),
    codexUninstallButton: document.querySelector("#codexUninstallButton"),
    codexTrustGuidance: document.querySelector("#codexTrustGuidance"),
    claudeCard: document.querySelector("#claudeCard"),
    claudeStatus: document.querySelector("#claudeStatus"),
    claudeDetail: document.querySelector("#claudeDetail"),
    claudeMeta: document.querySelector("#claudeMeta"),
    claudeInstallButton: document.querySelector("#claudeInstallButton"),
    claudeUninstallButton: document.querySelector("#claudeUninstallButton"),
    restartNotice: document.querySelector("#restartNotice"),
    uninstallDialog: document.querySelector("#uninstallDialog"),
    uninstallTitle: document.querySelector("#uninstallTitle"),
    uninstallDescription: document.querySelector("#uninstallDescription"),
    uninstallCancelButton: document.querySelector("#uninstallCancelButton"),
    uninstallConfirmButton: document.querySelector("#uninstallConfirmButton"),
    appearanceInputs: Array.from(
      document.querySelectorAll('input[name="appearance"]'),
    ),
  };

  const initialThemePreference = THEME_PREFERENCES.has(
    document.documentElement.dataset.themePreference,
  )
    ? document.documentElement.dataset.themePreference
    : "system";

  const state = {
    refreshPending: false,
    diagnosticsPending: false,
    diagnosticsLoaded: false,
    diagnosticsUnavailable: false,
    diagnosticWarningCount: 0,
    setupRefreshPending: false,
    setupRefreshPromise: null,
    integrationPending: false,
    integrationLoaded: false,
    integrationStatus: null,
    integrationAction: null,
    pendingUninstall: null,
    openingInstanceId: null,
    lastGoodSnapshot: null,
    windowChrome: null,
    windowChromePending: false,
    windowChromeRefreshTimer: null,
    themePreference: initialThemePreference,
  };

  const dialogReturnFocus = new WeakMap();

  function isObject(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value);
  }

  function asString(value, fallback = "") {
    return typeof value === "string" && value.trim() ? value.trim() : fallback;
  }

  function asFiniteNumber(value, fallback = null) {
    return typeof value === "number" && Number.isFinite(value) ? value : fallback;
  }

  function asNonNegativeInteger(value) {
    const number = asFiniteNumber(value, 0);
    return Math.max(0, Math.trunc(number));
  }

  function asTimestamp(value, fallback = null) {
    const number = asFiniteNumber(value);
    return number !== null && number >= 0 && number <= MAX_JAVASCRIPT_TIMESTAMP_MS
      ? number
      : fallback;
  }

  function asNullableBoolean(value) {
    return typeof value === "boolean" ? value : null;
  }

  function parseBridgeValue(value) {
    if (typeof value !== "string") {
      return value;
    }

    try {
      return JSON.parse(value);
    } catch (_error) {
      return value;
    }
  }

  async function invoke(command, args) {
    if (typeof tauriInvoke === "function") {
      return parseBridgeValue(await tauriInvoke(command, args));
    }

    throw new Error("The Tauri bridge is unavailable. Run VSParallel as a desktop app.");
  }

  function normalizeStateToken(value) {
    return asString(value, "unknown")
      .toLowerCase()
      .replace(/[\s-]+/g, "_");
  }

  function normalizeIntegrationComponent(rawValue, kind) {
    const raw = isObject(rawValue) ? rawValue : {};
    const token = normalizeStateToken(raw.state);
    const missingStates = new Set(["missing", "not_installed", "absent", "unconfigured"]);
    const readyStates = new Set([
      "installed",
      "ready",
      "current",
      "configured",
      "healthy",
      "up_to_date",
    ]);
    const warningStates = new Set([
      "outdated",
      "update_available",
      "repair_needed",
      "needs_repair",
      "modified",
      "partial",
    ]);
    const errorStates = new Set(["error", "failed", "unavailable", "unsupported"]);
    const installedVersion = asString(raw.installedVersion);
    let visualState = "warning";

    if (missingStates.has(token)) {
      visualState = "missing";
    } else if (readyStates.has(token)) {
      visualState = "ready";
    } else if (warningStates.has(token)) {
      visualState = "warning";
    } else if (errorStates.has(token)) {
      visualState = "error";
    } else if (installedVersion) {
      visualState = "ready";
    }

    const installed =
      readyStates.has(token) || warningStates.has(token) || Boolean(installedVersion);
    let actionLabel = "Install / repair";
    if (missingStates.has(token)) {
      actionLabel = "Install";
    } else if (token === "outdated" || token === "update_available") {
      actionLabel = "Update";
    } else if (installed || errorStates.has(token)) {
      actionLabel = "Repair";
    }

    const defaultLabel = visualState === "ready"
      ? "Installed"
      : visualState === "missing"
        ? "Not installed"
        : visualState === "error"
          ? "Unavailable"
          : "Needs attention";

    return {
      kind,
      optional: kind !== "companion",
      token,
      visualState,
      installed,
      actionLabel,
      label: asString(raw.label, defaultLabel),
      detail: asString(
        raw.detail,
        kind === "companion"
          ? "VS Code companion status details are unavailable."
          : kind === "codex"
            ? "Codex lifecycle hook status details are unavailable."
            : "Claude Code lifecycle hook status details are unavailable.",
      ),
      installedVersion,
      targetVersion: asString(raw.targetVersion),
      configPath: asString(raw.configPath),
    };
  }

  function normalizeIntegrationStatus(rawValue) {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw)) {
      throw new Error("VSParallel returned invalid integration status.");
    }
    if (raw.schemaVersion !== SCHEMA_VERSION) {
      throw new Error("VSParallel returned an unsupported integration status version.");
    }

    return {
      schemaVersion: raw.schemaVersion,
      companion: normalizeIntegrationComponent(raw.companion, "companion"),
      codex: normalizeIntegrationComponent(raw.codex, "codex"),
      claude: normalizeIntegrationComponent(raw.claude, "claude"),
      requiresRestart: raw.requiresRestart === true,
    };
  }

  function describeActivityState(token) {
    if (token === "activity_detected") {
      return {
        kind: "activity",
        mark: "●",
        label: "Activity detected",
      };
    }

    if (token === "turn_finished") {
      return {
        kind: "finished",
        mark: "✓",
        label: "Turn finished",
      };
    }

    if (["failed_or_interrupted", "failed/interrupted", "failed", "interrupted"].includes(token)) {
      return {
        kind: "failure",
        mark: "!",
        label: "Failed/interrupted",
      };
    }

    return {
      kind: "unknown",
      mark: "?",
      label: "Unknown",
    };
  }

  function normalizeActivityView(rawValue) {
    const raw = isObject(rawValue) ? rawValue : {};
    return {
      ...describeActivityState(normalizeStateToken(raw.state)),
      changedAtMs: asTimestamp(raw.changedAtMs),
      detail: asString(raw.detail),
      extensionDetectionAvailable: asNullableBoolean(raw.extensionDetectionAvailable),
      extensionInstalled: asNullableBoolean(raw.extensionInstalled),
      extensionActive: asNullableBoolean(raw.extensionActive),
    };
  }

  function normalizeWorkspace(raw, index) {
    if (!isObject(raw)) {
      throw new Error(`Workspace record ${index + 1} is not an object.`);
    }

    const instanceId = asString(raw.instanceId);
    const path = asString(raw.path);
    const name = asString(raw.name, deriveName(path) || "Unnamed workspace");

    return {
      instanceId,
      name,
      path,
      openable: raw.openable === true && Boolean(instanceId),
      active: raw.active === true,
      focused: raw.focused === true,
      lastSeenAtMs: asTimestamp(raw.lastSeenAtMs),
      codex: normalizeActivityView(raw.codex),
      claude: normalizeActivityView(raw.claude),
    };
  }

  function normalizeSnapshot(rawValue) {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw)) {
      throw new Error("The local monitor returned an invalid snapshot.");
    }

    if (!Array.isArray(raw.workspaces)) {
      throw new Error("The local monitor snapshot is missing its workspace list.");
    }
    if (raw.schemaVersion !== SCHEMA_VERSION) {
      throw new Error("The local monitor returned an unsupported snapshot version.");
    }

    const workspaces = [];
    let malformedRecords = 0;

    raw.workspaces.forEach((record, index) => {
      try {
        workspaces.push(normalizeWorkspace(record, index));
      } catch (_error) {
        malformedRecords += 1;
      }
    });

    workspaces.sort((left, right) => {
      const focusDifference = Number(right.focused) - Number(left.focused);
      const activeDifference = Number(right.active) - Number(left.active);
      return focusDifference || activeDifference || left.name.localeCompare(right.name);
    });

    return {
      schemaVersion: raw.schemaVersion,
      generatedAtMs: asTimestamp(raw.generatedAtMs, Date.now()),
      malformedRecords,
      workspaces,
    };
  }

  function deriveName(path) {
    if (!path) {
      return "";
    }
    const segments = path.replace(/[\\/]+$/, "").split(/[\\/]/);
    return segments.at(-1) || "";
  }

  function formatShortPath(path) {
    if (!path || path.length <= 54) {
      return path || "Path unavailable";
    }

    const separator = path.includes("\\") ? "\\" : "/";
    const drive = path.match(/^[A-Za-z]:/)?.[0] || "";
    const segments = path.split(/[\\/]/).filter(Boolean);
    const tail = segments.slice(-3).join(separator);
    return drive ? `${drive}${separator}…${separator}${tail}` : `…${separator}${tail}`;
  }

  function formatRelativeTime(timestamp) {
    if (!Number.isFinite(timestamp)) {
      return "Update time unknown";
    }

    const deltaSeconds = Math.round((timestamp - Date.now()) / 1_000);
    const absoluteSeconds = Math.abs(deltaSeconds);
    let value;
    let unit;

    if (absoluteSeconds < 10) {
      return "Updated just now";
    }
    if (absoluteSeconds < 60) {
      value = deltaSeconds;
      unit = "second";
    } else if (absoluteSeconds < 3_600) {
      value = Math.round(deltaSeconds / 60);
      unit = "minute";
    } else if (absoluteSeconds < 86_400) {
      value = Math.round(deltaSeconds / 3_600);
      unit = "hour";
    } else {
      value = Math.round(deltaSeconds / 86_400);
      unit = "day";
    }

    const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "always" }).format(value, unit);
    return `Updated ${relative}`;
  }

  function formatAbsoluteTime(timestamp) {
    if (!Number.isFinite(timestamp)) {
      return "";
    }
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
      second: "2-digit",
    }).format(new Date(timestamp));
  }

  function createElement(tag, className, text) {
    const element = document.createElement(tag);
    if (className) {
      element.className = className;
    }
    if (text !== undefined) {
      element.textContent = text;
    }
    return element;
  }

  function aggregateActivity(workspace) {
    const priority = { activity: 4, failure: 3, finished: 2, unknown: 1 };
    return [workspace.codex, workspace.claude].reduce((current, candidate) =>
      priority[candidate.kind] > priority[current.kind] ? candidate : current,
    );
  }

  function describeExtensionPresence(activity) {
    if (activity.extensionDetectionAvailable === false) {
      return {
        state: "unknown",
        label: "IDE extension status unavailable",
        title: "VS Code extension presence could not be checked. Lifecycle state remains independent.",
      };
    }
    if (activity.extensionInstalled === false && activity.extensionActive === true) {
      return {
        state: "warning",
        label: "IDE extension status inconsistent",
        title: "The extension reports active but not installed. Lifecycle state remains independent.",
      };
    }
    if (activity.extensionInstalled === false) {
      return {
        state: "missing",
        label: "IDE extension not installed",
        title: "The provider IDE extension was not detected in this VS Code window.",
      };
    }
    if (activity.extensionActive === true) {
      return {
        state: "present",
        label: activity.extensionInstalled === true
          ? "IDE extension active"
          : "IDE extension active · install unknown",
        title: "The IDE extension is active in this window. Activation does not mean an agent turn is running.",
      };
    }
    if (activity.extensionInstalled === true && activity.extensionActive === false) {
      return {
        state: "present",
        label: "IDE extension installed · inactive",
        title: "The IDE extension is installed but inactive. This is separate from lifecycle activity.",
      };
    }
    if (activity.extensionInstalled === true) {
      return {
        state: "present",
        label: "IDE extension installed",
        title: "The IDE extension is installed. Its activation state is unavailable.",
      };
    }
    return {
      state: "unknown",
      label: "IDE extension status unknown",
      title: "IDE extension presence and activation are unavailable. Lifecycle state remains independent.",
    };
  }

  function createProviderState(providerName, activity, accessibleProviderName = providerName) {
    const provider = createElement("div", "provider-state");
    provider.dataset.state = activity.kind;

    const name = createElement("span", "provider-name", providerName);
    if (accessibleProviderName !== providerName) {
      name.title = accessibleProviderName;
    }
    const body = createElement("div", "provider-body");
    const stateLine = createElement("div", "provider-state-line");
    const label = createElement("span", "provider-label", activity.label);
    label.dataset.state = activity.kind;
    if (activity.detail) {
      label.title = activity.detail;
    }

    const relativeTime = Number.isFinite(activity.changedAtMs)
      ? formatRelativeTime(activity.changedAtMs).replace("Updated ", "")
      : "Time unknown";
    const changedAt = createElement("time", "provider-time", relativeTime);
    if (Number.isFinite(activity.changedAtMs)) {
      changedAt.dateTime = new Date(activity.changedAtMs).toISOString();
      changedAt.title = `Lifecycle marker: ${formatAbsoluteTime(activity.changedAtMs)}`;
    }

    const presence = describeExtensionPresence(activity);
    const extension = createElement("span", "provider-extension", presence.label);
    extension.dataset.state = presence.state;
    extension.title = presence.title;
    stateLine.append(label, changedAt);
    body.append(stateLine, extension);
    provider.append(name, body);
    provider.setAttribute(
      "aria-label",
      `${accessibleProviderName}: ${activity.label}, ${relativeTime}. ${presence.label}.`,
    );
    return provider;
  }

  function createWorkspaceRow(workspace) {
    const row = createElement("li", "workspace-row");
    const opening = state.openingInstanceId === workspace.instanceId;
    const openable = workspace.openable;
    row.classList.toggle("is-focused", workspace.focused);
    row.classList.toggle("is-inactive", !workspace.active);
    row.classList.toggle("is-opening", opening);
    row.dataset.openable = String(openable);

    const aggregate = aggregateActivity(workspace);
    const statusMark = createElement("span", "status-mark", aggregate.mark);
    statusMark.dataset.state = aggregate.kind;
    statusMark.setAttribute("aria-hidden", "true");
    row.append(statusMark);

    const primary = createElement("div", "workspace-primary");
    const titleLine = createElement("div", "workspace-title-line");
    const name = createElement("h3", "workspace-name", workspace.name);
    name.title = workspace.name;

    let windowState = "Unavailable";
    let windowStateToken = "inactive";
    if (workspace.focused) {
      windowState = "Focused";
      windowStateToken = "focused";
    } else if (workspace.active) {
      windowState = "Open";
      windowStateToken = "active";
    }

    const windowBadge = createElement("span", "window-badge", windowState);
    windowBadge.dataset.state = windowStateToken;
    titleLine.append(name, windowBadge);

    const path = createElement("span", "workspace-path", formatShortPath(workspace.path));
    if (workspace.path) {
      path.title = workspace.path;
    }
    primary.append(titleLine, path);
    row.append(primary);

    const providers = createElement("div", "activity-providers");
    providers.setAttribute("aria-label", "Agent lifecycle and IDE extension status");
    providers.append(
      createProviderState("Codex", workspace.codex),
      createProviderState("Claude", workspace.claude, "Claude Code"),
    );
    row.append(providers);

    const openButton = createElement("button", "open-button");
    openButton.type = "button";
    openButton.dataset.instanceId = workspace.instanceId;
    openButton.disabled = !openable;
    openButton.setAttribute("aria-label", `Open ${workspace.name} in VS Code`);
    openButton.setAttribute("aria-busy", String(opening));
    if (!workspace.openable) {
      openButton.title = "This workspace cannot currently be opened";
    } else {
      openButton.title = `Open ${workspace.name} in VS Code`;
    }
    openButton.addEventListener("click", () => openWorkspace(workspace));
    row.append(openButton);

    return row;
  }

  function renderSnapshot(snapshot) {
    const focusedOpenButton = document.activeElement?.closest?.(".open-button");
    const focusedInstanceId = elements.workspaceList.contains(focusedOpenButton)
      ? focusedOpenButton.dataset.instanceId
      : "";
    const fragment = document.createDocumentFragment();
    snapshot.workspaces.forEach((workspace) => {
      fragment.append(createWorkspaceRow(workspace));
    });

    elements.workspaceList.replaceChildren(fragment);
    if (focusedInstanceId) {
      const replacement = Array.from(
        elements.workspaceList.querySelectorAll(".open-button:not(:disabled)"),
      ).find((button) => button.dataset.instanceId === focusedInstanceId);
      replacement?.focus({ preventScroll: true });
    }
    elements.workspaceList.setAttribute("aria-busy", "false");
    const workspaceCountLabel = `${snapshot.workspaces.length} ${snapshot.workspaces.length === 1 ? "workspace" : "workspaces"}`;
    elements.workspaceCount.textContent = workspaceCountLabel;
    elements.workspaceCount.setAttribute(
      "aria-label",
      workspaceCountLabel,
    );
    elements.emptyState.hidden = snapshot.workspaces.length !== 0;
    elements.connectionBar.dataset.state = snapshot.malformedRecords ? "warning" : "connected";
    elements.connectionText.textContent = snapshot.malformedRecords
      ? `Monitoring local state · ${snapshot.malformedRecords} malformed ${snapshot.malformedRecords === 1 ? "record" : "records"} ignored`
      : "Monitoring local workspace state";
    elements.updatedAt.textContent = formatRelativeTime(snapshot.generatedAtMs).replace("Updated ", "");
    elements.updatedAt.dateTime = new Date(snapshot.generatedAtMs).toISOString();
    elements.updatedAt.title = `Snapshot: ${formatAbsoluteTime(snapshot.generatedAtMs)}`;

    if (snapshot.malformedRecords) {
      showNotice(
        `${snapshot.malformedRecords} malformed workspace ${snapshot.malformedRecords === 1 ? "record was" : "records were"} ignored. Other workspaces remain available.`,
        "warning",
      );
    } else {
      clearNotice();
    }
  }

  function showNotice(message, kind = "error") {
    elements.errorText.textContent = message;
    elements.errorBanner.hidden = false;
    elements.errorBanner.classList.toggle("notice--error", kind === "error");
    elements.errorBanner.classList.toggle("notice--warning", kind === "warning");
  }

  function clearNotice() {
    elements.errorBanner.hidden = true;
    elements.errorText.textContent = "";
    elements.errorBanner.classList.add("notice--error");
    elements.errorBanner.classList.remove("notice--warning");
  }

  function readableError(error, fallback) {
    if (error instanceof Error && error.message) {
      return error.message;
    }
    if (typeof error === "string" && error.trim()) {
      return error.trim();
    }
    return fallback;
  }

  function isDialogOpen(dialog) {
    return Boolean(dialog?.open || dialog?.hasAttribute("open"));
  }

  function restoreDialogFocus(dialog) {
    const returnTarget = dialogReturnFocus.get(dialog);
    dialogReturnFocus.delete(dialog);
    if (returnTarget?.isConnected && typeof returnTarget.focus === "function") {
      window.requestAnimationFrame(() => returnTarget.focus());
    }
  }

  function showAccessibleDialog(dialog, initialFocus) {
    if (!dialog || isDialogOpen(dialog)) {
      return false;
    }

    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement) {
      dialogReturnFocus.set(dialog, activeElement);
    }

    dialog.setAttribute("aria-modal", "true");
    try {
      if (typeof dialog.showModal === "function") {
        dialog.showModal();
      } else {
        dialog.setAttribute("role", "dialog");
        dialog.setAttribute("open", "");
      }
    } catch (_error) {
      dialog.setAttribute("role", "dialog");
      dialog.setAttribute("open", "");
    }

    window.requestAnimationFrame(() => {
      if (isDialogOpen(dialog) && typeof initialFocus?.focus === "function") {
        initialFocus.focus();
      }
    });
    return true;
  }

  function closeAccessibleDialog(dialog) {
    if (!dialog || !isDialogOpen(dialog)) {
      return false;
    }

    if (typeof dialog.close === "function") {
      dialog.close();
    } else {
      dialog.removeAttribute("open");
      dialog.dispatchEvent(new Event("close"));
    }
    return true;
  }

  function normalizeWindowChromeState(rawValue) {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw) || raw.schemaVersion !== 1) {
      throw new Error("The desktop window returned an unsupported chrome state.");
    }

    return {
      schemaVersion: 1,
      platform: asString(raw.platform, "unknown").toLowerCase(),
      customControls: raw.customControls === true,
      maximized: raw.maximized === true,
      fullscreen: raw.fullscreen === true,
      focused: raw.focused !== false,
    };
  }

  function fallbackWindowChromeState() {
    const isMac = /Macintosh|Mac OS X/.test(window.navigator.userAgent);
    return {
      schemaVersion: 1,
      platform: isMac ? "macos" : "unknown",
      customControls: Boolean(tauriInvoke) && !isMac,
      maximized: false,
      fullscreen: false,
      focused: document.hasFocus(),
    };
  }

  function renderWindowChromeState(chrome) {
    state.windowChrome = chrome;
    document.documentElement.dataset.windowPlatform = chrome.platform;
    document.documentElement.dataset.windowFocused = String(chrome.focused);
    document.documentElement.dataset.windowMaximized = String(chrome.maximized);
    document.documentElement.dataset.windowFullscreen = String(chrome.fullscreen);

    elements.windowControls.hidden = !chrome.customControls;
    if (chrome.customControls) {
      elements.appTitlebar.setAttribute("data-tauri-drag-region", "deep");
      elements.titlebarDragRegion.setAttribute("data-tauri-drag-region", "deep");
    } else {
      elements.appTitlebar.removeAttribute("data-tauri-drag-region");
      elements.appTitlebar
        .querySelectorAll('[data-tauri-drag-region]:not([data-tauri-drag-region="false"])')
        .forEach((element) => element.removeAttribute("data-tauri-drag-region"));
    }

    const restore = chrome.maximized;
    const maximizeLabel = restore ? "Restore VSParallel" : "Maximize VSParallel";
    elements.maximizeButton.disabled = chrome.fullscreen;
    elements.maximizeButton.setAttribute("aria-label", maximizeLabel);
    elements.maximizeButton.title = chrome.fullscreen
      ? "Exit full screen before changing the window size"
      : maximizeLabel;
    elements.maximizeButton.querySelector(".maximize-icon").hidden = restore;
    elements.maximizeButton.querySelector(".restore-icon").hidden = !restore;
  }

  async function refreshWindowChromeState() {
    if (state.windowChromePending) {
      return;
    }

    state.windowChromePending = true;
    try {
      const raw = await invoke("get_window_chrome_state", {});
      renderWindowChromeState(normalizeWindowChromeState(raw));
    } catch (_error) {
      if (!state.windowChrome) {
        renderWindowChromeState(fallbackWindowChromeState());
      }
    } finally {
      state.windowChromePending = false;
    }
  }

  function scheduleWindowChromeRefresh() {
    if (state.windowChromeRefreshTimer !== null) {
      window.clearTimeout(state.windowChromeRefreshTimer);
    }
    state.windowChromeRefreshTimer = window.setTimeout(() => {
      state.windowChromeRefreshTimer = null;
      refreshWindowChromeState();
    }, 80);
  }

  function resolveColorTheme(preference = state.themePreference) {
    if (preference !== "system") {
      return preference;
    }
    return lightThemeQuery.matches ? "light" : "dark";
  }

  async function syncWindowChromeTheme() {
    const theme = resolveColorTheme();
    document.documentElement.dataset.colorTheme = theme;
    document.documentElement.style.colorScheme = theme;
    if (!tauriInvoke) {
      return;
    }
    try {
      await invoke("set_window_chrome_theme", { theme });
    } catch (_error) {
      // CSS still follows the system theme if the native background cannot be updated.
    }
  }

  function storeThemePreference(preference) {
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, preference);
    } catch (_error) {
      // The selected appearance still applies for this session when storage is unavailable.
    }
  }

  function applyThemePreference(preference, persist = true) {
    const normalizedPreference = THEME_PREFERENCES.has(preference)
      ? preference
      : "system";
    state.themePreference = normalizedPreference;
    document.documentElement.dataset.themePreference = normalizedPreference;
    elements.appearanceInputs.forEach((input) => {
      input.checked = input.value === normalizedPreference;
    });
    if (persist) {
      storeThemePreference(normalizedPreference);
    }
    return syncWindowChromeTheme();
  }

  function handleSystemThemeChange() {
    if (state.themePreference === "system") {
      syncWindowChromeTheme();
    }
  }

  async function refreshSnapshot() {
    if (state.refreshPending) {
      return;
    }

    state.refreshPending = true;
    elements.refreshButton.disabled = true;
    elements.refreshButton.setAttribute("aria-busy", "true");
    elements.refreshButton.classList.add("is-loading");
    if (!state.lastGoodSnapshot) {
      elements.workspaceList.setAttribute("aria-busy", "true");
    }

    try {
      const raw = await invoke("get_snapshot", {});
      const snapshot = normalizeSnapshot(raw);
      state.lastGoodSnapshot = snapshot;
      renderSnapshot(snapshot);
    } catch (error) {
      const message = readableError(error, "Could not refresh the local workspace snapshot.");
      elements.connectionBar.dataset.state = "error";
      elements.connectionText.textContent = state.lastGoodSnapshot
        ? "Refresh failed · showing the last good snapshot"
        : "Local monitor unavailable";
      elements.updatedAt.textContent = "";
      showNotice(message);
      elements.workspaceList.setAttribute("aria-busy", "false");
      if (!state.lastGoodSnapshot) {
        elements.emptyState.hidden = false;
      }
    } finally {
      state.refreshPending = false;
      elements.refreshButton.disabled = false;
      elements.refreshButton.setAttribute("aria-busy", "false");
      elements.refreshButton.classList.remove("is-loading");
    }
  }

  function updateWorkspaceOpeningState(instanceId, opening) {
    const button = Array.from(elements.workspaceList.querySelectorAll(".open-button"))
      .find((candidate) => candidate.dataset.instanceId === instanceId);
    const row = button?.closest(".workspace-row");
    row?.classList.toggle("is-opening", opening);
    button?.setAttribute("aria-busy", String(opening));
  }

  function beginWorkspaceLaunch(workspace) {
    state.openingInstanceId = workspace.instanceId;
    document.documentElement.dataset.workspaceOpening = "true";
    elements.launchStatus.textContent = `Opening ${workspace.name}…`;
    elements.launchOverlay.hidden = false;
    updateWorkspaceOpeningState(workspace.instanceId, true);
  }

  function finishWorkspaceLaunch(instanceId) {
    updateWorkspaceOpeningState(instanceId, false);
    state.openingInstanceId = null;
    delete document.documentElement.dataset.workspaceOpening;
    elements.launchOverlay.hidden = true;
    elements.launchStatus.textContent = "Opening workspace…";
  }

  async function openWorkspace(workspace) {
    if (!workspace.openable || state.openingInstanceId) {
      return;
    }

    beginWorkspaceLaunch(workspace);
    const transitionDelay = new Promise((resolve) => {
      window.setTimeout(resolve, LAUNCH_TRANSITION_MIN_MS);
    });

    try {
      const result = await invoke("open_workspace", { instanceId: workspace.instanceId });
      if (result === false || (isObject(result) && result.ok === false)) {
        throw new Error(asString(result?.error, "VS Code did not accept the open request."));
      }
      await transitionDelay;
    } catch (error) {
      showNotice(readableError(error, `Could not open ${workspace.name} in VS Code.`));
    } finally {
      finishWorkspaceLaunch(workspace.instanceId);
    }
  }

  function getIntegrationElements(kind) {
    if (kind === "companion") {
      return {
        card: elements.companionCard,
        status: elements.companionStatus,
        detail: elements.companionDetail,
        meta: elements.companionMeta,
        installButton: elements.companionInstallButton,
        uninstallButton: elements.companionUninstallButton,
      };
    }

    if (kind === "claude") {
      return {
        card: elements.claudeCard,
        status: elements.claudeStatus,
        detail: elements.claudeDetail,
        meta: elements.claudeMeta,
        installButton: elements.claudeInstallButton,
        uninstallButton: elements.claudeUninstallButton,
      };
    }

    return {
      card: elements.codexCard,
      status: elements.codexStatus,
      detail: elements.codexDetail,
      meta: elements.codexMeta,
      installButton: elements.codexInstallButton,
      uninstallButton: elements.codexUninstallButton,
    };
  }

  function renderIntegrationComponent(component) {
    const componentElements = getIntegrationElements(component.kind);
    componentElements.card.dataset.state = component.visualState;
    componentElements.status.dataset.state = component.visualState;
    componentElements.status.textContent = component.label;
    componentElements.detail.textContent = component.detail;
    componentElements.installButton.textContent = component.actionLabel;
    const componentName = component.kind === "companion"
      ? "VS Code companion"
      : component.kind === "codex"
        ? "Codex activity hooks"
        : "Claude Code activity hooks";
    componentElements.installButton.setAttribute(
      "aria-label",
      `${component.actionLabel} ${componentName}`,
    );
    componentElements.uninstallButton.hidden = !component.installed;

    let meta = "";
    if (component.kind === "companion") {
      if (component.installedVersion && component.targetVersion) {
        meta = `Installed ${component.installedVersion} · Bundled ${component.targetVersion}`;
      } else if (component.installedVersion) {
        meta = `Installed version ${component.installedVersion}`;
      } else if (component.targetVersion) {
        meta = `Bundled version ${component.targetVersion}`;
      }
    } else if (component.configPath) {
      meta = `Configuration: ${component.configPath}`;
    }
    componentElements.meta.textContent = meta;
    componentElements.meta.hidden = !meta;
  }

  function integrationProgressLabel(component, operation) {
    if (operation === "uninstall") {
      return "Uninstalling…";
    }
    if (component.actionLabel === "Update") {
      return "Updating…";
    }
    if (component.actionLabel === "Repair" || component.actionLabel === "Install / repair") {
      return "Repairing…";
    }
    return "Installing…";
  }

  function updateIntegrationControls() {
    const status = state.integrationStatus;
    const busy = state.integrationPending || Boolean(state.integrationAction);
    elements.integrationList.setAttribute("aria-busy", String(busy));
    elements.diagnosticsRefreshButton.disabled = busy || state.setupRefreshPending;
    elements.setupAllButton.disabled = busy || !status;
    elements.setupAllButton.textContent = state.integrationAction?.kind === "all"
      ? "Setting up…"
      : "Set up all";

    ["companion", "codex", "claude"].forEach((kind) => {
      const component = status?.[kind];
      const componentElements = getIntegrationElements(kind);
      const isCurrentAction = state.integrationAction?.kind === kind;
      componentElements.card.setAttribute("aria-busy", String(isCurrentAction));
      componentElements.installButton.disabled = busy || !component;
      componentElements.uninstallButton.disabled = busy || !component?.installed;

      if (component) {
        componentElements.installButton.textContent =
          isCurrentAction && state.integrationAction.operation === "install"
            ? integrationProgressLabel(component, "install")
            : component.actionLabel;
        componentElements.uninstallButton.textContent =
          isCurrentAction && state.integrationAction.operation === "uninstall"
            ? "Uninstalling…"
            : "Uninstall";
      }
    });
  }

  function updateSetupSummary() {
    let summary;
    let attention = false;

    if (!state.integrationStatus && !state.diagnosticsLoaded) {
      summary = state.diagnosticsUnavailable
        ? "Unavailable"
        : "Local only";
      attention = state.diagnosticsUnavailable;
    } else {
      const components = state.integrationStatus
        ? [
            state.integrationStatus.companion,
            state.integrationStatus.codex,
            state.integrationStatus.claude,
          ]
        : [];
      const requiredMissing = components.some(
        (component) => !component.optional && component.visualState === "missing",
      );
      const optionalMissing = components.some(
        (component) => component.optional && component.visualState === "missing",
      );
      const attentionCount = components.filter((component) =>
        ["warning", "error"].includes(component.visualState),
      ).length;
      const totalWarnings = attentionCount + state.diagnosticWarningCount;

      attention = requiredMissing || totalWarnings > 0;
      if (requiredMissing) {
        summary = "Setup needed";
      } else if (totalWarnings) {
        summary = `${totalWarnings} warning${totalWarnings === 1 ? "" : "s"}`;
      } else if (optionalMissing) {
        summary = "Optional setup";
      } else if (state.integrationStatus && state.diagnosticsLoaded) {
        summary = "Ready";
      } else if (state.integrationStatus) {
        summary = "Integrations ready";
      } else {
        summary = "Partially checked";
      }
    }

    elements.diagnosticsSummary.textContent = summary;
    elements.settingsButton.dataset.attention = String(attention);
    elements.settingsButton.setAttribute(
      "aria-label",
      `Open settings and diagnostics · ${summary}`,
    );
    elements.settingsButton.title = `Settings and diagnostics · ${summary}`;
  }

  function renderIntegrationStatus(status) {
    state.integrationStatus = status;
    state.integrationLoaded = true;
    renderIntegrationComponent(status.companion);
    renderIntegrationComponent(status.codex);
    renderIntegrationComponent(status.claude);
    elements.restartNotice.hidden = !status.requiresRestart;
    elements.codexTrustGuidance.dataset.active = String(status.codex.installed);
    elements.codexTrustGuidance.hidden = !status.codex.installed;
    updateIntegrationControls();
    updateSetupSummary();
  }

  function setIntegrationMessage(message, kind = "neutral") {
    elements.integrationMessage.textContent = message;
    elements.integrationMessage.hidden = !message;
    elements.integrationMessage.classList.toggle("has-error", kind === "error");
    elements.integrationMessage.classList.toggle("has-success", kind === "success");
  }

  async function refreshIntegrationStatus() {
    if (state.integrationPending || state.integrationAction) {
      return;
    }

    state.integrationPending = true;
    updateIntegrationControls();
    setIntegrationMessage("Checking integration status…");

    try {
      const raw = await invoke("get_integration_status", {});
      renderIntegrationStatus(normalizeIntegrationStatus(raw));
      setIntegrationMessage("");
    } catch (error) {
      const message = readableError(error, "Could not check the installed integrations.");
      state.integrationLoaded = false;
      renderIntegrationStatus(
        normalizeIntegrationStatus({
          schemaVersion: SCHEMA_VERSION,
          companion: { state: "error", label: "Check failed", detail: message },
          codex: { state: "error", label: "Check failed", detail: message },
          claude: { state: "error", label: "Check failed", detail: message },
          requiresRestart: false,
        }),
      );
      state.integrationLoaded = false;
      setIntegrationMessage(message, "error");
      updateSetupSummary();
    } finally {
      state.integrationPending = false;
      updateIntegrationControls();
    }
  }

  function integrationActionSuccess(kind, operation) {
    if (operation === "uninstall") {
      if (kind === "companion") {
        return "VS Code companion uninstalled. Existing stale heartbeats will age out automatically.";
      }
      return `${kind === "codex" ? "Codex" : "Claude Code"} activity hooks uninstalled. Existing stale activity markers will age out automatically.`;
    }
    if (kind === "companion") {
      return "VS Code companion installed. Reload open VS Code windows to start reporting heartbeats.";
    }
    return kind === "codex"
      ? "Codex hooks installed. In Codex, run /hooks and complete the required security review."
      : "Claude Code hooks installed. Restart affected Claude Code sessions to load the new lifecycle handlers.";
  }

  async function runIntegrationAction(kind, operation) {
    if (state.integrationAction) {
      return;
    }

    const commands = {
      companion: {
        install: "install_companion",
        uninstall: "uninstall_companion",
        name: "VS Code companion",
      },
      codex: {
        install: "install_codex_hooks",
        uninstall: "uninstall_codex_hooks",
        name: "Codex activity hooks",
      },
      claude: {
        install: "install_claude_hooks",
        uninstall: "uninstall_claude_hooks",
        name: "Claude Code activity hooks",
      },
    };
    const command = commands[kind][operation];
    const componentName = commands[kind].name;

    state.integrationAction = { kind, operation };
    updateIntegrationControls();
    setIntegrationMessage(
      `${operation === "uninstall" ? "Uninstalling" : "Installing or repairing"} ${componentName}…`,
    );

    try {
      const raw = await invoke(command, {});
      renderIntegrationStatus(normalizeIntegrationStatus(raw));
      setIntegrationMessage(integrationActionSuccess(kind, operation), "success");
    } catch (error) {
      setIntegrationMessage(
        readableError(error, `Could not ${operation} ${componentName}.`),
        "error",
      );
    } finally {
      state.integrationAction = null;
      updateIntegrationControls();
    }
  }

  function formatNaturalList(values) {
    if (values.length < 2) {
      return values[0] || "";
    }
    if (values.length === 2) {
      return `${values[0]} and ${values[1]}`;
    }
    return `${values.slice(0, -1).join(", ")}, and ${values.at(-1)}`;
  }

  function trimTerminalPunctuation(value) {
    return value.replace(/[.!?\s]+$/g, "");
  }

  async function setupAllIntegrations() {
    if (state.integrationAction || state.integrationPending) {
      return;
    }

    const steps = [
      {
        kind: "companion",
        name: "VS Code companion",
        command: "install_companion",
      },
      {
        kind: "codex",
        name: "Codex activity hooks",
        command: "install_codex_hooks",
      },
      {
        kind: "claude",
        name: "Claude Code activity hooks",
        command: "install_claude_hooks",
      },
    ];
    const completed = [];
    const unconfirmed = [];

    state.integrationAction = { kind: "all", operation: "install" };
    updateIntegrationControls();

    for (let index = 0; index < steps.length; index += 1) {
      const step = steps[index];
      setIntegrationMessage(
        `Step ${index + 1} of ${steps.length}: installing or repairing ${step.name}…`,
      );

      try {
        const raw = await invoke(step.command, {});
        renderIntegrationStatus(normalizeIntegrationStatus(raw));
        completed.push(step.name);
      } catch (error) {
        unconfirmed.push({
          name: step.name,
          error: readableError(error, "The operation did not return a confirmed result."),
        });
      }
    }

    if (unconfirmed.length === 0) {
      setIntegrationMessage(
        "All integrations are installed. Reload VS Code, restart affected provider sessions, then run /hooks in Codex and complete its required security review.",
        "success",
      );
    } else if (completed.length) {
      const completedNames = formatNaturalList(completed);
      const failureDetails = unconfirmed
        .map((failure) => `${failure.name}: ${trimTerminalPunctuation(failure.error)}`)
        .join(" · ");
      setIntegrationMessage(
        `Partial setup: ${completedNames} completed. The following could not be completed or verified: ${failureDetails}. Successful changes were kept; retry to finish setup.`,
        "error",
      );
    } else {
      const details = unconfirmed
        .map((failure) => `${failure.name}: ${trimTerminalPunctuation(failure.error)}`)
        .join(" · ");
      setIntegrationMessage(
        `Setup could not be completed or verified. No success was confirmed. ${details}`,
        "error",
      );
    }

    state.integrationAction = null;
    updateIntegrationControls();
  }

  function openSettingsDialog() {
    if (showAccessibleDialog(elements.settingsDialog, elements.settingsCloseButton)) {
      refreshSetup();
    }
  }

  function closeSettingsDialog() {
    closeAccessibleDialog(elements.settingsDialog);
  }

  function requestUninstall(kind) {
    if (state.integrationAction) {
      return;
    }

    const componentNames = {
      companion: "VS Code companion",
      codex: "Codex activity hooks",
      claude: "Claude Code activity hooks",
    };
    state.pendingUninstall = kind;
    elements.uninstallTitle.textContent = `Uninstall ${componentNames[kind]}?`;
    elements.uninstallDescription.textContent = kind === "companion"
      ? "VSParallel will stop receiving workspace heartbeats after existing VS Code windows are reloaded. No project files are removed."
      : `VSParallel will remove only its own ${kind === "codex" ? "Codex" : "Claude Code"} handlers. Other provider hooks remain unchanged, and no project files are removed.`;
    elements.uninstallConfirmButton.disabled = false;

    showAccessibleDialog(elements.uninstallDialog, elements.uninstallCancelButton);
  }

  function closeUninstallDialog() {
    closeAccessibleDialog(elements.uninstallDialog);
  }

  async function confirmUninstall() {
    const kind = state.pendingUninstall;
    if (!kind) {
      return;
    }

    elements.uninstallConfirmButton.disabled = true;
    closeUninstallDialog();
    state.pendingUninstall = null;
    await runIntegrationAction(kind, "uninstall");
  }

  function formatDuration(milliseconds) {
    const value = asFiniteNumber(milliseconds);
    if (value === null) {
      return "Unknown";
    }
    if (value < 60_000) {
      return `${Math.round(value / 1_000)} seconds`;
    }
    if (value < 3_600_000) {
      return `${Math.round(value / 60_000)} minutes`;
    }
    return `${Math.round(value / 3_600_000)} hours`;
  }

  function appendDiagnostic(label, value, warning = false) {
    const term = createElement("dt", "", label);
    const description = createElement("dd", warning ? "has-warning" : "", value);
    elements.diagnosticsList.append(term, description);
  }

  function renderDiagnostics(rawValue) {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw)) {
      throw new Error("The local monitor returned invalid diagnostics.");
    }
    if (raw.schemaVersion !== SCHEMA_VERSION) {
      throw new Error("The local monitor returned an unsupported diagnostics version.");
    }

    const malformedInstances = asNonNegativeInteger(raw.malformedInstanceRecords);
    const malformedCodex = asNonNegativeInteger(raw.malformedCodexRecords);
    const malformedClaude = asNonNegativeInteger(raw.malformedClaudeRecords);
    const omittedInstances = asNonNegativeInteger(raw.omittedInstanceRecords);
    const omittedCodex = asNonNegativeInteger(raw.omittedCodexRecords);
    const omittedClaude = asNonNegativeInteger(raw.omittedClaudeRecords);
    const validInstances = asNonNegativeInteger(raw.validInstanceRecords);
    const validCodex = asNonNegativeInteger(raw.validCodexRecords);
    const validClaude = asNonNegativeInteger(raw.validClaudeRecords);
    const totalMalformed = malformedInstances + malformedCodex + malformedClaude;
    const totalOmitted = omittedInstances + omittedCodex + omittedClaude;

    elements.diagnosticsList.replaceChildren();
    appendDiagnostic("State directory", asString(raw.stateDirectory, "Unavailable"));
    appendDiagnostic("VS Code command", asString(raw.codeCommand, "code"));
    appendDiagnostic(
      "Workspace records",
      `${validInstances} valid · ${malformedInstances} malformed · ${omittedInstances} omitted`,
      malformedInstances > 0 || omittedInstances > 0,
    );
    appendDiagnostic(
      "Codex activity records",
      `${validCodex} valid · ${malformedCodex} malformed · ${omittedCodex} omitted`,
      malformedCodex > 0 || omittedCodex > 0,
    );
    appendDiagnostic(
      "Claude Code activity records",
      `${validClaude} valid · ${malformedClaude} malformed · ${omittedClaude} omitted`,
      malformedClaude > 0 || omittedClaude > 0,
    );
    appendDiagnostic("Active heartbeat window", formatDuration(raw.activeTtlMs));
    appendDiagnostic("Inactive record retention", formatDuration(raw.staleRetentionMs));
    appendDiagnostic(
      "Activity freshness",
      formatDuration(raw.activityStaleMs ?? raw.codexStaleMs),
    );

    state.diagnosticWarningCount = totalMalformed + Number(totalOmitted > 0);
    state.diagnosticsLoaded = true;
    state.diagnosticsUnavailable = false;
    updateSetupSummary();
    elements.diagnosticsStatus.classList.remove("has-error");
    elements.diagnosticsStatus.textContent = validInstances
      ? "The companion is reporting one or more workspace records."
      : "No valid workspace heartbeat is present yet. Check that the companion extension is enabled in an open VS Code window.";
  }

  async function refreshDiagnostics() {
    if (state.diagnosticsPending) {
      return;
    }

    state.diagnosticsPending = true;
    elements.diagnosticsStatus.classList.remove("has-error");
    elements.diagnosticsStatus.textContent = "Checking local integration…";

    try {
      const diagnostics = await invoke("get_diagnostics", {});
      renderDiagnostics(diagnostics);
    } catch (error) {
      state.diagnosticsUnavailable = true;
      elements.diagnosticsStatus.classList.add("has-error");
      elements.diagnosticsStatus.textContent = readableError(
        error,
        "Could not load local integration diagnostics.",
      );
      updateSetupSummary();
    } finally {
      state.diagnosticsPending = false;
    }
  }

  async function refreshSetup() {
    if (state.setupRefreshPromise) {
      return state.setupRefreshPromise;
    }
    if (state.integrationAction) {
      return undefined;
    }

    state.setupRefreshPending = true;
    elements.diagnosticsRefreshButton.disabled = true;
    elements.diagnosticsRefreshButton.classList.add("is-loading");
    elements.diagnosticsRefreshButton.setAttribute("aria-busy", "true");
    elements.diagnosticsRefreshButton.setAttribute("aria-label", "Refreshing setup");

    state.setupRefreshPromise = Promise.all([
      refreshIntegrationStatus(),
      refreshDiagnostics(),
    ]).finally(() => {
      state.setupRefreshPending = false;
      state.setupRefreshPromise = null;
      elements.diagnosticsRefreshButton.classList.remove("is-loading");
      elements.diagnosticsRefreshButton.setAttribute("aria-busy", "false");
      elements.diagnosticsRefreshButton.removeAttribute("aria-label");
      updateIntegrationControls();
    });
    return state.setupRefreshPromise;
  }

  async function hideWindow() {
    elements.hideButton.disabled = true;
    elements.hideButton.setAttribute("aria-busy", "true");
    try {
      await invoke("hide_window", {});
    } catch (error) {
      showNotice(readableError(error, "Could not minimize VSParallel."));
    } finally {
      elements.hideButton.disabled = false;
      elements.hideButton.setAttribute("aria-busy", "false");
    }
  }

  async function toggleWindowMaximize() {
    if (elements.maximizeButton.disabled) {
      return;
    }

    elements.maximizeButton.disabled = true;
    elements.maximizeButton.setAttribute("aria-busy", "true");
    try {
      const raw = await invoke("toggle_window_maximize", {});
      renderWindowChromeState(normalizeWindowChromeState(raw));
      scheduleWindowChromeRefresh();
    } catch (error) {
      showNotice(readableError(error, "Could not maximize or restore VSParallel."));
    } finally {
      elements.maximizeButton.setAttribute("aria-busy", "false");
      elements.maximizeButton.disabled = state.windowChrome?.fullscreen === true;
    }
  }

  async function closeWindow() {
    elements.closeButton.disabled = true;
    elements.closeButton.setAttribute("aria-busy", "true");
    try {
      await invoke("close_window", {});
    } catch (error) {
      showNotice(readableError(error, "Could not close VSParallel."));
    } finally {
      elements.closeButton.disabled = false;
      elements.closeButton.setAttribute("aria-busy", "false");
    }
  }

  function navigateOpenButtons(event) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
      return;
    }

    const currentButton = event.target.closest?.(".open-button:not(:disabled)");
    if (!currentButton || !elements.workspaceList.contains(currentButton)) {
      return;
    }

    const buttons = Array.from(
      elements.workspaceList.querySelectorAll(".open-button:not(:disabled)"),
    );
    const currentIndex = buttons.indexOf(currentButton);
    let nextIndex = currentIndex;

    if (event.key === "ArrowDown") {
      nextIndex = Math.min(currentIndex + 1, buttons.length - 1);
    } else if (event.key === "ArrowUp") {
      nextIndex = Math.max(currentIndex - 1, 0);
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = buttons.length - 1;
    } else {
      return;
    }

    event.preventDefault();
    buttons[nextIndex]?.focus({ preventScroll: true });
  }

  function handleGlobalKeydown(event) {
    const commandModifier = (event.ctrlKey || event.metaKey) && !event.altKey;
    const normalizedKey = event.key.toLowerCase();

    if (commandModifier && !event.shiftKey && normalizedKey === ",") {
      event.preventDefault();
      if (!isDialogOpen(elements.uninstallDialog)) {
        openSettingsDialog();
      }
      return;
    }

    if (commandModifier && !event.shiftKey && normalizedKey === "r") {
      event.preventDefault();
      if (isDialogOpen(elements.uninstallDialog)) {
        return;
      }
      if (isDialogOpen(elements.settingsDialog)) {
        refreshSetup();
      } else {
        refreshSnapshot();
      }
      return;
    }

    if (event.key !== "Escape" || event.defaultPrevented) {
      return;
    }

    event.preventDefault();
    if (isDialogOpen(elements.uninstallDialog)) {
      closeUninstallDialog();
    } else if (isDialogOpen(elements.settingsDialog)) {
      closeSettingsDialog();
    } else {
      hideWindow();
    }
  }

  elements.refreshButton.addEventListener("click", refreshSnapshot);
  elements.emptyRefreshButton.addEventListener("click", refreshSnapshot);
  elements.hideButton.addEventListener("click", hideWindow);
  elements.maximizeButton.addEventListener("click", toggleWindowMaximize);
  elements.closeButton.addEventListener("click", closeWindow);
  elements.settingsButton.addEventListener("click", openSettingsDialog);
  elements.settingsCloseButton.addEventListener("click", closeSettingsDialog);
  elements.appearanceInputs.forEach((input) => {
    input.addEventListener("change", () => {
      if (input.checked) {
        applyThemePreference(input.value);
      }
    });
  });
  elements.diagnosticsRefreshButton.addEventListener("click", refreshSetup);
  elements.setupAllButton.addEventListener("click", setupAllIntegrations);
  elements.companionInstallButton.addEventListener("click", () =>
    runIntegrationAction("companion", "install"),
  );
  elements.codexInstallButton.addEventListener("click", () =>
    runIntegrationAction("codex", "install"),
  );
  elements.claudeInstallButton.addEventListener("click", () =>
    runIntegrationAction("claude", "install"),
  );
  elements.companionUninstallButton.addEventListener("click", () =>
    requestUninstall("companion"),
  );
  elements.codexUninstallButton.addEventListener("click", () => requestUninstall("codex"));
  elements.claudeUninstallButton.addEventListener("click", () => requestUninstall("claude"));
  elements.uninstallConfirmButton.addEventListener("click", confirmUninstall);
  elements.uninstallCancelButton.addEventListener("click", (event) => {
    event.preventDefault();
    closeUninstallDialog();
  });
  elements.settingsDialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    closeSettingsDialog();
  });
  elements.settingsDialog.addEventListener("close", () => {
    restoreDialogFocus(elements.settingsDialog);
  });
  elements.uninstallDialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    closeUninstallDialog();
  });
  elements.uninstallDialog.addEventListener("close", () => {
    state.pendingUninstall = null;
    restoreDialogFocus(elements.uninstallDialog);
  });
  elements.workspaceList.addEventListener("keydown", navigateOpenButtons);

  window.addEventListener("focus", () => {
    document.documentElement.dataset.windowFocused = "true";
    refreshSnapshot();
    scheduleWindowChromeRefresh();
  });
  window.addEventListener("blur", () => {
    document.documentElement.dataset.windowFocused = "false";
    scheduleWindowChromeRefresh();
  });
  window.addEventListener("resize", scheduleWindowChromeRefresh);
  window.addEventListener("keydown", handleGlobalKeydown);
  if (typeof lightThemeQuery.addEventListener === "function") {
    lightThemeQuery.addEventListener("change", handleSystemThemeChange);
  } else {
    lightThemeQuery.addListener(handleSystemThemeChange);
  }

  updateSetupSummary();
  renderWindowChromeState(fallbackWindowChromeState());
  refreshWindowChromeState();
  applyThemePreference(state.themePreference, false);
  refreshIntegrationStatus();
  refreshSnapshot();
  window.setInterval(refreshSnapshot, REFRESH_INTERVAL_MS);
})();
