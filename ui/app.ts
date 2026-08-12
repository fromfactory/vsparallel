(function () {
  "use strict";

  type JsonObject = Record<string, unknown>;
  type ThemePreference = "system" | "light" | "dark";
  type ColorTheme = Exclude<ThemePreference, "system">;
  type IntegrationKind =
    | "companion"
    | "antigravityIde"
    | "antigravity"
    | "codex"
    | "claude";
  type EditorIntegrationKind = Extract<IntegrationKind, "companion" | "antigravityIde">;
  type IntegrationActionKind = IntegrationKind | "all";
  type IntegrationOperation = "install" | "uninstall";
  type IntegrationVisualState = "missing" | "ready" | "warning" | "error";
  type ActivityKind = "activity" | "finished" | "failure" | "unknown";
  type AntigravityModelKind =
    | "automatic"
    | "gemini"
    | "gemini_3_6_flash_medium"
    | "gemini_3_5_flash"
    | "gemini_3_1_pro_high"
    | "gemini_3_1_pro_low"
    | "gemini_3_flash"
    | "claude"
    | "claude_sonnet_4_6_thinking"
    | "claude_opus_4_6_thinking"
    | "gpt_oss"
    | "gpt_oss_120b";
  type UsageKind = "codex" | "claude";
  type UsageState = "available" | "stale" | "unavailable";
  type NoticeKind = "error" | "warning";
  type IntegrationMessageKind = "neutral" | "error" | "success";
  type UpdatePhase =
    | "idle"
    | "checking"
    | "available"
    | "downloading"
    | "installing"
    | "restarting"
    | "restart-ready"
    | "failed";
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
    | "install_antigravity_hooks"
    | "install_antigravity_ide_companion"
    | "is_release_build"
    | "open_workspace"
    | "restore_full_window"
    | "set_window_chrome_theme"
    | "toggle_window_maximize"
    | "uninstall_claude_hooks"
    | "uninstall_codex_hooks"
    | "uninstall_companion"
    | "uninstall_antigravity_hooks"
    | "uninstall_antigravity_ide_companion";

  interface ActivityView {
    kind: ActivityKind;
    mark: string;
    label: string;
    changedAtMs: number | null;
    detail: string;
    modelKind: AntigravityModelKind | null;
    extensionDetectionAvailable: boolean | null;
    extensionInstalled: boolean | null;
    extensionActive: boolean | null;
    extensionRemote: boolean | null;
  }

  interface Workspace {
    instanceId: string;
    editor: "vscode" | "antigravity_ide" | "antigravity_2";
    editorName: string;
    name: string;
    path: string;
    openable: boolean;
    active: boolean;
    focused: boolean;
    recentlyActive: boolean;
    remoteWindow: boolean;
    lastSeenAtMs: number | null;
    codex: ActivityView;
    claude: ActivityView;
    antigravity: ActivityView | null;
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
    reviewRequired: boolean | null;
  }

  interface IntegrationStatus {
    schemaVersion: number;
    companion: IntegrationComponent;
    antigravityIde: IntegrationComponent;
    antigravity: IntegrationComponent;
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

  interface UpdateDownloadEvent {
    event: "Started" | "Progress" | "Finished";
    data?: {
      contentLength?: number;
      chunkLength?: number;
    };
  }

  interface AvailableUpdate {
    version: string;
    currentVersion: string;
    body?: string;
    date?: string;
    downloadAndInstall: (
      onEvent?: (event: UpdateDownloadEvent) => void,
      options?: { timeout?: number },
    ) => Promise<void>;
  }

  interface TauriUpdaterApi {
    check: (options?: { timeout?: number }) => Promise<AvailableUpdate | null>;
  }

  interface TauriProcessApi {
    relaunch: () => Promise<void>;
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
    updatePhase: UpdatePhase;
    availableUpdate: AvailableUpdate | null;
    dismissedUpdateVersion: string | null;
    updateDownloadedBytes: number;
    updateContentLength: number | null;
    updateMessage: string;
    updateError: string;
    updateChecksEnabled: boolean | null;
  }

  interface TauriWindow extends Window {
    __TAURI__?: {
      core?: {
        invoke?: (command: string, args?: JsonObject) => Promise<unknown>;
      };
      updater?: TauriUpdaterApi;
      process?: TauriProcessApi;
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
  const UPDATE_CHECK_DELAY_MS = 1_500;
  const UPDATE_CHECK_TIMEOUT_MS = 15_000;
  const UPDATE_DOWNLOAD_TIMEOUT_MS = 5 * 60_000;
  const THEME_STORAGE_KEY = "vsparallel.appearance";
  const THEME_PREFERENCES: ReadonlySet<string> = new Set(["system", "light", "dark"]);
  const INTEGRATION_KINDS = [
    "companion",
    "antigravityIde",
    "antigravity",
    "codex",
    "claude",
  ] as const;
  const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000;
  const tauriApi = (window as TauriWindow).__TAURI__;
  const tauriInvoke = tauriApi?.core?.invoke;
  const tauriUpdater = tauriApi?.updater;
  const tauriProcess = tauriApi?.process;
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
    updateBanner: requiredElement<HTMLElement>("#updateBanner"),
    updateVersion: requiredElement<HTMLSpanElement>("#updateVersion"),
    updateStatus: requiredElement<HTMLSpanElement>("#updateStatus"),
    updateProgress: requiredElement<HTMLProgressElement>("#updateProgress"),
    updateNowButton: requiredElement<HTMLButtonElement>("#updateNowButton"),
    updateLaterButton: requiredElement<HTMLButtonElement>("#updateLaterButton"),
    emptyState: requiredElement<HTMLDivElement>("#emptyState"),
    emptyRefreshButton: requiredElement<HTMLButtonElement>("#emptyRefreshButton"),
    launchOverlay: requiredElement<HTMLDivElement>("#launchOverlay"),
    launchStatus: requiredElement<HTMLSpanElement>("#launchStatus"),
    settingsButton: requiredElement<HTMLButtonElement>("#settingsButton"),
    settingsDialog: requiredElement<HTMLDialogElement>("#settingsDialog"),
    settingsCloseButton: requiredElement<HTMLButtonElement>("#settingsCloseButton"),
    checkForUpdatesButton: requiredElement<HTMLButtonElement>("#checkForUpdatesButton"),
    updateCheckStatus: requiredElement<HTMLParagraphElement>("#updateCheckStatus"),
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
    antigravityIdeCard: requiredElement<HTMLElement>("#antigravityIdeCard"),
    antigravityIdeStatus: requiredElement<HTMLSpanElement>("#antigravityIdeStatus"),
    antigravityIdeDetail: requiredElement<HTMLParagraphElement>("#antigravityIdeDetail"),
    antigravityIdeMeta: requiredElement<HTMLParagraphElement>("#antigravityIdeMeta"),
    antigravityIdeInstallButton: requiredElement<HTMLButtonElement>(
      "#antigravityIdeInstallButton",
    ),
    antigravityIdeUninstallButton: requiredElement<HTMLButtonElement>(
      "#antigravityIdeUninstallButton",
    ),
    antigravityCard: requiredElement<HTMLElement>("#antigravityCard"),
    antigravityStatus: requiredElement<HTMLSpanElement>("#antigravityStatus"),
    antigravityDetail: requiredElement<HTMLParagraphElement>("#antigravityDetail"),
    antigravityMeta: requiredElement<HTMLParagraphElement>("#antigravityMeta"),
    antigravityInstallButton: requiredElement<HTMLButtonElement>(
      "#antigravityInstallButton",
    ),
    antigravityUninstallButton: requiredElement<HTMLButtonElement>(
      "#antigravityUninstallButton",
    ),
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
    updatePhase: "idle",
    availableUpdate: null,
    dismissedUpdateVersion: null,
    updateDownloadedBytes: 0,
    updateContentLength: null,
    updateMessage: "Updates are checked quietly after VSParallel starts.",
    updateError: "",
    updateChecksEnabled: null,
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
      optional: kind !== "companion" && kind !== "antigravityIde",
      token,
      visualState,
      installed,
      actionLabel,
      label: asString(raw.label, defaultLabel),
      detail: asString(
        raw.detail,
        kind === "companion"
          ? "VS Code companion status details are unavailable."
          : kind === "antigravityIde"
            ? "Antigravity IDE companion status details are unavailable."
            : kind === "antigravity"
              ? "Antigravity activity hook status details are unavailable."
          : kind === "codex"
            ? "Codex lifecycle hook status details are unavailable."
            : "Claude Code lifecycle hook status details are unavailable.",
      ),
      installedVersion,
      targetVersion: asString(raw.targetVersion),
      configPath: asString(raw.configPath),
      reviewRequired: asNullableBoolean(raw.reviewRequired),
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
      antigravityIde: normalizeIntegrationComponent(raw.antigravityIde, "antigravityIde"),
      antigravity: normalizeIntegrationComponent(raw.antigravity, "antigravity"),
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

  function normalizeAntigravityModelKind(value: unknown): AntigravityModelKind | null {
    const token = normalizeStateToken(value);
    switch (token) {
      case "automatic":
      case "gemini":
      case "gemini_3_6_flash_medium":
      case "gemini_3_5_flash":
      case "gemini_3_1_pro_high":
      case "gemini_3_1_pro_low":
      case "gemini_3_flash":
      case "claude":
      case "claude_sonnet_4_6_thinking":
      case "claude_opus_4_6_thinking":
      case "gpt_oss":
      case "gpt_oss_120b":
        return token;
      default:
        return null;
    }
  }

  function antigravityModelLabel(kind: AntigravityModelKind | null): string {
    switch (kind) {
      case "automatic":
        return "Auto model";
      case "gemini_3_6_flash_medium":
        return "Gemini 3.6 Flash (Medium)";
      case "gemini_3_5_flash":
        return "Gemini 3.5 Flash";
      case "gemini_3_1_pro_high":
        return "Gemini 3.1 Pro (High)";
      case "gemini_3_1_pro_low":
        return "Gemini 3.1 Pro (Low)";
      case "gemini_3_flash":
        return "Gemini 3 Flash";
      case "claude_sonnet_4_6_thinking":
        return "Claude Sonnet 4.6 (Thinking)";
      case "claude_opus_4_6_thinking":
        return "Claude Opus 4.6 (Thinking)";
      case "gpt_oss_120b":
        return "GPT-OSS-120b";
      case "gemini":
        return "Gemini";
      case "claude":
        return "Claude";
      case "gpt_oss":
        return "GPT-OSS";
      default:
        return "";
    }
  }

  function normalizeActivityView(rawValue: unknown): ActivityView {
    const raw = isObject(rawValue) ? rawValue : {};
    const description = describeActivityState(normalizeStateToken(raw.state));
    return {
      ...description,
      label: asString(raw.label, description.label),
      changedAtMs: asTimestamp(raw.changedAtMs),
      detail: asString(raw.detail),
      modelKind: normalizeAntigravityModelKind(raw.modelKind),
      extensionDetectionAvailable: asNullableBoolean(raw.extensionDetectionAvailable),
      extensionInstalled: asNullableBoolean(raw.extensionInstalled),
      extensionActive: asNullableBoolean(raw.extensionActive),
      extensionRemote: asNullableBoolean(raw.extensionRemote),
    };
  }

  function normalizeWorkspace(raw: unknown, index: number): Workspace {
    if (!isObject(raw)) {
      throw new Error(`Workspace record ${index + 1} is not an object.`);
    }

    const instanceId = asString(raw.instanceId);
    const path = asString(raw.path);
    const name = asString(raw.name, deriveName(path) || "Unnamed workspace");
    const editorToken = normalizeStateToken(raw.editor);
    const editor = editorToken === "antigravity_ide" || editorToken === "antigravity_2"
      ? editorToken
      : "vscode";
    const defaultEditorName = editor === "antigravity_ide"
      ? "Antigravity IDE"
      : editor === "antigravity_2"
        ? "Antigravity 2.0"
        : "VS Code";

    return {
      instanceId,
      editor,
      editorName: asString(raw.editorName, defaultEditorName),
      name,
      path,
      openable: raw.openable === true && Boolean(instanceId),
      active: raw.active === true,
      focused: raw.focused === true,
      recentlyActive: raw.recentlyActive === true,
      remoteWindow: raw.remoteWindow === true,
      lastSeenAtMs: asTimestamp(raw.lastSeenAtMs),
      codex: normalizeActivityView(raw.codex),
      claude: normalizeActivityView(raw.claude),
      antigravity: isObject(raw.antigravity)
        ? normalizeActivityView(raw.antigravity)
        : null,
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
    return [workspace.codex, workspace.claude, workspace.antigravity]
      .filter((activity): activity is ActivityView => activity !== null)
      .reduce((current, candidate) =>
      priority[candidate.kind] > priority[current.kind] ? candidate : current,
    );
  }

  function describeExtensionPresence(
    activity: ActivityView,
    remoteWindow = false,
    editorName = "VS Code",
  ): {
    state: string;
    label: string;
    title: string;
  } {
    const hostLabel = activity.extensionRemote === true
      ? " · remote"
      : remoteWindow && activity.extensionRemote === false
        ? " · local"
        : "";
    const hostBoundary = remoteWindow
      ? " VSParallel usage queries and lifecycle hooks run on the desktop host and cannot cross the remote host boundary."
      : "";
    if (activity.extensionDetectionAvailable === false) {
      return {
        state: "unknown",
        label: remoteWindow
          ? "IDE extension status unavailable · remote window"
          : "IDE extension status unavailable",
        title: remoteWindow
          ? `${editorName} extension presence could not be checked for this remote window/profile. Reload the window after updating VSParallel Companion.${hostBoundary}`
          : `${editorName} extension presence could not be checked. Lifecycle state remains independent.`,
      };
    }
    if (activity.extensionInstalled === false && activity.extensionActive === true) {
      return {
        state: "warning",
        label: `IDE extension status inconsistent${hostLabel}`,
        title: `The extension reports active but not installed. Lifecycle state remains independent.${hostBoundary}`,
      };
    }
    if (activity.extensionInstalled === false) {
      return {
        state: "missing",
        label: "IDE extension not detected · this window/profile",
        title: remoteWindow
          ? `The provider IDE extension was not detected in this ${editorName} window/profile. Install or enable it in the intended Local or Remote extension host, then reload the window.${hostBoundary}`
          : `The provider IDE extension was not detected in this ${editorName} window/profile.`,
      };
    }
    if (activity.extensionActive === true) {
      return {
        state: "present",
        label: activity.extensionInstalled === true
          ? `IDE extension active${hostLabel}`
          : `IDE extension active${hostLabel} · install unknown`,
        title: `The IDE extension is active${activity.extensionRemote === true ? " in the remote extension host" : " in this window"}. Activation does not mean an agent turn is running.${hostBoundary}`,
      };
    }
    if (activity.extensionInstalled === true && activity.extensionActive === false) {
      return {
        state: "present",
        label: `IDE extension installed${hostLabel} · inactive`,
        title: `The IDE extension is installed in this ${editorName} window/profile but inactive. This is separate from lifecycle activity.${hostBoundary}`,
      };
    }
    if (activity.extensionInstalled === true) {
      return {
        state: "present",
        label: `IDE extension installed${hostLabel}`,
        title: `The IDE extension is installed in this ${editorName} window/profile. Its activation state is unavailable.${hostBoundary}`,
      };
    }
    return {
      state: "unknown",
      label: `Reload ${editorName} for IDE status`,
      title: `This window is reporting through an older VSParallel Companion. Update the companion in Setup if offered, then run Developer: Reload Window in ${editorName}.`,
    };
  }

  function createProviderState(
    providerName: string,
    activity: ActivityView,
    accessibleProviderName = providerName,
    editorName = "VS Code",
    remoteWindow = false,
    showExtensionPresence = true,
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

    stateLine.append(label, changedAt);
    body.append(stateLine);
    let presenceLabel = "Hook-derived lifecycle activity";
    if (showExtensionPresence) {
      const presence = describeExtensionPresence(activity, remoteWindow, editorName);
      const extension = createElement("span", "provider-extension", presence.label);
      extension.dataset.state = presence.state;
      extension.title = presence.title;
      body.append(extension);
      presenceLabel = presence.label;
    }
    provider.append(name, body);
    provider.setAttribute(
      "aria-label",
      `${accessibleProviderName}: ${activity.label}, ${relativeTime}. ${presenceLabel}.`,
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
    } else if (workspace.recentlyActive && !workspace.openable) {
      windowState = "Recent";
      windowStateToken = "recent";
    }

    const editorBadge = createElement("span", "editor-badge", workspace.editorName);
    editorBadge.dataset.editor = workspace.editor;
    editorBadge.title = `Tracked by ${workspace.editorName}`;
    const windowBadge = createElement("span", "window-badge", windowState);
    windowBadge.dataset.state = windowStateToken;
    titleLine.append(name, windowBadge);

    const path = createElement("span", "workspace-path", formatShortPath(workspace.path));
    if (workspace.path) {
      path.title = workspace.path;
    }
    const metaLine = createElement("div", "workspace-meta");
    metaLine.append(editorBadge, path);
    primary.append(titleLine, metaLine);
    row.append(primary);

    const providers = createElement("div", "activity-providers");
    providers.setAttribute("aria-label", "Agent lifecycle and IDE extension status");
    if (workspace.antigravity) {
      const modelLabel = antigravityModelLabel(workspace.antigravity.modelKind);
      providers.append(
        createProviderState(
          modelLabel || "Antigravity",
          workspace.antigravity,
          modelLabel
            ? `${modelLabel}, latest model reported by Antigravity`
            : "Antigravity",
          workspace.editorName,
          false,
          false,
        ),
      );
    }
    if (workspace.editor !== "antigravity_2") {
      providers.append(
        createProviderState(
          "Codex",
          workspace.codex,
          "Codex",
          workspace.editorName,
          workspace.remoteWindow,
        ),
        createProviderState(
          "Claude",
          workspace.claude,
          "Claude Code",
          workspace.editorName,
          workspace.remoteWindow,
        ),
      );
    }
    row.append(providers);

    const openButton = createElement("button", "open-button");
    const actionLabel = workspace.active ? "Switch to" : "Open";
    openButton.type = "button";
    openButton.dataset.instanceId = workspace.instanceId;
    openButton.disabled = !openable;
    openButton.setAttribute(
      "aria-label",
      `${actionLabel} ${workspace.name} in ${workspace.editorName}`,
    );
    openButton.setAttribute("aria-busy", String(opening));
    if (!workspace.openable) {
      openButton.title = workspace.recentlyActive
        ? `${workspace.editorName} hook activity does not identify a live window or exact open target`
        : "This workspace cannot currently be opened";
    } else {
      openButton.title = `${actionLabel} ${workspace.name} in ${workspace.editorName}`;
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

  function formatUpdateVersion(version: string): string {
    const normalized = version.trim();
    return normalized.startsWith("v") ? normalized : `v${normalized}`;
  }

  function formatByteCount(bytes: number): string {
    const normalized = Math.max(0, bytes);
    if (normalized < 1_024) {
      return `${Math.round(normalized)} B`;
    }
    if (normalized < 1_048_576) {
      return `${(normalized / 1_024).toFixed(1)} KB`;
    }
    return `${(normalized / 1_048_576).toFixed(1)} MB`;
  }

  function updateProgressPercent(downloaded: number, contentLength: number | null): number | null {
    if (contentLength === null || contentLength <= 0) {
      return null;
    }
    return Math.min(100, Math.max(0, (downloaded / contentLength) * 100));
  }

  function updatePhaseIsBusy(phase: UpdatePhase): boolean {
    return ["checking", "downloading", "installing", "restarting"].includes(phase);
  }

  function renderUpdateState(): void {
    const update = state.availableUpdate;
    const busy = updatePhaseIsBusy(state.updatePhase);
    const dismissed = Boolean(
      update && state.dismissedUpdateVersion === update.version,
    );
    const bannerVisible = Boolean(
      update && (!dismissed || busy || state.updatePhase === "restart-ready"),
    );

    elements.updateBanner.hidden = !bannerVisible;
    elements.updateBanner.dataset.state = state.updatePhase;
    elements.updateBanner.setAttribute("aria-busy", String(busy));
    elements.checkForUpdatesButton.disabled = busy;
    elements.checkForUpdatesButton.classList.toggle(
      "is-loading",
      state.updatePhase === "checking",
    );
    elements.checkForUpdatesButton.setAttribute(
      "aria-busy",
      String(state.updatePhase === "checking"),
    );
    elements.updateCheckStatus.classList.toggle(
      "has-error",
      state.updatePhase === "failed",
    );

    if (!update) {
      elements.updateCheckStatus.textContent = state.updatePhase === "checking"
        ? "Checking for a newer version…"
        : state.updateMessage;
      return;
    }

    const version = formatUpdateVersion(update.version);
    elements.updateVersion.textContent = version;
    elements.updateNowButton.disabled = busy;
    elements.updateLaterButton.hidden = state.updatePhase === "restart-ready";
    elements.updateLaterButton.disabled = busy || state.updatePhase === "restart-ready";
    elements.updateProgress.hidden = state.updatePhase !== "downloading";

    let status = "Ready to download and install.";
    let settingsStatus = `Version ${version} is available.`;
    let actionLabel = "Update now";
    if (state.updatePhase === "downloading") {
      const percent = updateProgressPercent(
        state.updateDownloadedBytes,
        state.updateContentLength,
      );
      if (percent === null) {
        elements.updateProgress.removeAttribute("value");
        status = `Downloading… ${formatByteCount(state.updateDownloadedBytes)}`;
      } else {
        elements.updateProgress.value = percent;
        status = `Downloading… ${Math.round(percent)}%`;
      }
      settingsStatus = `Downloading version ${version}…`;
      actionLabel = "Downloading…";
    } else if (state.updatePhase === "installing") {
      status = "Download complete. Installing update…";
      settingsStatus = `Installing version ${version}…`;
      actionLabel = "Installing…";
    } else if (state.updatePhase === "restarting") {
      status = "Update installed. Restarting VSParallel…";
      settingsStatus = `Version ${version} is installed. Restarting…`;
      actionLabel = "Restarting…";
    } else if (state.updatePhase === "restart-ready") {
      status = "Update installed. Restart VSParallel to finish.";
      settingsStatus = `Version ${version} is installed and ready to restart.`;
      actionLabel = "Restart now";
    } else if (state.updatePhase === "failed") {
      status = state.updateError || "The update could not be installed.";
      settingsStatus = status;
      actionLabel = "Try again";
    }

    elements.updateStatus.textContent = status;
    elements.updateCheckStatus.textContent = settingsStatus;
    elements.updateNowButton.textContent = actionLabel;
  }

  async function checkForUpdates(manual = false): Promise<void> {
    if (updatePhaseIsBusy(state.updatePhase)) {
      return;
    }

    if (state.availableUpdate) {
      state.dismissedUpdateVersion = null;
      if (state.updatePhase === "failed") {
        state.updatePhase = "available";
        state.updateError = "";
      }
      renderUpdateState();
      if (manual) {
        closeSettingsDialog();
      }
      return;
    }

    if (!tauriUpdater?.check) {
      if (manual) {
        state.updatePhase = "failed";
        state.updateMessage = "Update checking is unavailable in this build.";
        renderUpdateState();
      }
      return;
    }

    if (state.updateChecksEnabled === null) {
      try {
        state.updateChecksEnabled = await invoke("is_release_build", {}) === true;
      } catch (_error) {
        state.updateChecksEnabled = false;
      }
    }
    if (!state.updateChecksEnabled) {
      if (manual) {
        state.updatePhase = "idle";
        state.updateMessage = "Update checks are available in installed release builds.";
        renderUpdateState();
      }
      return;
    }

    state.updatePhase = "checking";
    state.updateMessage = "Checking for a newer version…";
    state.updateError = "";
    renderUpdateState();

    try {
      const update = await tauriUpdater.check({ timeout: UPDATE_CHECK_TIMEOUT_MS });
      if (!update) {
        state.updatePhase = "idle";
        state.updateMessage = manual
          ? "VSParallel is up to date."
          : "Updates are checked quietly after VSParallel starts.";
        renderUpdateState();
        return;
      }
      if (!asString(update.version) || typeof update.downloadAndInstall !== "function") {
        throw new Error("The update service returned invalid metadata.");
      }

      state.availableUpdate = update;
      state.dismissedUpdateVersion = null;
      state.updateDownloadedBytes = 0;
      state.updateContentLength = null;
      state.updatePhase = "available";
      state.updateMessage = `Version ${formatUpdateVersion(update.version)} is available.`;
      renderUpdateState();
      if (manual) {
        closeSettingsDialog();
      }
    } catch (_error) {
      state.updatePhase = manual ? "failed" : "idle";
      state.updateMessage = manual
        ? "Could not check for updates right now."
        : "Updates are checked quietly after VSParallel starts.";
      renderUpdateState();
    }
  }

  function handleUpdateDownloadEvent(event: UpdateDownloadEvent): void {
    if (event.event === "Started") {
      const contentLength = asFiniteNumber(event.data?.contentLength);
      state.updateContentLength = contentLength !== null && contentLength > 0
        ? contentLength
        : null;
      state.updateDownloadedBytes = 0;
      state.updatePhase = "downloading";
    } else if (event.event === "Progress") {
      const chunkLength = asFiniteNumber(event.data?.chunkLength, 0) ?? 0;
      state.updateDownloadedBytes += Math.max(0, chunkLength);
      state.updatePhase = "downloading";
    } else if (event.event === "Finished") {
      state.updatePhase = "installing";
    }
    renderUpdateState();
  }

  async function restartAfterUpdate(): Promise<void> {
    if (!tauriProcess?.relaunch) {
      state.updatePhase = "restart-ready";
      renderUpdateState();
      return;
    }

    state.updatePhase = "restarting";
    renderUpdateState();
    try {
      await tauriProcess.relaunch();
    } catch (_error) {
      state.updatePhase = "restart-ready";
      renderUpdateState();
    }
  }

  async function installAvailableUpdate(): Promise<void> {
    if (state.updatePhase === "restart-ready") {
      await restartAfterUpdate();
      return;
    }
    if (!state.availableUpdate || updatePhaseIsBusy(state.updatePhase)) {
      return;
    }

    state.updatePhase = "downloading";
    state.updateDownloadedBytes = 0;
    state.updateContentLength = null;
    state.updateError = "";
    renderUpdateState();

    try {
      await state.availableUpdate.downloadAndInstall(handleUpdateDownloadEvent, {
        timeout: UPDATE_DOWNLOAD_TIMEOUT_MS,
      });
    } catch (_error) {
      state.updatePhase = "failed";
      state.updateError = "Could not download or install the update. Try again later.";
      renderUpdateState();
      return;
    }

    await restartAfterUpdate();
  }

  function deferAvailableUpdate(): void {
    if (!state.availableUpdate || updatePhaseIsBusy(state.updatePhase)) {
      return;
    }
    state.dismissedUpdateVersion = state.availableUpdate.version;
    renderUpdateState();
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
    elements.launchStatus.textContent = `Opening ${workspace.name} in ${workspace.editorName}…`;
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
        throw new Error(
          asString(response?.error, `${workspace.editorName} did not accept the open request.`),
        );
      }
      commitWindowChromeState(result);
      await transitionDelay;
    } catch (error) {
      showNotice(
        readableError(error, `Could not open ${workspace.name} in ${workspace.editorName}.`),
      );
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

    if (kind === "antigravityIde") {
      return {
        card: elements.antigravityIdeCard,
        status: elements.antigravityIdeStatus,
        detail: elements.antigravityIdeDetail,
        meta: elements.antigravityIdeMeta,
        installButton: elements.antigravityIdeInstallButton,
        uninstallButton: elements.antigravityIdeUninstallButton,
      };
    }

    if (kind === "antigravity") {
      return {
        card: elements.antigravityCard,
        status: elements.antigravityStatus,
        detail: elements.antigravityDetail,
        meta: elements.antigravityMeta,
        installButton: elements.antigravityInstallButton,
        uninstallButton: elements.antigravityUninstallButton,
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
      : component.kind === "antigravityIde"
        ? "Antigravity IDE companion"
        : component.kind === "antigravity"
          ? "Antigravity activity hooks"
      : component.kind === "codex"
        ? "Codex activity hooks"
        : "Claude Code activity hooks";
    componentElements.installButton.setAttribute(
      "aria-label",
      `${component.actionLabel} ${componentName}`,
    );
    componentElements.uninstallButton.hidden = !component.installed;

    let meta = "";
    if (component.kind === "companion" || component.kind === "antigravityIde") {
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
      : "Set up monitoring";

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

  function summarizeEditorCompanions(status: IntegrationStatus): {
    installed: boolean;
    warningCount: number;
  } {
    const companions = [status.companion, status.antigravityIde];
    return {
      installed: companions.some((component) => component.installed),
      warningCount: companions.filter((component) =>
        ["warning", "error"].includes(component.visualState)
        && component.token !== "unavailable"
      ).length,
    };
  }

  function describeSetupSummary(
    status: IntegrationStatus | null,
    diagnosticsLoaded: boolean,
    diagnosticsUnavailable: boolean,
    diagnosticWarningCount: number,
  ): { summary: string; attention: boolean } {
    if (!status && !diagnosticsLoaded) {
      return {
        summary: diagnosticsUnavailable ? "Unavailable" : "Local only",
        attention: diagnosticsUnavailable,
      };
    }

    if (!status) {
      return { summary: "Partially checked", attention: diagnosticWarningCount > 0 };
    }

    const editorSummary = summarizeEditorCompanions(status);
    const optionalComponents = [status.antigravity, status.codex, status.claude];
    const optionalMissing = optionalComponents.some(
      (component) => component.visualState === "missing",
    );
    const optionalWarnings = optionalComponents.filter((component) =>
      ["warning", "error"].includes(component.visualState)
    ).length;
    const totalWarnings = editorSummary.warningCount
      + optionalWarnings
      + diagnosticWarningCount;

    if (!editorSummary.installed) {
      return { summary: "Editor setup needed", attention: true };
    }
    if (totalWarnings) {
      return {
        summary: `${totalWarnings} warning${totalWarnings === 1 ? "" : "s"}`,
        attention: true,
      };
    }
    if (optionalMissing) {
      return { summary: "Optional setup", attention: false };
    }
    return {
      summary: diagnosticsLoaded ? "Ready" : "Integrations ready",
      attention: false,
    };
  }

  function updateSetupSummary(): void {
    const { summary, attention } = describeSetupSummary(
      state.integrationStatus,
      state.diagnosticsLoaded,
      state.diagnosticsUnavailable,
      state.diagnosticWarningCount,
    );

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
    renderIntegrationComponent(status.antigravityIde);
    renderIntegrationComponent(status.antigravity);
    renderIntegrationComponent(status.codex);
    renderIntegrationComponent(status.claude);
    elements.restartNotice.hidden = !status.requiresRestart;
    const codexReviewRequired = status.codex.reviewRequired === true;
    elements.codexTrustGuidance.dataset.active = String(codexReviewRequired);
    elements.codexTrustGuidance.hidden = !codexReviewRequired;
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
          antigravityIde: { state: "error", label: "Check failed", detail: message },
          antigravity: { state: "error", label: "Check failed", detail: message },
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
      if (kind === "antigravityIde") {
        return "Antigravity IDE companion uninstalled. Existing stale heartbeats will age out automatically.";
      }
      const provider = kind === "antigravity"
        ? "Antigravity"
        : kind === "codex"
          ? "Codex"
          : "Claude Code";
      return `${provider} activity hooks uninstalled. Existing stale activity markers will age out automatically.`;
    }
    if (kind === "companion") {
      return "VS Code companion installed. Reload open VS Code windows to start reporting heartbeats.";
    }
    if (kind === "antigravityIde") {
      return "Antigravity IDE companion installed. Reload open Antigravity IDE windows to start reporting heartbeats.";
    }
    if (kind === "antigravity") {
      return "Antigravity activity hooks installed. In Antigravity 2.0, open a saved Project and start an agent turn; opening the Project alone does not fire a hook.";
    }
    return kind === "codex"
      ? "Codex activity hooks installed. Usage remaining is separate; see the requirement above. In Codex, run /hooks and complete the required security review."
      : "Claude Code activity hooks installed. Usage remaining is separate; see the requirement above. Restart affected Claude Code sessions to load the new lifecycle handlers.";
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
      antigravityIde: {
        install: "install_antigravity_ide_companion",
        uninstall: "uninstall_antigravity_ide_companion",
        name: "Antigravity IDE companion",
      },
      antigravity: {
        install: "install_antigravity_hooks",
        uninstall: "uninstall_antigravity_hooks",
        name: "Antigravity activity hooks",
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

  function availableEditorSetupKinds(status: IntegrationStatus): EditorIntegrationKind[] {
    const kinds: EditorIntegrationKind[] = [];
    if (status.companion.token !== "unavailable") {
      kinds.push("companion");
    }
    if (status.antigravityIde.token !== "unavailable") {
      kinds.push("antigravityIde");
    }
    return kinds;
  }

  async function setupAllIntegrations(): Promise<void> {
    const integrationStatus = state.integrationStatus;
    if (state.integrationAction || state.integrationPending || !integrationStatus) {
      return;
    }

    const editorKinds = availableEditorSetupKinds(integrationStatus);
    const editorSteps = editorKinds.map((kind) => kind === "companion"
      ? {
          kind,
          name: "VS Code companion",
          editorName: "VS Code",
          command: "install_companion" as const,
        }
      : {
          kind,
          name: "Antigravity IDE companion",
          editorName: "Antigravity IDE",
          command: "install_antigravity_ide_companion" as const,
        });
    const steps: Array<{
      kind: IntegrationKind;
      name: string;
      command: TauriCommand;
    }> = [
      ...editorSteps,
      {
        kind: "antigravity",
        name: "Antigravity activity hooks",
        command: "install_antigravity_hooks",
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
      const editorNames = editorSteps.map((step) => step.editorName);
      const successMessage = editorNames.length
        ? `${formatNaturalList(editorNames)} companion${editorNames.length === 1 ? "" : "s"} and activity hooks are installed. Reload ${formatNaturalList(editorNames)}, restart affected provider sessions, then run /hooks in Codex and complete its required security review.`
        : "Activity hooks are installed. No available editor companion CLI was detected, so editor setup was skipped. Restart affected provider sessions, then run /hooks in Codex and complete its required security review.";
      setIntegrationMessage(
        successMessage,
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
      antigravityIde: "Antigravity IDE companion",
      antigravity: "Antigravity activity hooks",
      codex: "Codex activity hooks",
      claude: "Claude Code activity hooks",
    };
    state.pendingUninstall = kind;
    elements.uninstallTitle.textContent = `Uninstall ${componentNames[kind]}?`;
    if (kind === "companion" || kind === "antigravityIde") {
      const editorName = kind === "companion" ? "VS Code" : "Antigravity IDE";
      elements.uninstallDescription.textContent = `VSParallel will stop receiving workspace heartbeats after existing ${editorName} windows are reloaded. No project files are removed.`;
    } else {
      const provider = kind === "antigravity"
        ? "Antigravity"
        : kind === "codex"
          ? "Codex"
          : "Claude Code";
      elements.uninstallDescription.textContent = `VSParallel will remove only its own ${provider} handlers. Other hooks remain unchanged, and no project files are removed.`;
    }
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
    const malformedAntigravity = asNonNegativeInteger(raw.malformedAntigravityRecords);
    const omittedInstances = asNonNegativeInteger(raw.omittedInstanceRecords);
    const omittedCodex = asNonNegativeInteger(raw.omittedCodexRecords);
    const omittedClaude = asNonNegativeInteger(raw.omittedClaudeRecords);
    const omittedAntigravity = asNonNegativeInteger(raw.omittedAntigravityRecords);
    const validInstances = asNonNegativeInteger(raw.validInstanceRecords);
    const validCodex = asNonNegativeInteger(raw.validCodexRecords);
    const validClaude = asNonNegativeInteger(raw.validClaudeRecords);
    const validAntigravity = asNonNegativeInteger(raw.validAntigravityRecords);
    const antigravityHookOutcome = normalizeStateToken(raw.antigravityTwoHookOutcome);
    const antigravityHookEvent = asString(raw.antigravityTwoHookEvent)
      .replaceAll("-", " ");
    const antigravityHookObservedAt = asTimestamp(raw.antigravityTwoHookObservedAtMs);
    const antigravityHookWorkspaceCount = asNonNegativeInteger(
      raw.antigravityTwoHookWorkspaceCount,
    );
    const antigravityHookWarning = ![
      "not_observed",
      "recorded",
    ].includes(antigravityHookOutcome);
    let antigravityHookDetail = "Not observed · start an Antigravity 2.0 agent turn";
    if (antigravityHookOutcome === "recorded") {
      const observed = formatRelativeTime(antigravityHookObservedAt).toLowerCase();
      antigravityHookDetail = [
        antigravityHookEvent || "agent event",
        observed,
        `${antigravityHookWorkspaceCount} workspace path${
          antigravityHookWorkspaceCount === 1 ? "" : "s"
        } recorded`,
      ].join(" · ");
    } else if (antigravityHookOutcome === "no_workspace") {
      antigravityHookDetail = "Observed, but no usable local Project workspace path was reported";
    } else if (antigravityHookOutcome === "missing_conversation") {
      antigravityHookDetail = "Observed, but no conversation identifier was reported";
    } else if (antigravityHookOutcome === "persist_failed") {
      antigravityHookDetail = "Observed, but VSParallel could not save workspace activity";
    } else if (antigravityHookOutcome === "health_unreadable") {
      antigravityHookDetail = "The local hook execution-health record is unreadable";
    } else if (antigravityHookOutcome !== "not_observed") {
      antigravityHookDetail = "Observed, but the event could not be used for workspace activity";
    }
    const totalMalformed = malformedInstances
      + malformedCodex
      + malformedClaude
      + malformedAntigravity;
    const totalOmitted = omittedInstances + omittedCodex + omittedClaude + omittedAntigravity;

    elements.diagnosticsList.replaceChildren();
    appendDiagnostic("State directory", asString(raw.stateDirectory, "Unavailable"));
    appendDiagnostic("VS Code command", asString(raw.codeCommand, "code"));
    appendDiagnostic(
      "Antigravity IDE command",
      asString(raw.antigravityIdeCommand, "antigravity-ide"),
    );
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
    appendDiagnostic(
      "Antigravity activity records",
      `${validAntigravity} valid · ${malformedAntigravity} malformed · ${omittedAntigravity} omitted`,
      malformedAntigravity > 0 || omittedAntigravity > 0,
    );
    appendDiagnostic(
      "Antigravity 2.0 hook execution",
      antigravityHookDetail,
      antigravityHookWarning,
    );
    appendDiagnostic("Active heartbeat window", formatDuration(raw.activeTtlMs));
    appendDiagnostic("Inactive record retention", formatDuration(raw.staleRetentionMs));
    appendDiagnostic(
      "Activity freshness",
      formatDuration(raw.activityStaleMs ?? raw.codexStaleMs),
    );

    state.diagnosticWarningCount = totalMalformed
      + Number(totalOmitted > 0)
      + Number(antigravityHookWarning);
    state.diagnosticsLoaded = true;
    state.diagnosticsUnavailable = false;
    updateSetupSummary();
    elements.diagnosticsStatus.classList.remove("has-error");
    elements.diagnosticsStatus.textContent = validInstances || validAntigravity
      ? "One or more editor heartbeats or Antigravity activity records are available."
      : antigravityHookOutcome === "not_observed"
        ? "No workspace source is present yet. Opening an Antigravity 2.0 Project does not fire hooks; start an agent turn to create recent activity."
        : antigravityHookWarning
          ? "Antigravity 2.0 invoked VSParallel, but the latest event did not create a workspace record."
          : "No valid workspace source is present yet. Check an editor companion or Antigravity activity hooks.";
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
  elements.checkForUpdatesButton.addEventListener("click", () => checkForUpdates(true));
  elements.updateNowButton.addEventListener("click", installAvailableUpdate);
  elements.updateLaterButton.addEventListener("click", deferAvailableUpdate);
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
  elements.antigravityIdeInstallButton.addEventListener("click", () =>
    runIntegrationAction("antigravityIde", "install"),
  );
  elements.antigravityInstallButton.addEventListener("click", () =>
    runIntegrationAction("antigravity", "install"),
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
  elements.antigravityIdeUninstallButton.addEventListener("click", () =>
    requestUninstall("antigravityIde"),
  );
  elements.antigravityUninstallButton.addEventListener("click", () =>
    requestUninstall("antigravity"),
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
    if (isDialogOpen(elements.settingsDialog)) {
      refreshIntegrationStatus();
    }
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
  renderUpdateState();
  refreshIntegrationStatus();
  refreshAll();
  window.setTimeout(() => {
    void checkForUpdates(false);
  }, UPDATE_CHECK_DELAY_MS);
  window.setInterval(refreshSnapshot, REFRESH_INTERVAL_MS);
  window.setInterval(refreshUsage, USAGE_REFRESH_INTERVAL_MS);
})();
