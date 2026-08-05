(function () {
  "use strict";

  type JsonObject = Record<string, unknown>;
  type ThemePreference = "system" | "light" | "dark";
  type ColorTheme = Exclude<ThemePreference, "system">;
  type IntegrationKind = "companion" | "codex" | "claude";
  type IntegrationActionKind = IntegrationKind | "all";
  type IntegrationOperation = "install" | "uninstall";
  type IntegrationVisualState = "missing" | "ready" | "warning" | "error";
  type ActivityKind = "activity" | "finished" | "failure" | "unknown";
  type UsageKind = "codex" | "claude";
  type UsageState = "available" | "stale" | "unavailable";
  type NoticeKind = "error" | "warning";
  type IntegrationMessageKind = "neutral" | "error" | "success";
  type TauriCommand =
    | "close_window"
    | "get_diagnostics"
    | "get_integration_status"
    | "get_snapshot"
    | "get_usage"
    | "get_window_chrome_state"
    | "hide_window"
    | "install_claude_hooks"
    | "install_codex_hooks"
    | "install_companion"
    | "open_workspace"
    | "restore_full_window"
    | "set_window_chrome_theme"
    | "toggle_window_maximize"
    | "uninstall_claude_hooks"
    | "uninstall_codex_hooks"
    | "uninstall_companion";

  interface ActivityView {
    kind: ActivityKind;
    mark: string;
    label: string;
    changedAtMs: number | null;
    detail: string;
    extensionDetectionAvailable: boolean | null;
    extensionInstalled: boolean | null;
    extensionActive: boolean | null;
  }

  interface Workspace {
    instanceId: string;
    name: string;
    path: string;
    openable: boolean;
    active: boolean;
    focused: boolean;
    lastSeenAtMs: number | null;
    codex: ActivityView;
    claude: ActivityView;
  }

  interface Snapshot {
    schemaVersion: number;
    generatedAtMs: number;
    malformedRecords: number;
    workspaces: Workspace[];
  }

  interface UsageWindow {
    label: string;
    durationMinutes: number | null;
    usedPercent: number;
    remainingPercent: number;
    resetsAtMs: number | null;
  }

  interface UsageProvider {
    providerName: string;
    state: UsageState;
    remainingPercent: number | null;
    windowLabel: string;
    resetsAtMs: number | null;
    updatedAtMs: number | null;
    detail: string;
    windows: UsageWindow[];
  }

  interface UsageSnapshot {
    schemaVersion: number;
    generatedAtMs: number;
    codex: UsageProvider;
    claude: UsageProvider;
  }

  interface IntegrationComponent {
    kind: IntegrationKind;
    optional: boolean;
    token: string;
    visualState: IntegrationVisualState;
    installed: boolean;
    actionLabel: string;
    label: string;
    detail: string;
    installedVersion: string;
    targetVersion: string;
    configPath: string;
  }

  interface IntegrationStatus {
    schemaVersion: number;
    companion: IntegrationComponent;
    codex: IntegrationComponent;
    claude: IntegrationComponent;
    requiresRestart: boolean;
  }

  interface IntegrationAction {
    kind: IntegrationActionKind;
    operation: IntegrationOperation;
  }

  interface IntegrationElements {
    card: HTMLElement;
    status: HTMLSpanElement;
    detail: HTMLParagraphElement;
    meta: HTMLParagraphElement;
    installButton: HTMLButtonElement;
    uninstallButton: HTMLButtonElement;
  }

  interface WindowChromeState {
    schemaVersion: 1;
    platform: string;
    customControls: boolean;
    maximized: boolean;
    fullscreen: boolean;
    focused: boolean;
    floating: boolean;
  }

  interface AppState {
    refreshPending: boolean;
    usagePending: boolean;
    diagnosticsPending: boolean;
    diagnosticsLoaded: boolean;
    diagnosticsUnavailable: boolean;
    diagnosticWarningCount: number;
    setupRefreshPending: boolean;
    setupRefreshPromise: Promise<[void, void]> | null;
    integrationPending: boolean;
    integrationLoaded: boolean;
    integrationStatus: IntegrationStatus | null;
    integrationAction: IntegrationAction | null;
    pendingUninstall: IntegrationKind | null;
    openingInstanceId: string | null;
    lastGoodSnapshot: Snapshot | null;
    lastUsage: UsageSnapshot | null;
    lastUsageAttemptAtMs: number | null;
    windowChrome: WindowChromeState | null;
    windowChromeRequestId: number;
    windowChromeRefreshTimer: number | null;
    themePreference: ThemePreference;
  }

  interface TauriWindow extends Window {
    __TAURI__?: {
      core?: {
        invoke?: (command: string, args?: JsonObject) => Promise<unknown>;
      };
    };
  }

  function requiredElement<T extends Element>(selector: string): T {
    const element = document.querySelector<T>(selector);
    if (!element) {
      throw new Error(`The UI is missing its required ${selector} element.`);
    }
    return element;
  }

  function requiredDescendant<T extends Element>(parent: ParentNode, selector: string): T {
    const element = parent.querySelector<T>(selector);
    if (!element) {
      throw new Error(`The UI is missing its required ${selector} descendant.`);
    }
    return element;
  }

  const SCHEMA_VERSION = 1;
  const REFRESH_INTERVAL_MS = 3_000;
  const USAGE_REFRESH_INTERVAL_MS = 60_000;
  const USAGE_LAST_KNOWN_MAX_AGE_MS = 15 * 60_000;
  const LAUNCH_TRANSITION_MIN_MS = 750;
  const THEME_STORAGE_KEY = "vsparallel.appearance";
  const THEME_PREFERENCES: ReadonlySet<string> = new Set(["system", "light", "dark"]);
  const INTEGRATION_KINDS = ["companion", "codex", "claude"] as const;
  const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000;
  const tauriInvoke = (window as TauriWindow).__TAURI__?.core?.invoke;
  const lightThemeQuery = window.matchMedia("(prefers-color-scheme: light)");

  const elements = {
    connectionBar: requiredElement<HTMLDivElement>("#connectionBar"),
    connectionText: requiredElement<HTMLSpanElement>("#connectionText"),
    updatedAt: requiredElement<HTMLTimeElement>("#updatedAt"),
    refreshButton: requiredElement<HTMLButtonElement>("#refreshButton"),
    restoreFullButton: requiredElement<HTMLButtonElement>("#restoreFullButton"),
    hidePanelButton: requiredElement<HTMLButtonElement>("#hidePanelButton"),
    hideButton: requiredElement<HTMLButtonElement>("#hideButton"),
    appTitlebar: requiredElement<HTMLElement>("#appTitlebar"),
    titlebarDragRegion: requiredElement<HTMLDivElement>("#titlebarDragRegion"),
    windowControls: requiredElement<HTMLDivElement>("#windowControls"),
    maximizeButton: requiredElement<HTMLButtonElement>("#maximizeButton"),
    closeButton: requiredElement<HTMLButtonElement>("#closeButton"),
    workspaceCount: requiredElement<HTMLSpanElement>("#workspaceCount"),
    usageOverview: requiredElement<HTMLElement>("#usageOverview"),
    usageStatus: requiredElement<HTMLParagraphElement>("#usageStatus"),
    codexUsage: requiredElement<HTMLElement>("#codexUsage"),
    codexUsageValue: requiredElement<HTMLElement>("#codexUsageValue"),
    codexUsageState: requiredElement<HTMLSpanElement>("#codexUsageState"),
    codexUsageMeter: requiredElement<HTMLDivElement>("#codexUsageMeter"),
    codexUsageDetail: requiredElement<HTMLSpanElement>("#codexUsageDetail"),
    claudeUsage: requiredElement<HTMLElement>("#claudeUsage"),
    claudeUsageValue: requiredElement<HTMLElement>("#claudeUsageValue"),
    claudeUsageState: requiredElement<HTMLSpanElement>("#claudeUsageState"),
    claudeUsageMeter: requiredElement<HTMLDivElement>("#claudeUsageMeter"),
    claudeUsageDetail: requiredElement<HTMLSpanElement>("#claudeUsageDetail"),
    workspaceList: requiredElement<HTMLUListElement>("#workspaceList"),
    errorBanner: requiredElement<HTMLDivElement>("#errorBanner"),
    errorText: requiredElement<HTMLSpanElement>("#errorText"),
    emptyState: requiredElement<HTMLDivElement>("#emptyState"),
    emptyRefreshButton: requiredElement<HTMLButtonElement>("#emptyRefreshButton"),
    launchOverlay: requiredElement<HTMLDivElement>("#launchOverlay"),
    launchStatus: requiredElement<HTMLSpanElement>("#launchStatus"),
    settingsButton: requiredElement<HTMLButtonElement>("#settingsButton"),
    settingsDialog: requiredElement<HTMLDialogElement>("#settingsDialog"),
    settingsCloseButton: requiredElement<HTMLButtonElement>("#settingsCloseButton"),
    diagnosticsSummary: requiredElement<HTMLSpanElement>("#diagnosticsSummary"),
    diagnosticsList: requiredElement<HTMLDListElement>("#diagnosticsList"),
    diagnosticsStatus: requiredElement<HTMLParagraphElement>("#diagnosticsStatus"),
    diagnosticsRefreshButton: requiredElement<HTMLButtonElement>("#diagnosticsRefreshButton"),
    setupAllButton: requiredElement<HTMLButtonElement>("#setupAllButton"),
    integrationList: requiredElement<HTMLDivElement>("#integrationList"),
    integrationMessage: requiredElement<HTMLParagraphElement>("#integrationMessage"),
    companionCard: requiredElement<HTMLElement>("#companionCard"),
    companionStatus: requiredElement<HTMLSpanElement>("#companionStatus"),
    companionDetail: requiredElement<HTMLParagraphElement>("#companionDetail"),
    companionMeta: requiredElement<HTMLParagraphElement>("#companionMeta"),
    companionInstallButton: requiredElement<HTMLButtonElement>("#companionInstallButton"),
    companionUninstallButton: requiredElement<HTMLButtonElement>("#companionUninstallButton"),
    codexCard: requiredElement<HTMLElement>("#codexCard"),
    codexStatus: requiredElement<HTMLSpanElement>("#codexStatus"),
    codexDetail: requiredElement<HTMLParagraphElement>("#codexDetail"),
    codexMeta: requiredElement<HTMLParagraphElement>("#codexMeta"),
    codexInstallButton: requiredElement<HTMLButtonElement>("#codexInstallButton"),
    codexUninstallButton: requiredElement<HTMLButtonElement>("#codexUninstallButton"),
    codexTrustGuidance: requiredElement<HTMLDivElement>("#codexTrustGuidance"),
    claudeCard: requiredElement<HTMLElement>("#claudeCard"),
    claudeStatus: requiredElement<HTMLSpanElement>("#claudeStatus"),
    claudeDetail: requiredElement<HTMLParagraphElement>("#claudeDetail"),
    claudeMeta: requiredElement<HTMLParagraphElement>("#claudeMeta"),
    claudeInstallButton: requiredElement<HTMLButtonElement>("#claudeInstallButton"),
    claudeUninstallButton: requiredElement<HTMLButtonElement>("#claudeUninstallButton"),
    restartNotice: requiredElement<HTMLDivElement>("#restartNotice"),
    uninstallDialog: requiredElement<HTMLDialogElement>("#uninstallDialog"),
    uninstallTitle: requiredElement<HTMLHeadingElement>("#uninstallTitle"),
    uninstallDescription: requiredElement<HTMLParagraphElement>("#uninstallDescription"),
    uninstallCancelButton: requiredElement<HTMLButtonElement>("#uninstallCancelButton"),
    uninstallConfirmButton: requiredElement<HTMLButtonElement>("#uninstallConfirmButton"),
    appearanceInputs: Array.from(
      document.querySelectorAll<HTMLInputElement>('input[name="appearance"]'),
    ),
  };

  const initialThemePreference: ThemePreference = isThemePreference(
    document.documentElement.dataset.themePreference,
  )
    ? document.documentElement.dataset.themePreference
    : "system";

  const state: AppState = {
    refreshPending: false,
    usagePending: false,
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
    lastUsage: null,
    lastUsageAttemptAtMs: null,
    windowChrome: null,
    windowChromeRequestId: 0,
    windowChromeRefreshTimer: null,
    themePreference: initialThemePreference,
  };

  const dialogReturnFocus = new WeakMap<HTMLDialogElement, HTMLElement>();

  function isObject(value: unknown): value is JsonObject {
    return value !== null && typeof value === "object" && !Array.isArray(value);
  }

  function isUnknownArray(value: unknown): value is unknown[] {
    return Array.isArray(value);
  }

  function isThemePreference(value: unknown): value is ThemePreference {
    return typeof value === "string" && THEME_PREFERENCES.has(value);
  }

  function asString(value: unknown, fallback = ""): string {
    return typeof value === "string" && value.trim() ? value.trim() : fallback;
  }

  function asFiniteNumber(value: unknown, fallback: number | null = null): number | null {
    return typeof value === "number" && Number.isFinite(value) ? value : fallback;
  }

  function asNonNegativeInteger(value: unknown): number {
    const number = asFiniteNumber(value) ?? 0;
    return Math.max(0, Math.trunc(number));
  }

  function asTimestamp(value: unknown, fallback: number | null = null): number | null {
    const number = asFiniteNumber(value);
    return number !== null && number >= 0 && number <= MAX_JAVASCRIPT_TIMESTAMP_MS
      ? number
      : fallback;
  }

  function asPercentage(value: unknown): number | null {
    const number = asFiniteNumber(value);
    return number === null ? null : Math.min(100, Math.max(0, number));
  }

  function asNullableBoolean(value: unknown): boolean | null {
    return typeof value === "boolean" ? value : null;
  }

  function parseBridgeValue(value: unknown): unknown {
    if (typeof value !== "string") {
      return value;
    }

    try {
      return JSON.parse(value) as unknown;
    } catch (_error) {
      return value;
    }
  }

  async function invoke(command: TauriCommand, args: JsonObject): Promise<unknown> {
    if (typeof tauriInvoke === "function") {
      return parseBridgeValue(await tauriInvoke(command, args));
    }

    throw new Error("The Tauri bridge is unavailable. Run VSParallel as a desktop app.");
  }

  function normalizeStateToken(value: unknown): string {
    return asString(value, "unknown")
      .toLowerCase()
      .replace(/[\s-]+/g, "_");
  }

  function normalizeIntegrationComponent(
    rawValue: unknown,
    kind: IntegrationKind,
  ): IntegrationComponent {
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
    let visualState: IntegrationVisualState = "warning";

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

  function normalizeIntegrationStatus(rawValue: unknown): IntegrationStatus {
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

  function describeActivityState(token: string): Pick<ActivityView, "kind" | "mark" | "label"> {
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

  function normalizeActivityView(rawValue: unknown): ActivityView {
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

  function normalizeWorkspace(raw: unknown, index: number): Workspace {
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

  function normalizeSnapshot(rawValue: unknown): Snapshot {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw)) {
      throw new Error("The local monitor returned an invalid snapshot.");
    }

    if (!isUnknownArray(raw.workspaces)) {
      throw new Error("The local monitor snapshot is missing its workspace list.");
    }
    if (raw.schemaVersion !== SCHEMA_VERSION) {
      throw new Error("The local monitor returned an unsupported snapshot version.");
    }

    const workspaces: Workspace[] = [];
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
      generatedAtMs: asTimestamp(raw.generatedAtMs) ?? Date.now(),
      malformedRecords,
      workspaces,
    };
  }

  function usageWindowLabel(
    durationMinutes: number | null,
    fallback = "Usage limit",
  ): string {
    if (durationMinutes === 300) {
      return "5-hour limit";
    }
    if (durationMinutes === 10_080) {
      return "7-day limit";
    }
    if (durationMinutes !== null && Number.isFinite(durationMinutes) && durationMinutes > 0) {
      if (durationMinutes % 1_440 === 0) {
        const days = durationMinutes / 1_440;
        return `${days}-day limit`;
      }
      if (durationMinutes % 60 === 0) {
        const hours = durationMinutes / 60;
        return `${hours}-hour limit`;
      }
      return `${durationMinutes}-minute limit`;
    }
    return fallback;
  }

  function normalizeUsageWindow(rawValue: unknown): UsageWindow | null {
    const raw = isObject(rawValue) ? rawValue : {};
    const usedPercent = asPercentage(raw.usedPercent);
    const suppliedRemaining = asPercentage(raw.remainingPercent);
    const remainingPercent = suppliedRemaining ?? (
      usedPercent === null ? null : 100 - usedPercent
    );
    if (remainingPercent === null) {
      return null;
    }

    const durationMinutes = asFiniteNumber(raw.durationMinutes)
      ?? asFiniteNumber(raw.windowDurationMins);
    return {
      label: asString(
        raw.label ?? raw.kind,
        usageWindowLabel(durationMinutes),
      ),
      durationMinutes,
      usedPercent: usedPercent ?? 100 - remainingPercent,
      remainingPercent,
      resetsAtMs: asTimestamp(raw.resetsAtMs),
    };
  }

  function normalizeUsageProvider(rawValue: unknown, providerName: string): UsageProvider {
    const raw = isObject(rawValue) ? rawValue : {};
    const windows = isUnknownArray(raw.windows)
      ? raw.windows
          .map(normalizeUsageWindow)
          .filter((window): window is UsageWindow => window !== null)
      : [];
    const limitingWindow = windows.reduce<UsageWindow | null>(
      (current, candidate) => !current || candidate.remainingPercent < current.remainingPercent
        ? candidate
        : current,
      null,
    );
    const suppliedRemaining = asPercentage(
      raw.summaryRemainingPercent ?? raw.remainingPercent,
    );
    const remainingPercent = suppliedRemaining ?? limitingWindow?.remainingPercent ?? null;
    const rawState = normalizeStateToken(raw.state);
    const available = remainingPercent !== null
      && !["unavailable", "unsupported", "not_authenticated", "unknown"].includes(rawState);

    return {
      providerName,
      state: available ? (rawState === "stale" ? "stale" : "available") : "unavailable",
      remainingPercent: available ? remainingPercent : null,
      windowLabel: asString(
        raw.summaryWindowLabel ?? raw.windowLabel,
        limitingWindow?.label || "Usage limit",
      ),
      resetsAtMs: asTimestamp(
        raw.summaryResetsAtMs ?? raw.resetsAtMs,
        limitingWindow?.resetsAtMs ?? null,
      ),
      updatedAtMs: asTimestamp(raw.updatedAtMs ?? raw.capturedAtMs),
      detail: asString(raw.detail),
      windows,
    };
  }

  function normalizeUsageSnapshot(rawValue: unknown): UsageSnapshot {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw) || raw.schemaVersion !== SCHEMA_VERSION) {
      throw new Error("The local monitor returned unsupported usage data.");
    }

    return {
      schemaVersion: raw.schemaVersion,
      generatedAtMs: asTimestamp(raw.generatedAtMs) ?? Date.now(),
      codex: normalizeUsageProvider(raw.codex, "Codex"),
      claude: normalizeUsageProvider(raw.claude, "Claude"),
    };
  }

  function usageProviderWithFallback(
    current: UsageProvider,
    previous: UsageProvider | null,
    nowMs: number,
  ): UsageProvider {
    if (!previous || current.remainingPercent !== null || previous.remainingPercent === null) {
      return current;
    }

    if (previous.updatedAtMs === null || !Number.isFinite(previous.updatedAtMs)) {
      return current;
    }
    const ageMs = nowMs - previous.updatedAtMs;
    if (ageMs < 0 || ageMs > USAGE_LAST_KNOWN_MAX_AGE_MS) {
      return current;
    }

    const windows = previous.windows.filter(
      (window) => window.resetsAtMs === null || window.resetsAtMs > nowMs,
    );
    if (previous.windows.length && !windows.length) {
      return current;
    }

    const limitingWindow = windows.reduce<UsageWindow | null>(
      (selected, candidate) => !selected
          || candidate.remainingPercent < selected.remainingPercent
        ? candidate
        : selected,
      null,
    );
    return {
      ...previous,
      state: "stale",
      remainingPercent: limitingWindow?.remainingPercent ?? previous.remainingPercent,
      windowLabel: limitingWindow?.label || previous.windowLabel,
      resetsAtMs: limitingWindow ? limitingWindow.resetsAtMs : previous.resetsAtMs,
      detail: current.detail || "Provider refresh failed; showing the last known value.",
      windows,
    };
  }

  function resolveUsageSnapshot(
    current: UsageSnapshot,
    previous: UsageSnapshot | null,
    nowMs = Date.now(),
  ): UsageSnapshot {
    if (!previous) {
      return current;
    }
    return {
      ...current,
      codex: usageProviderWithFallback(current.codex, previous.codex, nowMs),
      claude: usageProviderWithFallback(current.claude, previous.claude, nowMs),
    };
  }

  function unavailableUsageSnapshot(detail: string): UsageSnapshot {
    const unavailable = normalizeUsageProvider({ detail }, "Provider");
    return {
      schemaVersion: SCHEMA_VERSION,
      generatedAtMs: Date.now(),
      codex: { ...unavailable, providerName: "Codex" },
      claude: { ...unavailable, providerName: "Claude" },
    };
  }

  function deriveName(path: string): string {
    if (!path) {
      return "";
    }
    const segments = path.replace(/[\\/]+$/, "").split(/[\\/]/);
    return segments.at(-1) || "";
  }

  function formatShortPath(path: string): string {
    if (!path || path.length <= 54) {
      return path || "Path unavailable";
    }

    const separator = path.includes("\\") ? "\\" : "/";
    const drive = path.match(/^[A-Za-z]:/)?.[0] || "";
    const segments = path.split(/[\\/]/).filter(Boolean);
    const tail = segments.slice(-3).join(separator);
    return drive ? `${drive}${separator}…${separator}${tail}` : `…${separator}${tail}`;
  }

  function formatRelativeTime(timestamp: number | null): string {
    if (timestamp === null || !Number.isFinite(timestamp)) {
      return "Update time unknown";
    }

    const deltaSeconds = Math.round((timestamp - Date.now()) / 1_000);
    const absoluteSeconds = Math.abs(deltaSeconds);
    let value: number;
    let unit: Intl.RelativeTimeFormatUnit;

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

  function formatAbsoluteTime(timestamp: number | null): string {
    if (timestamp === null || !Number.isFinite(timestamp)) {
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

  function formatResetTime(timestamp: number | null): string {
    if (timestamp === null || !Number.isFinite(timestamp)) {
      return "reset time unavailable";
    }
    const includeWeekday = timestamp - Date.now() > 24 * 60 * 60 * 1_000;
    return `resets ${new Intl.DateTimeFormat(undefined, {
      ...(includeWeekday ? { weekday: "short" } : {}),
      hour: "numeric",
      minute: "2-digit",
    }).format(new Date(timestamp))}`;
  }

  function createElement<K extends keyof HTMLElementTagNameMap>(
    tag: K,
    className: string,
    text?: string,
  ): HTMLElementTagNameMap[K] {
    const element = document.createElement(tag);
    if (className) {
      element.className = className;
    }
    if (text !== undefined) {
      element.textContent = text;
    }
    return element;
  }

  function aggregateActivity(workspace: Workspace): ActivityView {
    const priority: Record<ActivityKind, number> = {
      activity: 4,
      failure: 3,
      finished: 2,
      unknown: 1,
    };
    return [workspace.codex, workspace.claude].reduce((current, candidate) =>
      priority[candidate.kind] > priority[current.kind] ? candidate : current,
    );
  }

  function describeExtensionPresence(activity: ActivityView): {
    state: string;
    label: string;
    title: string;
  } {
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

  function createProviderState(
    providerName: string,
    activity: ActivityView,
    accessibleProviderName = providerName,
  ): HTMLDivElement {
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

    const relativeTime = activity.changedAtMs !== null && Number.isFinite(activity.changedAtMs)
      ? formatRelativeTime(activity.changedAtMs).replace("Updated ", "")
      : "Time unknown";
    const changedAt = createElement("time", "provider-time", relativeTime);
    if (activity.changedAtMs !== null && Number.isFinite(activity.changedAtMs)) {
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

  function createWorkspaceRow(workspace: Workspace): HTMLLIElement {
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
    const actionLabel = workspace.active ? "Switch to" : "Open";
    openButton.type = "button";
    openButton.dataset.instanceId = workspace.instanceId;
    openButton.disabled = !openable;
    openButton.setAttribute("aria-label", `${actionLabel} ${workspace.name} in VS Code`);
    openButton.setAttribute("aria-busy", String(opening));
    if (!workspace.openable) {
      openButton.title = "This workspace cannot currently be opened";
    } else {
      openButton.title = `${actionLabel} ${workspace.name} in VS Code`;
    }
    openButton.addEventListener("click", () => openWorkspace(workspace));
    row.append(openButton);

    return row;
  }

  function renderSnapshot(snapshot: Snapshot): void {
    const focusedOpenButton = document.activeElement
      ?.closest<HTMLButtonElement>(".open-button") ?? null;
    const focusedInstanceId = focusedOpenButton
      && elements.workspaceList.contains(focusedOpenButton)
      ? focusedOpenButton.dataset.instanceId
      : "";
    const fragment = document.createDocumentFragment();
    snapshot.workspaces.forEach((workspace) => {
      fragment.append(createWorkspaceRow(workspace));
    });

    elements.workspaceList.replaceChildren(fragment);
    if (focusedInstanceId) {
      const replacement = Array.from(
        elements.workspaceList.querySelectorAll<HTMLButtonElement>(
          ".open-button:not(:disabled)",
        ),
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

  function usageElements(kind: UsageKind) {
    return kind === "codex"
      ? {
          card: elements.codexUsage,
          value: elements.codexUsageValue,
          stateLabel: elements.codexUsageState,
          meter: elements.codexUsageMeter,
          detail: elements.codexUsageDetail,
        }
      : {
          card: elements.claudeUsage,
          value: elements.claudeUsageValue,
          stateLabel: elements.claudeUsageState,
          meter: elements.claudeUsageMeter,
          detail: elements.claudeUsageDetail,
        };
  }

  function usageLevel(remainingPercent: number): "critical" | "warning" | "normal" {
    if (remainingPercent <= 10) {
      return "critical";
    }
    if (remainingPercent <= 25) {
      return "warning";
    }
    return "normal";
  }

  function renderUsageProvider(kind: UsageKind, provider: UsageProvider): string {
    const target = usageElements(kind);
    const remainingPercent = provider.remainingPercent;
    const available = remainingPercent !== null;
    const stale = available && provider.state === "stale";
    target.card.dataset.state = available ? (stale ? "stale" : "available") : "unavailable";
    target.stateLabel.hidden = !stale;

    if (!available) {
      delete target.card.dataset.level;
      target.card.style.setProperty("--usage-remaining", "0");
      target.value.textContent = "—";
      target.detail.textContent = provider.detail || "Usage unavailable";
      target.meter.removeAttribute("role");
      target.meter.removeAttribute("aria-label");
      target.meter.removeAttribute("aria-valuemin");
      target.meter.removeAttribute("aria-valuemax");
      target.meter.removeAttribute("aria-valuenow");
      target.meter.removeAttribute("aria-valuetext");
      target.meter.setAttribute("aria-hidden", "true");
      target.card.title = provider.detail || `${provider.providerName} usage is unavailable.`;
      return provider.detail
        ? `${provider.providerName} usage unavailable: ${provider.detail}`
        : `${provider.providerName} usage unavailable`;
    }

    const roundedRemaining = Math.round(remainingPercent);
    const resetLabel = formatResetTime(provider.resetsAtMs);
    const updateLabel = provider.updatedAtMs !== null && Number.isFinite(provider.updatedAtMs)
      ? formatRelativeTime(provider.updatedAtMs).toLowerCase()
      : "";
    const detailParts = [provider.windowLabel, resetLabel];
    if (stale) {
      detailParts.push("last known value");
    }
    target.card.dataset.level = usageLevel(remainingPercent);
    target.card.style.setProperty("--usage-remaining", remainingPercent.toFixed(2));
    target.value.textContent = `${roundedRemaining}% left`;
    target.detail.textContent = detailParts.join(" · ");
    target.meter.removeAttribute("aria-hidden");
    target.meter.setAttribute("role", "meter");
    target.meter.setAttribute("aria-label", `${provider.providerName} usage remaining`);
    target.meter.setAttribute("aria-valuemin", "0");
    target.meter.setAttribute("aria-valuemax", "100");
    target.meter.setAttribute("aria-valuenow", remainingPercent.toFixed(1));
    target.meter.setAttribute(
      "aria-valuetext",
      [
        `${roundedRemaining}% remaining on the ${provider.windowLabel.toLowerCase()}`,
        resetLabel,
        stale ? "last known value" : "",
        stale ? provider.detail : "",
      ].filter(Boolean).join("; "),
    );
    const windowDescriptions = provider.windows.map((window) => {
      const remaining = Math.round(window.remainingPercent);
      return `${window.label}: ${remaining}% remaining, ${formatResetTime(window.resetsAtMs)}`;
    });
    target.card.title = [
      `${provider.providerName}: ${roundedRemaining}% remaining on the ${provider.windowLabel.toLowerCase()}`,
      resetLabel,
      ...windowDescriptions,
      updateLabel,
      provider.detail,
    ].filter(Boolean).join(" · ");
    return `${provider.providerName} ${roundedRemaining}% remaining${stale ? " (last known)" : ""}`;
  }

  function renderUsageSnapshot(snapshot: UsageSnapshot): void {
    const summaries = [
      renderUsageProvider("codex", snapshot.codex),
      renderUsageProvider("claude", snapshot.claude),
    ];
    const summary = summaries.join(". ");
    if (elements.usageStatus.textContent !== summary) {
      elements.usageStatus.textContent = summary;
    }
  }

  function showNotice(message: string, kind: NoticeKind = "error"): void {
    elements.errorText.textContent = message;
    elements.errorBanner.hidden = false;
    elements.errorBanner.classList.toggle("notice--error", kind === "error");
    elements.errorBanner.classList.toggle("notice--warning", kind === "warning");
  }

  function clearNotice(): void {
    elements.errorBanner.hidden = true;
    elements.errorText.textContent = "";
    elements.errorBanner.classList.add("notice--error");
    elements.errorBanner.classList.remove("notice--warning");
  }

  function updateRefreshControl(): void {
    const pending = state.refreshPending || state.usagePending;
    elements.refreshButton.disabled = pending;
    elements.refreshButton.setAttribute("aria-busy", String(pending));
    elements.refreshButton.classList.toggle("is-loading", pending);
    elements.usageOverview.setAttribute(
      "aria-busy",
      String(state.usagePending || !state.lastUsage),
    );
  }

  function readableError(error: unknown, fallback: string): string {
    if (error instanceof Error && error.message) {
      return error.message;
    }
    if (typeof error === "string" && error.trim()) {
      return error.trim();
    }
    return fallback;
  }

  function isDialogOpen(dialog: HTMLDialogElement): boolean {
    return Boolean(dialog?.open || dialog?.hasAttribute("open"));
  }

  function restoreDialogFocus(dialog: HTMLDialogElement): void {
    const returnTarget = dialogReturnFocus.get(dialog);
    dialogReturnFocus.delete(dialog);
    if (returnTarget?.isConnected && typeof returnTarget.focus === "function") {
      window.requestAnimationFrame(() => returnTarget.focus());
    }
  }

  function showAccessibleDialog(
    dialog: HTMLDialogElement,
    initialFocus: HTMLElement,
  ): boolean {
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

  function closeAccessibleDialog(dialog: HTMLDialogElement): boolean {
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

  function normalizeWindowChromeState(rawValue: unknown): WindowChromeState {
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
      floating: raw.floating === true,
    };
  }

  function fallbackWindowChromeState(): WindowChromeState {
    const isMac = /Macintosh|Mac OS X/.test(window.navigator.userAgent);
    return {
      schemaVersion: 1,
      platform: isMac ? "macos" : "unknown",
      customControls: Boolean(tauriInvoke) && !isMac,
      maximized: false,
      fullscreen: false,
      focused: document.hasFocus(),
      floating: false,
    };
  }

  function renderWindowChromeState(chrome: WindowChromeState): void {
    state.windowChrome = chrome;
    document.documentElement.dataset.windowPlatform = chrome.platform;
    document.documentElement.dataset.windowFocused = String(chrome.focused);
    document.documentElement.dataset.windowMaximized = String(chrome.maximized);
    document.documentElement.dataset.windowFullscreen = String(chrome.fullscreen);
    document.documentElement.dataset.windowMode = chrome.floating ? "floating" : "full";

    elements.restoreFullButton.hidden = !chrome.floating;
    elements.hidePanelButton.hidden = !chrome.floating;

    elements.windowControls.hidden = !chrome.customControls;
    if (chrome.customControls || chrome.floating) {
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
    requiredDescendant<SVGElement>(elements.maximizeButton, ".maximize-icon")
      .toggleAttribute("hidden", restore);
    requiredDescendant<SVGElement>(elements.maximizeButton, ".restore-icon")
      .toggleAttribute("hidden", !restore);
  }

  function advanceWindowChromeRequestId(): number {
    state.windowChromeRequestId += 1;
    return state.windowChromeRequestId;
  }

  function isCurrentWindowChromeRequest(requestId: number): boolean {
    return requestId === state.windowChromeRequestId;
  }

  function commitWindowChromeState(raw: unknown): void {
    // Invalidate focus/resize refreshes that began before an explicit window command. Otherwise
    // a late pre-launch response can overwrite the authoritative floating-panel result.
    advanceWindowChromeRequestId();
    renderWindowChromeState(normalizeWindowChromeState(raw));
  }

  async function refreshWindowChromeState() {
    const requestId = advanceWindowChromeRequestId();
    try {
      const raw = await invoke("get_window_chrome_state", {});
      if (isCurrentWindowChromeRequest(requestId)) {
        renderWindowChromeState(normalizeWindowChromeState(raw));
      }
    } catch (_error) {
      if (isCurrentWindowChromeRequest(requestId) && !state.windowChrome) {
        renderWindowChromeState(fallbackWindowChromeState());
      }
    }
  }

  function scheduleWindowChromeRefresh(): void {
    if (state.windowChromeRefreshTimer !== null) {
      window.clearTimeout(state.windowChromeRefreshTimer);
    }
    state.windowChromeRefreshTimer = window.setTimeout(() => {
      state.windowChromeRefreshTimer = null;
      refreshWindowChromeState();
    }, 80);
  }

  function resolveColorTheme(preference: ThemePreference = state.themePreference): ColorTheme {
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

  function storeThemePreference(preference: ThemePreference): void {
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, preference);
    } catch (_error) {
      // The selected appearance still applies for this session when storage is unavailable.
    }
  }

  function applyThemePreference(
    preference: unknown,
    persist = true,
  ): Promise<void> {
    const normalizedPreference: ThemePreference = isThemePreference(preference)
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

  function handleSystemThemeChange(): void {
    if (state.themePreference === "system") {
      syncWindowChromeTheme();
    }
  }

  async function refreshSnapshot(): Promise<void> {
    if (state.refreshPending) {
      return;
    }

    state.refreshPending = true;
    updateRefreshControl();
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
      updateRefreshControl();
    }
  }

  async function refreshUsage(): Promise<void> {
    if (state.usagePending) {
      return;
    }

    state.usagePending = true;
    state.lastUsageAttemptAtMs = Date.now();
    updateRefreshControl();
    try {
      let current: UsageSnapshot;
      try {
        const raw = await invoke("get_usage", {});
        current = normalizeUsageSnapshot(raw);
      } catch (_error) {
        current = unavailableUsageSnapshot("Could not refresh provider usage.");
      }
      const usage = resolveUsageSnapshot(current, state.lastUsage);
      state.lastUsage = usage;
      renderUsageSnapshot(usage);
    } finally {
      state.usagePending = false;
      updateRefreshControl();
    }
  }

  function refreshUsageIfDue(): void {
    if (state.lastUsageAttemptAtMs === null
        || Date.now() - state.lastUsageAttemptAtMs >= USAGE_REFRESH_INTERVAL_MS) {
      refreshUsage();
    }
  }

  function refreshAll() {
    return Promise.allSettled([refreshSnapshot(), refreshUsage()]);
  }

  function updateWorkspaceOpeningState(instanceId: string, opening: boolean): void {
    const button = Array.from(
      elements.workspaceList.querySelectorAll<HTMLButtonElement>(".open-button"),
    )
      .find((candidate) => candidate.dataset.instanceId === instanceId);
    const row = button?.closest(".workspace-row");
    row?.classList.toggle("is-opening", opening);
    button?.setAttribute("aria-busy", String(opening));
  }

  function beginWorkspaceLaunch(workspace: Workspace): void {
    state.openingInstanceId = workspace.instanceId;
    document.documentElement.dataset.workspaceOpening = "true";
    elements.launchStatus.textContent = `Opening ${workspace.name}…`;
    elements.launchOverlay.hidden = false;
    updateWorkspaceOpeningState(workspace.instanceId, true);
  }

  function finishWorkspaceLaunch(instanceId: string): void {
    updateWorkspaceOpeningState(instanceId, false);
    state.openingInstanceId = null;
    delete document.documentElement.dataset.workspaceOpening;
    elements.launchOverlay.hidden = true;
    elements.launchStatus.textContent = "Opening workspace…";
  }

  async function openWorkspace(workspace: Workspace): Promise<void> {
    if (!workspace.openable || state.openingInstanceId) {
      return;
    }

    beginWorkspaceLaunch(workspace);
    const transitionDelay = new Promise<void>((resolve) => {
      window.setTimeout(resolve, LAUNCH_TRANSITION_MIN_MS);
    });

    try {
      const result = await invoke("open_workspace", { instanceId: workspace.instanceId });
      const response = isObject(result) ? result : null;
      if (result === false || response?.ok === false) {
        throw new Error(asString(response?.error, "VS Code did not accept the open request."));
      }
      commitWindowChromeState(result);
      await transitionDelay;
    } catch (error) {
      showNotice(readableError(error, `Could not open ${workspace.name} in VS Code.`));
      await refreshWindowChromeState();
    } finally {
      finishWorkspaceLaunch(workspace.instanceId);
    }
  }

  function getIntegrationElements(kind: IntegrationKind): IntegrationElements {
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

  function renderIntegrationComponent(component: IntegrationComponent): void {
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

  function integrationProgressLabel(
    component: IntegrationComponent,
    operation: IntegrationOperation,
  ): string {
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

  function updateIntegrationControls(): void {
    const status = state.integrationStatus;
    const action = state.integrationAction;
    const busy = state.integrationPending || Boolean(state.integrationAction);
    elements.integrationList.setAttribute("aria-busy", String(busy));
    elements.diagnosticsRefreshButton.disabled = busy || state.setupRefreshPending;
    elements.setupAllButton.disabled = busy || !status;
    elements.setupAllButton.textContent = state.integrationAction?.kind === "all"
      ? "Setting up…"
      : "Set up all";

    INTEGRATION_KINDS.forEach((kind) => {
      const component = status?.[kind];
      const componentElements = getIntegrationElements(kind);
      const isCurrentAction = action?.kind === kind;
      componentElements.card.setAttribute("aria-busy", String(isCurrentAction));
      componentElements.installButton.disabled = busy || !component;
      componentElements.uninstallButton.disabled = busy || !component?.installed;

      if (component) {
        componentElements.installButton.textContent =
          isCurrentAction && action?.operation === "install"
            ? integrationProgressLabel(component, "install")
            : component.actionLabel;
        componentElements.uninstallButton.textContent =
          isCurrentAction && action?.operation === "uninstall"
            ? "Uninstalling…"
            : "Uninstall";
      }
    });
  }

  function updateSetupSummary(): void {
    let summary: string;
    let attention = false;

    if (!state.integrationStatus && !state.diagnosticsLoaded) {
      summary = state.diagnosticsUnavailable
        ? "Unavailable"
        : "Local only";
      attention = state.diagnosticsUnavailable;
    } else {
      const components: IntegrationComponent[] = state.integrationStatus
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

  function renderIntegrationStatus(status: IntegrationStatus): void {
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

  function setIntegrationMessage(
    message: string,
    kind: IntegrationMessageKind = "neutral",
  ): void {
    elements.integrationMessage.textContent = message;
    elements.integrationMessage.hidden = !message;
    elements.integrationMessage.classList.toggle("has-error", kind === "error");
    elements.integrationMessage.classList.toggle("has-success", kind === "success");
  }

  async function refreshIntegrationStatus(): Promise<void> {
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

  function integrationActionSuccess(
    kind: IntegrationKind,
    operation: IntegrationOperation,
  ): string {
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

  async function runIntegrationAction(
    kind: IntegrationKind,
    operation: IntegrationOperation,
  ): Promise<void> {
    if (state.integrationAction) {
      return;
    }

    const commands: Record<
      IntegrationKind,
      { install: TauriCommand; uninstall: TauriCommand; name: string }
    > = {
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

  function formatNaturalList(values: readonly string[]): string {
    if (values.length < 2) {
      return values[0] || "";
    }
    if (values.length === 2) {
      return `${values[0]} and ${values[1]}`;
    }
    return `${values.slice(0, -1).join(", ")}, and ${values.at(-1)}`;
  }

  function trimTerminalPunctuation(value: string): string {
    return value.replace(/[.!?\s]+$/g, "");
  }

  async function setupAllIntegrations(): Promise<void> {
    if (state.integrationAction || state.integrationPending) {
      return;
    }

    const steps: Array<{
      kind: IntegrationKind;
      name: string;
      command: TauriCommand;
    }> = [
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
    const completed: string[] = [];
    const unconfirmed: Array<{ name: string; error: string }> = [];

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

  function openSettingsDialog(): void {
    if (showAccessibleDialog(elements.settingsDialog, elements.settingsCloseButton)) {
      refreshSetup();
    }
  }

  function closeSettingsDialog(): void {
    closeAccessibleDialog(elements.settingsDialog);
  }

  function requestUninstall(kind: IntegrationKind): void {
    if (state.integrationAction) {
      return;
    }

    const componentNames: Record<IntegrationKind, string> = {
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

  function closeUninstallDialog(): void {
    closeAccessibleDialog(elements.uninstallDialog);
  }

  async function confirmUninstall(): Promise<void> {
    const kind = state.pendingUninstall;
    if (!kind) {
      return;
    }

    elements.uninstallConfirmButton.disabled = true;
    closeUninstallDialog();
    state.pendingUninstall = null;
    await runIntegrationAction(kind, "uninstall");
  }

  function formatDuration(milliseconds: unknown): string {
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

  function appendDiagnostic(label: string, value: string, warning = false): void {
    const term = createElement("dt", "", label);
    const description = createElement("dd", warning ? "has-warning" : "", value);
    elements.diagnosticsList.append(term, description);
  }

  function renderDiagnostics(rawValue: unknown): void {
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

  async function refreshDiagnostics(): Promise<void> {
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

  async function refreshSetup(): Promise<[void, void] | undefined> {
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

  async function hideWindow(): Promise<void> {
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

  async function hideFloatingPanel(): Promise<void> {
    elements.hidePanelButton.disabled = true;
    elements.hidePanelButton.setAttribute("aria-busy", "true");
    try {
      await invoke("hide_window", {});
    } catch (error) {
      showNotice(readableError(error, "Could not hide the floating panel."));
    } finally {
      elements.hidePanelButton.disabled = false;
      elements.hidePanelButton.setAttribute("aria-busy", "false");
    }
  }

  async function restoreFullWindow(): Promise<void> {
    elements.restoreFullButton.disabled = true;
    elements.restoreFullButton.setAttribute("aria-busy", "true");
    try {
      const raw = await invoke("restore_full_window", {});
      commitWindowChromeState(raw);
      scheduleWindowChromeRefresh();
    } catch (error) {
      showNotice(readableError(error, "Could not restore the full VSParallel window."));
      await refreshWindowChromeState();
    } finally {
      elements.restoreFullButton.setAttribute("aria-busy", "false");
      elements.restoreFullButton.disabled = false;
    }
  }

  async function toggleWindowMaximize(): Promise<void> {
    if (elements.maximizeButton.disabled) {
      return;
    }

    elements.maximizeButton.disabled = true;
    elements.maximizeButton.setAttribute("aria-busy", "true");
    try {
      const raw = await invoke("toggle_window_maximize", {});
      commitWindowChromeState(raw);
      scheduleWindowChromeRefresh();
    } catch (error) {
      showNotice(readableError(error, "Could not maximize or restore VSParallel."));
    } finally {
      elements.maximizeButton.setAttribute("aria-busy", "false");
      elements.maximizeButton.disabled = state.windowChrome?.fullscreen === true;
    }
  }

  async function closeWindow(): Promise<void> {
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

  function navigateOpenButtons(event: KeyboardEvent): void {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
      return;
    }

    const currentButton = event.target instanceof Element
      ? event.target.closest<HTMLButtonElement>(".open-button:not(:disabled)")
      : null;
    if (!currentButton || !elements.workspaceList.contains(currentButton)) {
      return;
    }

    const buttons = Array.from(
      elements.workspaceList.querySelectorAll<HTMLButtonElement>(
        ".open-button:not(:disabled)",
      ),
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

  function handleGlobalKeydown(event: KeyboardEvent): void {
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
        refreshAll();
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
      const hideAction = state.windowChrome?.floating ? hideFloatingPanel : hideWindow;
      hideAction();
    }
  }

  elements.refreshButton.addEventListener("click", refreshAll);
  elements.emptyRefreshButton.addEventListener("click", refreshSnapshot);
  elements.restoreFullButton.addEventListener("click", restoreFullWindow);
  elements.hidePanelButton.addEventListener("click", hideFloatingPanel);
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
    refreshUsageIfDue();
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
  refreshAll();
  window.setInterval(refreshSnapshot, REFRESH_INTERVAL_MS);
  window.setInterval(refreshUsage, USAGE_REFRESH_INTERVAL_MS);
})();
