(function () {
  "use strict";

  type JsonObject = Record<string, unknown>;
  type ThemePreference = "system" | "light" | "dark";
  type ColorTheme = Exclude<ThemePreference, "system">;
  type IntegrationKind =
    | "companion"
    | "cursorCompanion"
    | "antigravityIde"
    | "cursor"
    | "antigravity"
    | "gemini"
    | "codex"
    | "claude";
  type EditorIntegrationKind = Extract<
    IntegrationKind,
    "companion" | "cursorCompanion" | "antigravityIde"
  >;
  type VisibleIntegrationKind = EditorIntegrationKind | "gemini" | "codex" | "claude";
  type EditorVisibilityKind = "vscode" | "cursor" | "antigravity" | "zed";
  type IntegrationActionKind = IntegrationKind | "all";
  type IntegrationOperation = "install" | "uninstall";
  type IntegrationVisualState = "missing" | "ready" | "warning" | "error";
  type ActivityKind = "activity" | "finished" | "failure" | "recent" | "unknown";
  type WorkspaceSurface = "editor_workspace" | "hook_only" | "cursor_agent_thread";
  type CursorAgentsBridgeAvailability =
    | "disabled"
    | "connected"
    | "waiting"
    | "unsupported"
    | "error";
  type AntigravityModelKind =
    | "automatic"
    | "gemini"
    | "gemini_3_6_flash_medium"
    | "gemini_3_6_flash_high"
    | "gemini_3_5_flash"
    | "gemini_3_1_pro_high"
    | "gemini_3_1_pro_low"
    | "gemini_3_flash"
    | "claude"
    | "claude_sonnet_4_6_thinking"
    | "claude_opus_4_6_thinking"
    | "gpt_oss"
    | "gpt_oss_120b"
    | "gpt_oss_120b_medium";
  type UsageKind = "codex" | "claude" | "gemini" | "antigravity" | "zed" | "cursor";
  type UsageState = "available" | "stale" | "unavailable";
  type UsageMetricKind = "quota" | "context" | "tokens" | "none";
  type NoticeKind = "error" | "warning";
  type IntegrationMessageKind = "neutral" | "warning" | "error" | "success";
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
    | "get_cursor_agents_bridge_status"
    | "get_diagnostics"
    | "get_display_preferences"
    | "get_integration_status"
    | "get_snapshot"
    | "get_usage"
    | "get_window_chrome_state"
    | "hide_window"
    | "install_claude_hooks"
    | "install_codex_hooks"
    | "install_companion"
    | "install_cursor_companion"
    | "install_cursor_hooks"
    | "install_cursor_monitoring"
    | "uninstall_cursor_monitoring"
    | "install_antigravity_hooks"
    | "install_antigravity_ide_companion"
    | "install_antigravity_monitoring"
    | "install_gemini_usage"
    | "is_release_build"
    | "open_workspace"
    | "restore_full_window"
    | "set_cursor_agents_monitoring_enabled"
    | "set_editor_visibility"
    | "set_usage_limit_percentage_visible"
    | "set_window_chrome_theme"
    | "toggle_window_maximize"
    | "uninstall_claude_hooks"
    | "uninstall_codex_hooks"
    | "uninstall_companion"
    | "uninstall_cursor_companion"
    | "uninstall_cursor_hooks"
    | "uninstall_antigravity_hooks"
    | "uninstall_antigravity_ide_companion"
    | "uninstall_antigravity_monitoring"
    | "uninstall_gemini_usage"
    | "uninstall_all_integrations";

  interface ActivityView {
    kind: ActivityKind;
    label: string;
    changedAtMs: number | null;
    detail: string;
    modelKind: AntigravityModelKind | null;
    modelName: string;
    agentKind: string;
    extensionDetectionAvailable: boolean | null;
    extensionInstalled: boolean | null;
    extensionActive: boolean | null;
    extensionRemote: boolean | null;
  }

  interface Workspace {
    instanceId: string;
    editor: "vscode" | "cursor" | "antigravity_ide" | "antigravity_2" | "zed";
    editorName: string;
    surface: WorkspaceSurface;
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
    cursor: ActivityView | null;
    zed: ActivityView | null;
  }

  interface WorkspaceGroup {
    kind: "open" | "recent";
    label: "Open" | "Recent";
    workspaces: Workspace[];
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
    metricKind: UsageMetricKind;
    remainingPercent: number | null;
    tokenCount: number | null;
    metricLabel: string;
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
    gemini: UsageProvider;
    antigravity: UsageProvider;
    zed: UsageProvider;
    cursor: UsageProvider;
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
    cursorCompanion: IntegrationComponent;
    antigravityIde: IntegrationComponent;
    cursor: IntegrationComponent;
    antigravity: IntegrationComponent;
    gemini: IntegrationComponent;
    codex: IntegrationComponent;
    claude: IntegrationComponent;
    requiresRestart: boolean;
  }

  interface CursorAgentsBridgeStatus {
    schemaVersion: 1;
    enabled: boolean;
    availability: CursorAgentsBridgeAvailability;
    connected: boolean;
    instanceCount: number;
    threadCount: number;
    lastCheckedAtMs: number | null;
    errorCode: string;
    detail: string;
  }

  interface IntegrationAction {
    kind: IntegrationActionKind;
    operation: IntegrationOperation;
  }

  interface IntegrationElements {
    card: HTMLElement;
    status: HTMLSpanElement;
    detail: HTMLParagraphElement;
    helpDetail: HTMLSpanElement;
    meta: HTMLParagraphElement;
    installButton: HTMLButtonElement;
    uninstallButton: HTMLButtonElement;
  }

  interface UsageElements {
    card: HTMLElement;
    value: HTMLElement;
    stateLabel: HTMLSpanElement;
    meter: HTMLDivElement;
    detail: HTMLSpanElement;
  }

  interface VisibilityPreferences {
    editors: Record<EditorVisibilityKind, boolean>;
    usage: boolean;
  }

  interface DisplayPreferencesResponse {
    schemaVersion: 1;
    editors: Record<EditorVisibilityKind, boolean>;
    usageLimitPercentage: boolean;
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
    usageRefreshPromise: Promise<void> | null;
    usageRefreshGeneration: number;
    diagnosticsPending: boolean;
    diagnosticsLoaded: boolean;
    diagnosticsUnavailable: boolean;
    diagnosticWarningCount: number;
    diagnosticWarnings: string[];
    setupRefreshPending: boolean;
    setupRefreshPromise: Promise<[void, void, void, void]> | null;
    integrationPending: boolean;
    integrationLoaded: boolean;
    integrationStatus: IntegrationStatus | null;
    integrationAction: IntegrationAction | null;
    cursorAgentsBridgePending: boolean;
    cursorAgentsBridgeStatus: CursorAgentsBridgeStatus | null;
    pendingUninstall: IntegrationActionKind | null;
    openingInstanceId: string | null;
    lastGoodSnapshot: Snapshot | null;
    lastUsage: UsageSnapshot | null;
    lastUsageAttemptAtMs: number | null;
    windowChrome: WindowChromeState | null;
    windowChromeRequestId: number;
    windowChromeRefreshTimer: number | null;
    themePreference: ThemePreference;
    editorVisibility: Record<EditorVisibilityKind, boolean>;
    usageVisible: boolean;
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
  const VISIBILITY_STORAGE_KEY = "vsparallel.visibility";
  const THEME_PREFERENCES: ReadonlySet<string> = new Set(["system", "light", "dark"]);
  const INTEGRATION_KINDS = [
    "companion",
    "cursorCompanion",
    "antigravityIde",
    "cursor",
    "antigravity",
    "gemini",
    "codex",
    "claude",
  ] as const;
  const VISIBLE_INTEGRATION_KINDS = [
    "companion",
    "cursorCompanion",
    "antigravityIde",
    "gemini",
    "codex",
    "claude",
  ] as const satisfies readonly VisibleIntegrationKind[];
  const EDITOR_VISIBILITY_KINDS = [
    "vscode",
    "cursor",
    "antigravity",
    "zed",
  ] as const satisfies readonly EditorVisibilityKind[];
  const MAX_JAVASCRIPT_TIMESTAMP_MS = 8_640_000_000_000_000;
  const tauriApi = (window as TauriWindow).__TAURI__;
  const tauriInvoke = tauriApi?.core?.invoke;
  const tauriUpdater = tauriApi?.updater;
  const tauriProcess = tauriApi?.process;
  const lightThemeQuery = window.matchMedia("(prefers-color-scheme: light)");

  function defaultVisibilityPreferences(): VisibilityPreferences {
    return {
      editors: {
        vscode: true,
        cursor: true,
        antigravity: true,
        zed: true,
      },
      usage: true,
    };
  }

  function loadVisibilityPreferences(): VisibilityPreferences {
    const preferences = defaultVisibilityPreferences();
    try {
      const rawValue = window.localStorage.getItem(VISIBILITY_STORAGE_KEY);
      if (!rawValue) {
        return preferences;
      }
      const raw = JSON.parse(rawValue) as unknown;
      if (!isObject(raw)) {
        return preferences;
      }
      const rawEditors = isObject(raw.editors) ? raw.editors : {};
      EDITOR_VISIBILITY_KINDS.forEach((kind) => {
        if (typeof rawEditors[kind] === "boolean") {
          preferences.editors[kind] = rawEditors[kind];
        }
      });
      if (typeof raw.usage === "boolean") {
        preferences.usage = raw.usage;
      }
    } catch (_error) {
      // Visibility defaults remain enabled when a stored preference is unreadable.
    }
    return preferences;
  }

  function normalizeDisplayPreferences(rawValue: unknown): DisplayPreferencesResponse {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw) || raw.schemaVersion !== SCHEMA_VERSION || !isObject(raw.editors)) {
      throw new Error("VSParallel returned invalid display preferences.");
    }
    const rawEditors = raw.editors;
    const editors = defaultVisibilityPreferences().editors;
    EDITOR_VISIBILITY_KINDS.forEach((kind) => {
      if (typeof rawEditors[kind] !== "boolean") {
        throw new Error("VSParallel returned incomplete editor visibility preferences.");
      }
      editors[kind] = rawEditors[kind];
    });
    if (typeof raw.usageLimitPercentage !== "boolean") {
      throw new Error("VSParallel returned an invalid usage visibility preference.");
    }
    return {
      schemaVersion: SCHEMA_VERSION,
      editors,
      usageLimitPercentage: raw.usageLimitPercentage,
    };
  }

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
    geminiUsage: requiredElement<HTMLElement>("#geminiUsage"),
    geminiUsageValue: requiredElement<HTMLElement>("#geminiUsageValue"),
    geminiUsageState: requiredElement<HTMLSpanElement>("#geminiUsageState"),
    geminiUsageMeter: requiredElement<HTMLDivElement>("#geminiUsageMeter"),
    geminiUsageDetail: requiredElement<HTMLSpanElement>("#geminiUsageDetail"),
    antigravityUsage: requiredElement<HTMLElement>("#antigravityUsage"),
    antigravityUsageValue: requiredElement<HTMLElement>("#antigravityUsageValue"),
    antigravityUsageState: requiredElement<HTMLSpanElement>("#antigravityUsageState"),
    antigravityUsageMeter: requiredElement<HTMLDivElement>("#antigravityUsageMeter"),
    antigravityUsageDetail: requiredElement<HTMLSpanElement>("#antigravityUsageDetail"),
    zedUsage: requiredElement<HTMLElement>("#zedUsage"),
    zedUsageValue: requiredElement<HTMLElement>("#zedUsageValue"),
    zedUsageState: requiredElement<HTMLSpanElement>("#zedUsageState"),
    zedUsageMeter: requiredElement<HTMLDivElement>("#zedUsageMeter"),
    zedUsageDetail: requiredElement<HTMLSpanElement>("#zedUsageDetail"),
    cursorUsage: requiredElement<HTMLElement>("#cursorUsage"),
    cursorUsageValue: requiredElement<HTMLElement>("#cursorUsageValue"),
    cursorUsageState: requiredElement<HTMLSpanElement>("#cursorUsageState"),
    cursorUsageMeter: requiredElement<HTMLDivElement>("#cursorUsageMeter"),
    cursorUsageDetail: requiredElement<HTMLSpanElement>("#cursorUsageDetail"),
    workspaceList: requiredElement<HTMLDivElement>("#workspaceList"),
    errorBanner: requiredElement<HTMLDivElement>("#errorBanner"),
    errorText: requiredElement<HTMLSpanElement>("#errorText"),
    updateBanner: requiredElement<HTMLElement>("#updateBanner"),
    updateVersion: requiredElement<HTMLSpanElement>("#updateVersion"),
    updateStatus: requiredElement<HTMLSpanElement>("#updateStatus"),
    updateProgress: requiredElement<HTMLProgressElement>("#updateProgress"),
    updateNowButton: requiredElement<HTMLButtonElement>("#updateNowButton"),
    updateLaterButton: requiredElement<HTMLButtonElement>("#updateLaterButton"),
    emptyState: requiredElement<HTMLDivElement>("#emptyState"),
    emptyStateTitle: requiredElement<HTMLHeadingElement>("#emptyStateTitle"),
    emptyStateDescription: requiredElement<HTMLParagraphElement>("#emptyStateDescription"),
    emptySetupButton: requiredElement<HTMLButtonElement>("#emptySetupButton"),
    emptyRefreshButton: requiredElement<HTMLButtonElement>("#emptyRefreshButton"),
    launchOverlay: requiredElement<HTMLDivElement>("#launchOverlay"),
    launchStatus: requiredElement<HTMLSpanElement>("#launchStatus"),
    settingsButton: requiredElement<HTMLButtonElement>("#settingsButton"),
    settingsDialog: requiredElement<HTMLDialogElement>("#settingsDialog"),
    settingsCloseButton: requiredElement<HTMLButtonElement>("#settingsCloseButton"),
    checkForUpdatesButton: requiredElement<HTMLButtonElement>("#checkForUpdatesButton"),
    updateCheckStatus: requiredElement<HTMLParagraphElement>("#updateCheckStatus"),
    diagnosticsSummary: requiredElement<HTMLButtonElement>("#diagnosticsSummary"),
    diagnosticsSummaryDetail: requiredElement<HTMLSpanElement>("#diagnosticsSummaryDetail"),
    diagnosticsList: requiredElement<HTMLDListElement>("#diagnosticsList"),
    diagnosticsStatus: requiredElement<HTMLParagraphElement>("#diagnosticsStatus"),
    diagnosticsRefreshButton: requiredElement<HTMLButtonElement>("#diagnosticsRefreshButton"),
    setupAllButton: requiredElement<HTMLButtonElement>("#setupAllButton"),
    uninstallAllButton: requiredElement<HTMLButtonElement>("#uninstallAllButton"),
    integrationList: requiredElement<HTMLDivElement>("#integrationList"),
    integrationMessage: requiredElement<HTMLParagraphElement>("#integrationMessage"),
    companionCard: requiredElement<HTMLElement>("#companionCard"),
    companionStatus: requiredElement<HTMLSpanElement>("#companionStatus"),
    companionDetail: requiredElement<HTMLParagraphElement>("#companionDetail"),
    companionHelpStatus: requiredElement<HTMLSpanElement>("#companionHelpStatus"),
    companionMeta: requiredElement<HTMLParagraphElement>("#companionMeta"),
    companionInstallButton: requiredElement<HTMLButtonElement>("#companionInstallButton"),
    companionUninstallButton: requiredElement<HTMLButtonElement>("#companionUninstallButton"),
    cursorCompanionCard: requiredElement<HTMLElement>("#cursorCompanionCard"),
    cursorCompanionStatus: requiredElement<HTMLSpanElement>("#cursorCompanionStatus"),
    cursorCompanionDetail: requiredElement<HTMLParagraphElement>("#cursorCompanionDetail"),
    cursorCompanionHelpStatus: requiredElement<HTMLSpanElement>(
      "#cursorCompanionHelpStatus",
    ),
    cursorCompanionMeta: requiredElement<HTMLParagraphElement>("#cursorCompanionMeta"),
    cursorCompanionInstallButton: requiredElement<HTMLButtonElement>(
      "#cursorCompanionInstallButton",
    ),
    cursorCompanionUninstallButton: requiredElement<HTMLButtonElement>(
      "#cursorCompanionUninstallButton",
    ),
    cursorAgentsBridgeCard: requiredElement<HTMLElement>("#cursorAgentsBridgeCard"),
    cursorAgentsBridgeStatus: requiredElement<HTMLSpanElement>("#cursorAgentsBridgeStatus"),
    cursorAgentsBridgeDetail: requiredElement<HTMLParagraphElement>("#cursorAgentsBridgeDetail"),
    cursorAgentsBridgeHelpStatus: requiredElement<HTMLSpanElement>(
      "#cursorAgentsBridgeHelpStatus",
    ),
    cursorAgentsBridgeMessage: requiredElement<HTMLParagraphElement>(
      "#cursorAgentsBridgeMessage",
    ),
    cursorAgentsMonitoringEnabled: requiredElement<HTMLInputElement>(
      "#cursorAgentsMonitoringEnabled",
    ),
    experimentalIntegrations: requiredElement<HTMLDivElement>("#experimentalIntegrations"),
    antigravityIdeCard: requiredElement<HTMLElement>("#antigravityIdeCard"),
    antigravityIdeStatus: requiredElement<HTMLSpanElement>("#antigravityIdeStatus"),
    antigravityIdeDetail: requiredElement<HTMLParagraphElement>("#antigravityIdeDetail"),
    antigravityIdeHelpStatus: requiredElement<HTMLSpanElement>(
      "#antigravityIdeHelpStatus",
    ),
    antigravityIdeMeta: requiredElement<HTMLParagraphElement>("#antigravityIdeMeta"),
    antigravityIdeInstallButton: requiredElement<HTMLButtonElement>(
      "#antigravityIdeInstallButton",
    ),
    antigravityIdeUninstallButton: requiredElement<HTMLButtonElement>(
      "#antigravityIdeUninstallButton",
    ),
    geminiCard: requiredElement<HTMLElement>("#geminiCard"),
    geminiStatus: requiredElement<HTMLSpanElement>("#geminiStatus"),
    geminiDetail: requiredElement<HTMLParagraphElement>("#geminiDetail"),
    geminiUsageHelpStatus: requiredElement<HTMLSpanElement>("#geminiUsageHelpStatus"),
    geminiMeta: requiredElement<HTMLParagraphElement>("#geminiMeta"),
    geminiInstallButton: requiredElement<HTMLButtonElement>("#geminiInstallButton"),
    geminiUninstallButton: requiredElement<HTMLButtonElement>("#geminiUninstallButton"),
    codexCard: requiredElement<HTMLElement>("#codexCard"),
    codexStatus: requiredElement<HTMLSpanElement>("#codexStatus"),
    codexDetail: requiredElement<HTMLParagraphElement>("#codexDetail"),
    codexUsageHelpStatus: requiredElement<HTMLSpanElement>("#codexUsageHelpStatus"),
    codexMeta: requiredElement<HTMLParagraphElement>("#codexMeta"),
    codexInstallButton: requiredElement<HTMLButtonElement>("#codexInstallButton"),
    codexUninstallButton: requiredElement<HTMLButtonElement>("#codexUninstallButton"),
    codexTrustGuidance: requiredElement<HTMLDivElement>("#codexTrustGuidance"),
    claudeCard: requiredElement<HTMLElement>("#claudeCard"),
    claudeStatus: requiredElement<HTMLSpanElement>("#claudeStatus"),
    claudeDetail: requiredElement<HTMLParagraphElement>("#claudeDetail"),
    claudeUsageHelpStatus: requiredElement<HTMLSpanElement>("#claudeUsageHelpStatus"),
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
    editorVisibilityInputs: Array.from(
      document.querySelectorAll<HTMLInputElement>("[data-editor-visibility]"),
    ),
    usageVisibilityInput: requiredElement<HTMLInputElement>("#usageVisibilityInput"),
    visibilityStatus: requiredElement<HTMLParagraphElement>("#visibilityStatus"),
  };

  const initialThemePreference: ThemePreference = isThemePreference(
    document.documentElement.dataset.themePreference,
  )
    ? document.documentElement.dataset.themePreference
    : "system";
  const initialVisibilityPreferences = loadVisibilityPreferences();

  const state: AppState = {
    refreshPending: false,
    usagePending: false,
    usageRefreshPromise: null,
    usageRefreshGeneration: 0,
    diagnosticsPending: false,
    diagnosticsLoaded: false,
    diagnosticsUnavailable: false,
    diagnosticWarningCount: 0,
    diagnosticWarnings: [],
    setupRefreshPending: false,
    setupRefreshPromise: null,
    integrationPending: false,
    integrationLoaded: false,
    integrationStatus: null,
    integrationAction: null,
    cursorAgentsBridgePending: false,
    cursorAgentsBridgeStatus: null,
    pendingUninstall: null,
    openingInstanceId: null,
    lastGoodSnapshot: null,
    lastUsage: null,
    lastUsageAttemptAtMs: null,
    windowChrome: null,
    windowChromeRequestId: 0,
    windowChromeRefreshTimer: null,
    themePreference: initialThemePreference,
    editorVisibility: initialVisibilityPreferences.editors,
    usageVisible: initialVisibilityPreferences.usage,
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
  const HELP_POPOVER_GAP_PX = 7;
  const HELP_POPOVER_GUTTER_PX = 10;

  interface ActiveHelpPopover {
    trigger: HTMLButtonElement;
    content: HTMLElement;
    pinned: boolean;
  }

  let activeHelpPopover: ActiveHelpPopover | null = null;

  function helpPopoverIsOpen(content: HTMLElement): boolean {
    if (typeof content.showPopover === "function") {
      return content.matches(":popover-open");
    }
    return content.dataset.fallbackOpen === "true";
  }

  function positionHelpPopover(trigger: HTMLButtonElement, content: HTMLElement): void {
    if (!helpPopoverIsOpen(content)) {
      return;
    }

    const triggerRect = trigger.getBoundingClientRect();
    const contentRect = content.getBoundingClientRect();
    const maximumLeft = Math.max(
      HELP_POPOVER_GUTTER_PX,
      window.innerWidth - contentRect.width - HELP_POPOVER_GUTTER_PX,
    );
    const centeredLeft = triggerRect.left + (triggerRect.width - contentRect.width) / 2;
    const left = Math.min(
      Math.max(centeredLeft, HELP_POPOVER_GUTTER_PX),
      maximumLeft,
    );
    const preferredTop = triggerRect.top - contentRect.height - HELP_POPOVER_GAP_PX;
    const belowTop = triggerRect.bottom + HELP_POPOVER_GAP_PX;
    const maximumTop = Math.max(
      HELP_POPOVER_GUTTER_PX,
      window.innerHeight - contentRect.height - HELP_POPOVER_GUTTER_PX,
    );
    const top = Math.min(
      Math.max(
        preferredTop >= HELP_POPOVER_GUTTER_PX ? preferredTop : belowTop,
        HELP_POPOVER_GUTTER_PX,
      ),
      maximumTop,
    );
    content.style.left = `${Math.round(left)}px`;
    content.style.top = `${Math.round(top)}px`;
  }

  function closeActiveHelpPopover(): boolean {
    const active = activeHelpPopover;
    if (!active) {
      return false;
    }
    activeHelpPopover = null;
    if (typeof active.content.hidePopover === "function") {
      if (active.content.matches(":popover-open")) {
        active.content.hidePopover();
      }
    } else {
      delete active.content.dataset.fallbackOpen;
    }
    return true;
  }

  function openHelpPopover(
    trigger: HTMLButtonElement,
    content: HTMLElement,
    pinned: boolean,
  ): void {
    if (activeHelpPopover?.content !== content) {
      closeActiveHelpPopover();
    }
    activeHelpPopover = { trigger, content, pinned };
    if (typeof content.showPopover === "function") {
      if (!content.matches(":popover-open")) {
        content.showPopover();
      }
    } else {
      content.dataset.fallbackOpen = "true";
    }
    positionHelpPopover(trigger, content);
  }

  function initializeHelpPopovers(): void {
    document.querySelectorAll<HTMLElement>(".help-popover").forEach((wrapper) => {
      const trigger = wrapper.querySelector<HTMLButtonElement>(".help-popover__trigger");
      const describedId = trigger?.getAttribute("aria-describedby") || "";
      const content = describedId
        ? document.getElementById(describedId)
        : null;
      if (!trigger || !content || !content.classList.contains("help-popover__content")) {
        return;
      }

      wrapper.addEventListener("pointerenter", () => {
        openHelpPopover(trigger, content, false);
      });
      wrapper.addEventListener("pointerleave", () => {
        if (
          activeHelpPopover?.trigger === trigger
          && !activeHelpPopover.pinned
          && document.activeElement !== trigger
        ) {
          closeActiveHelpPopover();
        }
      });
      trigger.addEventListener("focus", () => {
        openHelpPopover(trigger, content, false);
      });
      trigger.addEventListener("blur", () => {
        window.setTimeout(() => {
          if (activeHelpPopover?.trigger === trigger && !activeHelpPopover.pinned) {
            closeActiveHelpPopover();
          }
        }, 0);
      });
      trigger.addEventListener("click", (event) => {
        event.preventDefault();
        if (activeHelpPopover?.trigger === trigger && activeHelpPopover.pinned) {
          closeActiveHelpPopover();
        } else {
          openHelpPopover(trigger, content, true);
        }
      });
    });

    document.addEventListener("pointerdown", (event) => {
      if (
        activeHelpPopover
        && event.target instanceof Node
        && !activeHelpPopover.trigger.contains(event.target)
      ) {
        closeActiveHelpPopover();
      }
    }, true);
    elements.settingsDialog.addEventListener("scroll", () => {
      if (activeHelpPopover) {
        positionHelpPopover(activeHelpPopover.trigger, activeHelpPopover.content);
      }
    }, true);
  }

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
      "manual_action_required",
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
      optional: !["companion", "cursorCompanion", "antigravityIde"].includes(kind),
      token,
      visualState,
      installed,
      actionLabel,
      label: asString(raw.label, defaultLabel),
      detail: asString(
        raw.detail,
        kind === "companion"
          ? "VS Code companion status details are unavailable."
          : kind === "cursorCompanion"
            ? "Cursor companion status details are unavailable."
            : kind === "antigravityIde"
              ? "Antigravity IDE companion status details are unavailable."
              : kind === "cursor"
                ? "Cursor hooks-only status details are unavailable."
                : kind === "antigravity"
                  ? "Antigravity activity hook status details are unavailable."
                  : kind === "gemini"
                    ? "Gemini usage hook status details are unavailable."
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
      cursorCompanion: normalizeIntegrationComponent(raw.cursorCompanion, "cursorCompanion"),
      antigravityIde: normalizeIntegrationComponent(raw.antigravityIde, "antigravityIde"),
      cursor: normalizeIntegrationComponent(raw.cursor, "cursor"),
      antigravity: normalizeIntegrationComponent(raw.antigravity, "antigravity"),
      gemini: normalizeIntegrationComponent(raw.gemini, "gemini"),
      codex: normalizeIntegrationComponent(raw.codex, "codex"),
      claude: normalizeIntegrationComponent(raw.claude, "claude"),
      requiresRestart: raw.requiresRestart === true,
    };
  }

  function normalizeCursorAgentsBridgeStatus(rawValue: unknown): CursorAgentsBridgeStatus {
    const raw = parseBridgeValue(rawValue);
    if (!isObject(raw)) {
      throw new Error("VSParallel returned invalid Cursor agent-thread status.");
    }
    if (raw.schemaVersion !== SCHEMA_VERSION) {
      throw new Error("VSParallel returned an unsupported Cursor agent-thread status version.");
    }

    const availabilityToken = normalizeStateToken(raw.availability);
    const availability: CursorAgentsBridgeAvailability = [
      "disabled",
      "connected",
      "waiting",
      "unsupported",
      "error",
    ].includes(availabilityToken)
      ? availabilityToken as CursorAgentsBridgeAvailability
      : "error";

    return {
      schemaVersion: SCHEMA_VERSION,
      enabled: raw.enabled === true,
      availability,
      connected: availability === "connected" && raw.connected === true,
      instanceCount: asNonNegativeInteger(raw.instanceCount),
      threadCount: asNonNegativeInteger(raw.threadCount),
      lastCheckedAtMs: asTimestamp(raw.lastCheckedAtMs),
      errorCode: asString(raw.errorCode),
      detail: asString(raw.detail),
    };
  }

  function describeActivityState(token: string): Pick<ActivityView, "kind" | "label"> {
    if (token === "activity_detected") {
      return {
        kind: "activity",
        label: "Activity detected",
      };
    }

    if (token === "turn_finished") {
      return {
        kind: "finished",
        label: "Turn finished",
      };
    }

    if (token === "recent_activity") {
      return {
        kind: "recent",
        label: "Recent agent activity",
      };
    }

    if (["failed_or_interrupted", "failed/interrupted", "failed", "interrupted"].includes(token)) {
      return {
        kind: "failure",
        label: "Failed/interrupted",
      };
    }

    return {
      kind: "unknown",
      label: "Unknown",
    };
  }

  function normalizeAntigravityModelKind(value: unknown): AntigravityModelKind | null {
    const token = normalizeStateToken(value);
    switch (token) {
      case "automatic":
      case "gemini":
      case "gemini_3_6_flash_medium":
      case "gemini_3_6_flash_high":
      case "gemini_3_5_flash":
      case "gemini_3_1_pro_high":
      case "gemini_3_1_pro_low":
      case "gemini_3_flash":
      case "claude":
      case "claude_sonnet_4_6_thinking":
      case "claude_opus_4_6_thinking":
      case "gpt_oss":
      case "gpt_oss_120b":
      case "gpt_oss_120b_medium":
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
      case "gemini_3_6_flash_high":
        return "Gemini 3.6 Flash (High)";
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
        return "GPT-OSS 120B";
      case "gpt_oss_120b_medium":
        return "GPT-OSS 120B (Medium)";
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

  function antigravityModelFamilyLabel(kind: AntigravityModelKind | null): string {
    switch (kind) {
      case "automatic":
        return "Auto";
      case "gemini":
      case "gemini_3_6_flash_medium":
      case "gemini_3_6_flash_high":
      case "gemini_3_5_flash":
      case "gemini_3_1_pro_high":
      case "gemini_3_1_pro_low":
      case "gemini_3_flash":
        return "Gemini";
      case "claude":
      case "claude_sonnet_4_6_thinking":
      case "claude_opus_4_6_thinking":
        return "Claude";
      case "gpt_oss":
      case "gpt_oss_120b":
      case "gpt_oss_120b_medium":
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
      modelName: asString(raw.modelName),
      agentKind: asString(raw.agentKind),
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
    const editor: Workspace["editor"] = editorToken === "cursor"
      ? "cursor"
      : editorToken === "antigravity_ide"
        ? "antigravity_ide"
        : editorToken === "antigravity_2"
          ? "antigravity_2"
          : editorToken === "zed"
            ? "zed"
          : "vscode";
    const surfaceToken = normalizeStateToken(raw.surface);
    const surface: WorkspaceSurface = surfaceToken === "hook_only"
      ? "hook_only"
      : surfaceToken === "cursor_agent_thread"
        ? "cursor_agent_thread"
        : "editor_workspace";
    const defaultEditorName = surface === "cursor_agent_thread"
      ? "Cursor agent thread (experimental)"
      : editor === "cursor"
      ? "Cursor"
      : editor === "antigravity_ide"
        ? "Antigravity IDE"
        : editor === "antigravity_2"
          ? "Antigravity 2.0"
          : editor === "zed"
            ? "Zed"
          : "VS Code";

    return {
      instanceId,
      editor,
      editorName: asString(raw.editorName, defaultEditorName),
      surface,
      name,
      path,
      openable: surface !== "cursor_agent_thread"
        && raw.openable === true
        && Boolean(instanceId),
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
      cursor: isObject(raw.cursor)
        ? normalizeActivityView(raw.cursor)
        : null,
      zed: isObject(raw.zed)
        ? normalizeActivityView(raw.zed)
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

  function normalizeUsageMetricKind(
    value: unknown,
    remainingPercent: number | null,
    tokenCount: number | null,
    hasWindows: boolean,
  ): UsageMetricKind {
    const token = normalizeStateToken(value);
    if (["quota", "context", "tokens", "none"].includes(token)) {
      return token as UsageMetricKind;
    }
    if (value !== undefined && value !== null && asString(value)) {
      return "none";
    }
    if (tokenCount !== null) {
      return "tokens";
    }
    if (remainingPercent !== null || hasWindows) {
      // Snapshots from versions before metricKind represented Codex and Claude quotas.
      return "quota";
    }
    return "none";
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
    const suppliedTokenCount = asFiniteNumber(raw.tokenCount);
    const tokenCount = suppliedTokenCount !== null
        && Number.isSafeInteger(suppliedTokenCount)
        && suppliedTokenCount >= 0
      ? suppliedTokenCount
      : null;
    const metricKind = normalizeUsageMetricKind(
      raw.metricKind,
      remainingPercent,
      tokenCount,
      windows.length > 0,
    );
    const rawState = normalizeStateToken(raw.state);
    const metricAvailable = metricKind === "tokens"
      ? tokenCount !== null
      : ["quota", "context"].includes(metricKind) && remainingPercent !== null;
    const available = metricAvailable && ["available", "stale"].includes(rawState);
    const metricLabel = asString(
      raw.metricLabel,
      asString(
        raw.summaryWindowLabel ?? raw.windowLabel,
        limitingWindow?.label || "",
      ),
    );

    return {
      providerName,
      state: available ? (rawState === "stale" ? "stale" : "available") : "unavailable",
      metricKind,
      remainingPercent: available && metricKind !== "tokens" ? remainingPercent : null,
      tokenCount: available && metricKind === "tokens" ? tokenCount : null,
      metricLabel,
      windowLabel: asString(
        raw.summaryWindowLabel ?? raw.windowLabel,
        limitingWindow?.label || metricLabel || "Usage limit",
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
      gemini: normalizeUsageProvider(raw.gemini, "Gemini"),
      antigravity: normalizeUsageProvider(raw.antigravity, "Antigravity"),
      zed: normalizeUsageProvider(raw.zed, "Zed Agent"),
      cursor: normalizeUsageProvider(raw.cursor, "Cursor"),
    };
  }

  function usageProviderHasMetric(provider: UsageProvider): boolean {
    return provider.metricKind === "tokens"
      ? provider.tokenCount !== null
      : ["quota", "context"].includes(provider.metricKind)
        && provider.remainingPercent !== null;
  }

  function usageProviderWithFallback(
    current: UsageProvider,
    previous: UsageProvider | null,
    nowMs: number,
  ): UsageProvider {
    const normalizedDetail = current.detail.toLowerCase();
    const captureNeedsSetup = [
      "capture is disabled",
      "disable hooks",
      "not installed",
      "repair",
      "conflict",
    ].some((marker) => normalizedDetail.includes(marker));
    if (
      captureNeedsSetup
      || !previous
      || usageProviderHasMetric(current)
      || !usageProviderHasMetric(previous)
    ) {
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
      remainingPercent: previous.metricKind === "tokens"
        ? null
        : limitingWindow?.remainingPercent ?? previous.remainingPercent,
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
      gemini: usageProviderWithFallback(current.gemini, previous.gemini, nowMs),
      antigravity: usageProviderWithFallback(
        current.antigravity,
        previous.antigravity,
        nowMs,
      ),
      zed: usageProviderWithFallback(current.zed, previous.zed, nowMs),
      cursor: usageProviderWithFallback(current.cursor, previous.cursor, nowMs),
    };
  }

  function unavailableUsageSnapshot(detail: string): UsageSnapshot {
    const unavailable = normalizeUsageProvider({ detail }, "Provider");
    return {
      schemaVersion: SCHEMA_VERSION,
      generatedAtMs: Date.now(),
      codex: { ...unavailable, providerName: "Codex" },
      claude: { ...unavailable, providerName: "Claude" },
      gemini: { ...unavailable, providerName: "Gemini" },
      antigravity: { ...unavailable, providerName: "Antigravity" },
      zed: { ...unavailable, providerName: "Zed Agent" },
      cursor: { ...unavailable, providerName: "Cursor" },
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
      activity: 5,
      failure: 4,
      finished: 3,
      recent: 2,
      unknown: 1,
    };
    return [workspace.codex, workspace.claude, workspace.antigravity, workspace.cursor, workspace.zed]
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
    lifecycleSource = "",
    providerNameDetail = "",
    providerNameDetailTitle = providerNameDetail,
  ): HTMLDivElement {
    const provider = createElement("div", "provider-state");
    provider.dataset.state = activity.kind;

    const name = createElement("span", "provider-name", providerName);
    if (providerNameDetail) {
      const detail = createElement(
        "span",
        "provider-name-detail",
        `(${providerNameDetail})`,
      );
      detail.title = providerNameDetailTitle;
      name.append(detail);
    }
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
      changedAt.title = `Activity timestamp: ${formatAbsoluteTime(activity.changedAtMs)}`;
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
    } else if (lifecycleSource) {
      const source = createElement("span", "provider-extension", lifecycleSource);
      source.dataset.state = "present";
      source.title = lifecycleSource === "Cursor desktop bridge"
        ? "Live thread state reported by Cursor's experimental read-only Desktop Bridge and correlated with bounded Cursor hook metadata."
        : lifecycleSource === "Cursor hooks"
          ? "Lifecycle activity reported by Cursor's documented agent hooks."
          : lifecycleSource === "Zed local metadata"
            ? "Coarse persisted Zed Agent turn boundaries and model information reported by Zed's read-only local metadata; this can lag live generation."
          : lifecycleSource === "Workspace-matched lifecycle records"
            ? "Lifecycle state from local provider records matched to this workspace path when observed."
          : "Lifecycle activity reported by Antigravity's built-in model hook.";
      body.append(source);
      presenceLabel = lifecycleSource;
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
    row.dataset.surface = workspace.surface;

    const primary = createElement("div", "workspace-primary");
    const application = createElement(
      "span",
      "workspace-application",
      workspace.editorName,
    );
    application.dataset.editor = workspace.editor;
    if (workspace.surface === "cursor_agent_thread") {
      application.title = "Correlated through Cursor's experimental local Desktop Bridge.";
    }
    const titleLine = createElement("div", "workspace-title-line");
    const name = createElement("h4", "workspace-name", workspace.name);
    name.title = workspace.name;
    if (workspace.focused) {
      const focused = createElement("span", "workspace-focus");
      focused.setAttribute("aria-hidden", "true");
      titleLine.append(focused);
    }
    titleLine.append(name);

    const path = createElement("span", "workspace-path", formatShortPath(workspace.path));
    if (workspace.path) {
      path.title = workspace.path;
    }
    const metaLine = createElement("div", "workspace-meta");
    metaLine.append(path);
    primary.append(application, titleLine, metaLine);
    row.append(primary);

    const aggregate = aggregateActivity(workspace);
    const compactStatus = createElement(
      "span",
      "workspace-compact-status",
      aggregate.label,
    );
    compactStatus.dataset.state = aggregate.kind;
    row.append(compactStatus);

    const providers = createElement("div", "activity-providers");
    providers.setAttribute(
      "aria-label",
      workspace.editor === "zed"
        ? "Agent lifecycle and local metadata"
        : "Agent lifecycle and IDE extension status",
    );
    if (workspace.antigravity) {
      const modelLabel = antigravityModelLabel(workspace.antigravity.modelKind);
      const modelFamily = antigravityModelFamilyLabel(workspace.antigravity.modelKind);
      providers.append(
        createProviderState(
          "Antigravity",
          workspace.antigravity,
          modelLabel
            ? `Antigravity (${modelLabel}), latest model reported by Antigravity`
            : "Antigravity",
          workspace.editorName,
          false,
          false,
          "Antigravity built-in model",
          modelFamily,
          modelLabel,
        ),
      );
    }
    if (workspace.cursor) {
      const cursorDetails = [workspace.cursor.agentKind, workspace.cursor.modelName]
        .filter(Boolean)
        .join(" · ");
      const cursorSource = workspace.surface === "cursor_agent_thread"
        ? "Cursor desktop bridge"
        : "Cursor hooks";
      providers.append(
        createProviderState(
          "Cursor Agent",
          workspace.cursor,
          cursorDetails
            ? `Cursor Agent (${cursorDetails}), latest agent and model reported by Cursor`
            : "Cursor Agent",
          workspace.editorName,
          false,
          false,
          cursorSource,
          cursorDetails,
          cursorDetails ? `Latest Cursor agent and model: ${cursorDetails}` : "",
        ),
      );
    }
    if (workspace.zed) {
      const zedAgentKind = workspace.zed.agentKind === "Agent panel"
        ? ""
        : workspace.zed.agentKind;
      const zedDetails = [zedAgentKind, workspace.zed.modelName]
        .filter(Boolean)
        .join(" · ");
      providers.append(
        createProviderState(
          "Zed Agent",
          workspace.zed,
          zedDetails
            ? `Zed Agent (${zedDetails}), latest model or external agent metadata reported by Zed`
            : "Zed Agent",
          workspace.editorName,
          false,
          false,
          "Zed local metadata",
          zedDetails,
          zedDetails ? `Latest Zed model or external agent: ${zedDetails}` : "",
        ),
      );
    }
    if (
      workspace.editor !== "antigravity_2"
      && workspace.surface !== "cursor_agent_thread"
    ) {
      const nativeReadOnlyEditor = workspace.editor === "zed";
      providers.append(
        createProviderState(
          "Codex",
          workspace.codex,
          "Codex",
          workspace.editorName,
          workspace.remoteWindow,
          !nativeReadOnlyEditor,
          nativeReadOnlyEditor ? "Workspace-matched lifecycle records" : "",
        ),
        createProviderState(
          "Claude",
          workspace.claude,
          "Claude Code",
          workspace.editorName,
          workspace.remoteWindow,
          !nativeReadOnlyEditor,
          nativeReadOnlyEditor ? "Workspace-matched lifecycle records" : "",
        ),
      );
    }
    row.append(providers);

    const openButton = createElement("button", "open-button");
    const actionLabel = workspace.active ? "Switch to" : "Open";
    const focusContext = workspace.focused ? ", currently focused" : "";
    const accessibleActionLabel = openable
      ? `${actionLabel} ${workspace.name} in ${workspace.editorName}${focusContext}`
      : `${workspace.name} in ${workspace.editorName}${focusContext}, cannot currently be opened`;
    openButton.type = "button";
    openButton.dataset.instanceId = workspace.instanceId;
    openButton.disabled = !openable;
    openButton.setAttribute("aria-label", accessibleActionLabel);
    openButton.setAttribute("aria-busy", String(opening));
    if (!workspace.openable) {
      openButton.title = workspace.surface === "cursor_agent_thread"
        ? "Cursor's experimental desktop bridge reports agent-thread status but does not provide a safe window activation target"
        : workspace.editor === "zed"
          ? "This saved Zed workspace has no safely reconstructed local target, or belongs to a non-Stable release channel"
        : workspace.recentlyActive
        ? `${workspace.editorName} hook activity does not identify a live window or exact open target`
        : "This workspace cannot currently be opened";
    } else {
      openButton.title = `${actionLabel} ${workspace.name} in ${workspace.editorName}`;
    }
    openButton.addEventListener("click", () => openWorkspace(workspace));
    row.append(openButton);

    return row;
  }

  function groupWorkspaces(workspaces: Workspace[]): WorkspaceGroup[] {
    const open = workspaces.filter((workspace) => workspace.active);
    const recent = workspaces.filter((workspace) => !workspace.active);
    const groups: WorkspaceGroup[] = [
      { kind: "open", label: "Open", workspaces: open },
      { kind: "recent", label: "Recent", workspaces: recent },
    ];
    return groups.filter((group) => group.workspaces.length > 0);
  }

  function createWorkspaceGroup(group: WorkspaceGroup): HTMLElement {
    const section = createElement("section", "workspace-group");
    const heading = createElement("h3", "workspace-group__heading", group.label);
    const headingId = `workspaceGroup-${group.kind}`;
    const cards = createElement("ul", "workspace-group__cards");

    section.dataset.state = group.kind;
    section.setAttribute("aria-labelledby", headingId);
    heading.id = headingId;
    cards.setAttribute("aria-label", `${group.label} workspaces`);
    group.workspaces.forEach((workspace) => {
      cards.append(createWorkspaceRow(workspace));
    });
    section.append(heading, cards);
    return section;
  }

  function workspaceVisibilityKind(workspace: Workspace): EditorVisibilityKind {
    if (workspace.editor === "antigravity_2" || workspace.editor === "antigravity_ide") {
      return "antigravity";
    }
    return workspace.editor;
  }

  function visibleWorkspaces(workspaces: readonly Workspace[]): Workspace[] {
    return workspaces.filter(
      (workspace) => state.editorVisibility[workspaceVisibilityKind(workspace)],
    );
  }

  function renderSnapshot(snapshot: Snapshot): void {
    const focusedOpenButton = document.activeElement
      ?.closest<HTMLButtonElement>(".open-button") ?? null;
    const focusedInstanceId = focusedOpenButton
      && elements.workspaceList.contains(focusedOpenButton)
      ? focusedOpenButton.dataset.instanceId
      : "";
    const displayedWorkspaces = visibleWorkspaces(snapshot.workspaces);
    const fragment = document.createDocumentFragment();
    groupWorkspaces(displayedWorkspaces).forEach((group) => {
      fragment.append(createWorkspaceGroup(group));
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
    const workspaceCountLabel = `${displayedWorkspaces.length} ${displayedWorkspaces.length === 1 ? "workspace" : "workspaces"}`;
    elements.workspaceCount.textContent = workspaceCountLabel;
    elements.workspaceCount.setAttribute(
      "aria-label",
      workspaceCountLabel,
    );
    elements.emptyState.hidden = displayedWorkspaces.length !== 0;
    const visibilityFilterActive = EDITOR_VISIBILITY_KINDS.some(
      (kind) => !state.editorVisibility[kind],
    );
    elements.emptyStateTitle.textContent = visibilityFilterActive
      ? "No visible workspaces"
      : "No workspaces detected";
    elements.emptyStateDescription.textContent = visibilityFilterActive
      ? "No workspaces from enabled editors are visible. Re-enable an editor under Settings › Visibility, or refresh after opening a workspace in an enabled editor."
      : "Open a folder in VS Code, Cursor, or Antigravity IDE with its VSParallel integration enabled, or open one in Zed for automatic read-only monitoring. Start an agent turn to see activity and model information when available.";
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

  function usageElements(kind: UsageKind): UsageElements {
    const targets: Record<UsageKind, UsageElements> = {
      codex: {
        card: elements.codexUsage,
        value: elements.codexUsageValue,
        stateLabel: elements.codexUsageState,
        meter: elements.codexUsageMeter,
        detail: elements.codexUsageDetail,
      },
      claude: {
        card: elements.claudeUsage,
        value: elements.claudeUsageValue,
        stateLabel: elements.claudeUsageState,
        meter: elements.claudeUsageMeter,
        detail: elements.claudeUsageDetail,
      },
      gemini: {
        card: elements.geminiUsage,
        value: elements.geminiUsageValue,
        stateLabel: elements.geminiUsageState,
        meter: elements.geminiUsageMeter,
        detail: elements.geminiUsageDetail,
      },
      antigravity: {
        card: elements.antigravityUsage,
        value: elements.antigravityUsageValue,
        stateLabel: elements.antigravityUsageState,
        meter: elements.antigravityUsageMeter,
        detail: elements.antigravityUsageDetail,
      },
      zed: {
        card: elements.zedUsage,
        value: elements.zedUsageValue,
        stateLabel: elements.zedUsageState,
        meter: elements.zedUsageMeter,
        detail: elements.zedUsageDetail,
      },
      cursor: {
        card: elements.cursorUsage,
        value: elements.cursorUsageValue,
        stateLabel: elements.cursorUsageState,
        meter: elements.cursorUsageMeter,
        detail: elements.cursorUsageDetail,
      },
    };
    return targets[kind];
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

  function resetUsageMeter(meter: HTMLDivElement, hidden: boolean): void {
    meter.hidden = hidden;
    meter.removeAttribute("role");
    meter.removeAttribute("aria-label");
    meter.removeAttribute("aria-valuemin");
    meter.removeAttribute("aria-valuemax");
    meter.removeAttribute("aria-valuenow");
    meter.removeAttribute("aria-valuetext");
    meter.setAttribute("aria-hidden", "true");
  }

  function formatTokenCount(tokenCount: number): string {
    return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(tokenCount);
  }

  function renderUsageProvider(kind: UsageKind, provider: UsageProvider): string {
    const target = usageElements(kind);
    const remainingPercent = provider.remainingPercent;
    const available = usageProviderHasMetric(provider);
    const stale = available && provider.state === "stale";
    target.card.dataset.state = available ? (stale ? "stale" : "available") : "unavailable";
    target.card.dataset.metricKind = available ? provider.metricKind : "none";
    target.stateLabel.hidden = !stale;

    if (!available) {
      delete target.card.dataset.level;
      target.card.style.setProperty("--usage-remaining", "0");
      target.value.textContent = "—";
      target.detail.textContent = provider.detail || "Usage unavailable";
      resetUsageMeter(target.meter, false);
      target.card.title = provider.detail || `${provider.providerName} usage is unavailable.`;
      return provider.detail
        ? `${provider.providerName} usage unavailable: ${provider.detail}`
        : `${provider.providerName} usage unavailable`;
    }

    if (provider.metricKind === "tokens" && provider.tokenCount !== null) {
      delete target.card.dataset.level;
      target.card.style.setProperty("--usage-remaining", "0");
      const formattedTokens = formatTokenCount(provider.tokenCount);
      const updateLabel = provider.updatedAtMs !== null
          && Number.isFinite(provider.updatedAtMs)
        ? formatRelativeTime(provider.updatedAtMs).toLowerCase()
        : "";
      const detailParts = [provider.metricLabel || "Local token usage", updateLabel];
      if (stale) {
        detailParts.push("last known value");
      }
      target.value.textContent = `${formattedTokens} tokens`;
      target.detail.textContent = detailParts.filter(Boolean).join(" · ");
      resetUsageMeter(target.meter, true);
      target.card.title = [
        `${provider.providerName}: ${formattedTokens} tokens`,
        provider.metricLabel,
        updateLabel,
        stale ? "last known value" : "",
        provider.detail,
      ].filter(Boolean).join(" · ");
      return `${provider.providerName} ${formattedTokens} tokens${stale ? " (last known)" : ""}`;
    }

    if (remainingPercent === null) {
      // usageProviderHasMetric keeps this unreachable, but retain a closed rendering fallback.
      resetUsageMeter(target.meter, false);
      return `${provider.providerName} usage unavailable`;
    }

    const roundedRemaining = Math.round(remainingPercent);
    const resetLabel = formatResetTime(provider.resetsAtMs);
    const updateLabel = provider.updatedAtMs !== null && Number.isFinite(provider.updatedAtMs)
      ? formatRelativeTime(provider.updatedAtMs).toLowerCase()
      : "";
    const isContext = provider.metricKind === "context";
    const detailParts = isContext
      ? [provider.metricLabel || provider.windowLabel, updateLabel]
      : [provider.windowLabel, resetLabel];
    if (stale) {
      detailParts.push("last known value");
    }
    target.card.dataset.level = usageLevel(remainingPercent);
    target.card.style.setProperty("--usage-remaining", remainingPercent.toFixed(2));
    target.value.textContent = isContext
      ? `${roundedRemaining}% context left`
      : `${roundedRemaining}% left`;
    target.detail.textContent = detailParts.filter(Boolean).join(" · ");
    target.meter.hidden = false;
    target.meter.removeAttribute("aria-hidden");
    target.meter.setAttribute("role", "meter");
    target.meter.setAttribute(
      "aria-label",
      `${provider.providerName} ${isContext ? "context" : "quota"} remaining`,
    );
    target.meter.setAttribute("aria-valuemin", "0");
    target.meter.setAttribute("aria-valuemax", "100");
    target.meter.setAttribute("aria-valuenow", remainingPercent.toFixed(1));
    target.meter.setAttribute(
      "aria-valuetext",
      [
        isContext
          ? `${roundedRemaining}% of the latest context window remaining`
          : `${roundedRemaining}% remaining on the ${provider.windowLabel.toLowerCase()}`,
        isContext ? "" : resetLabel,
        stale ? "last known value" : "",
        stale ? provider.detail : "",
      ].filter(Boolean).join("; "),
    );
    const windowDescriptions = provider.windows.map((window) => {
      const remaining = Math.round(window.remainingPercent);
      return `${window.label}: ${remaining}% remaining, ${formatResetTime(window.resetsAtMs)}`;
    });
    target.card.title = [
      isContext
        ? `${provider.providerName}: ${roundedRemaining}% of context remaining`
        : `${provider.providerName}: ${roundedRemaining}% remaining on the ${provider.windowLabel.toLowerCase()}`,
      isContext ? provider.metricLabel : resetLabel,
      ...(isContext ? [] : windowDescriptions),
      updateLabel,
      provider.detail,
    ].filter(Boolean).join(" · ");
    return `${provider.providerName} ${roundedRemaining}% ${isContext ? "context " : ""}remaining${stale ? " (last known)" : ""}`;
  }

  function renderUsageSnapshot(snapshot: UsageSnapshot): void {
    const summaries = [
      renderUsageProvider("codex", snapshot.codex),
      renderUsageProvider("claude", snapshot.claude),
      renderUsageProvider("gemini", snapshot.gemini),
      renderUsageProvider("antigravity", snapshot.antigravity),
      renderUsageProvider("zed", snapshot.zed),
      renderUsageProvider("cursor", snapshot.cursor),
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

  function storeVisibilityPreferences(): void {
    try {
      window.localStorage.setItem(
        VISIBILITY_STORAGE_KEY,
        JSON.stringify({ editors: state.editorVisibility, usage: state.usageVisible }),
      );
    } catch (_error) {
      // The selected visibility still applies for this session when storage is unavailable.
    }
  }

  function applyVisibilityPreferences(persist = true): void {
    elements.editorVisibilityInputs.forEach((input) => {
      const kind = input.dataset.editorVisibility as EditorVisibilityKind | undefined;
      if (kind && EDITOR_VISIBILITY_KINDS.includes(kind)) {
        input.checked = state.editorVisibility[kind];
      }
    });
    elements.usageVisibilityInput.checked = state.usageVisible;
    elements.usageOverview.hidden = !state.usageVisible;
    if (persist) {
      storeVisibilityPreferences();
    }
    if (state.lastGoodSnapshot) {
      renderSnapshot(state.lastGoodSnapshot);
    }
  }

  function commitDisplayPreferences(
    preferences: DisplayPreferencesResponse,
    persist = true,
  ): void {
    state.editorVisibility = { ...preferences.editors };
    state.usageVisible = preferences.usageLimitPercentage;
    applyVisibilityPreferences(persist);
  }

  function setVisibilityStatus(message: string, error = false): void {
    elements.visibilityStatus.textContent = message;
    elements.visibilityStatus.hidden = !message;
    elements.visibilityStatus.classList.toggle("has-error", error);
  }

  async function refreshDisplayPreferences(): Promise<void> {
    try {
      const raw = await invoke("get_display_preferences", {});
      commitDisplayPreferences(normalizeDisplayPreferences(raw));
      setVisibilityStatus("");
    } catch (_error) {
      applyVisibilityPreferences(false);
      setVisibilityStatus("Using visibility saved for this window; app-wide sync is unavailable.", true);
    }
  }

  async function updateEditorVisibility(input: HTMLInputElement): Promise<void> {
    const kind = input.dataset.editorVisibility as EditorVisibilityKind | undefined;
    if (!kind || !EDITOR_VISIBILITY_KINDS.includes(kind)) {
      return;
    }
    state.editorVisibility[kind] = input.checked;
    applyVisibilityPreferences();
    input.disabled = true;
    setVisibilityStatus("Saving visibility…");
    try {
      const raw = await invoke("set_editor_visibility", {
        editor: kind,
        visible: input.checked,
      });
      commitDisplayPreferences(normalizeDisplayPreferences(raw));
      setVisibilityStatus("");
      await refreshSnapshot();
    } catch (_error) {
      setVisibilityStatus("Saved for this window; app-wide sync is unavailable.", true);
    } finally {
      input.disabled = false;
    }
  }

  async function updateUsageVisibility(visible: boolean): Promise<void> {
    state.usageVisible = visible;
    applyVisibilityPreferences();
    elements.usageVisibilityInput.disabled = true;
    setVisibilityStatus("Saving visibility…");
    try {
      const raw = await invoke("set_usage_limit_percentage_visible", { visible });
      commitDisplayPreferences(normalizeDisplayPreferences(raw));
      setVisibilityStatus("");
    } catch (_error) {
      setVisibilityStatus("Saved for this window; app-wide sync is unavailable.", true);
    } finally {
      elements.usageVisibilityInput.disabled = false;
    }
    if (state.usageVisible) {
      void refreshUsage(true);
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

  async function refreshUsage(forceAfterPending = false): Promise<void> {
    if (forceAfterPending) {
      // Invalidate any response that began before an integration mutation.
      state.usageRefreshGeneration += 1;
    }
    if (!state.usageVisible) {
      return;
    }
    if (state.usageRefreshPromise) {
      const pending = state.usageRefreshPromise;
      await pending;
      if (forceAfterPending && state.usageVisible) {
        await refreshUsage();
      }
      return;
    }

    const generation = state.usageRefreshGeneration;
    state.usagePending = true;
    state.lastUsageAttemptAtMs = Date.now();
    updateRefreshControl();
    const operation = (async () => {
      try {
        let current: UsageSnapshot;
        try {
          const raw = await invoke("get_usage", {});
          current = normalizeUsageSnapshot(raw);
        } catch (_error) {
          current = unavailableUsageSnapshot("Could not refresh provider usage.");
        }
        if (generation !== state.usageRefreshGeneration) {
          return;
        }
        const usage = resolveUsageSnapshot(current, state.lastUsage);
        state.lastUsage = usage;
        renderUsageSnapshot(usage);
      } finally {
        state.usagePending = false;
        state.usageRefreshPromise = null;
        updateRefreshControl();
      }
    })();
    state.usageRefreshPromise = operation;
    await operation;
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
        helpDetail: elements.companionHelpStatus,
        meta: elements.companionMeta,
        installButton: elements.companionInstallButton,
        uninstallButton: elements.companionUninstallButton,
      };
    }

    if (kind === "cursorCompanion") {
      return {
        card: elements.cursorCompanionCard,
        status: elements.cursorCompanionStatus,
        detail: elements.cursorCompanionDetail,
        helpDetail: elements.cursorCompanionHelpStatus,
        meta: elements.cursorCompanionMeta,
        installButton: elements.cursorCompanionInstallButton,
        uninstallButton: elements.cursorCompanionUninstallButton,
      };
    }

    if (kind === "antigravityIde") {
      return {
        card: elements.antigravityIdeCard,
        status: elements.antigravityIdeStatus,
        detail: elements.antigravityIdeDetail,
        helpDetail: elements.antigravityIdeHelpStatus,
        meta: elements.antigravityIdeMeta,
        installButton: elements.antigravityIdeInstallButton,
        uninstallButton: elements.antigravityIdeUninstallButton,
      };
    }

    if (kind === "cursor" || kind === "antigravity") {
      throw new Error(`${integrationComponentName(kind)} has no separate visible row.`);
    }

    if (kind === "gemini") {
      return {
        card: elements.geminiCard,
        status: elements.geminiStatus,
        detail: elements.geminiDetail,
        helpDetail: elements.geminiUsageHelpStatus,
        meta: elements.geminiMeta,
        installButton: elements.geminiInstallButton,
        uninstallButton: elements.geminiUninstallButton,
      };
    }

    if (kind === "claude") {
      return {
        card: elements.claudeCard,
        status: elements.claudeStatus,
        detail: elements.claudeDetail,
        helpDetail: elements.claudeUsageHelpStatus,
        meta: elements.claudeMeta,
        installButton: elements.claudeInstallButton,
        uninstallButton: elements.claudeUninstallButton,
      };
    }

    return {
      card: elements.codexCard,
      status: elements.codexStatus,
      detail: elements.codexDetail,
      helpDetail: elements.codexUsageHelpStatus,
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
    componentElements.detail.textContent = integrationPurpose(component.kind);
    const installButtonLabel = integrationInstallButtonLabel(component);
    componentElements.installButton.textContent = installButtonLabel;
    componentElements.installButton.hidden = component.visualState === "ready"
      || component.token === "manual_action_required";
    const componentName = component.kind === "companion"
      ? "VS Code"
      : component.kind === "cursorCompanion"
        ? "Cursor"
        : component.kind === "antigravityIde"
          ? "Antigravity"
          : component.kind === "cursor"
            ? "Cursor hooks only"
            : component.kind === "antigravity"
              ? "Antigravity activity hooks"
              : component.kind === "gemini"
                ? "Gemini usage hook"
                : component.kind === "codex"
                  ? "Codex activity hooks"
                  : "Claude Code activity hooks";
    componentElements.installButton.setAttribute(
      "aria-label",
      installButtonLabel === component.actionLabel
        ? `${component.actionLabel} ${componentName}`
        : installButtonLabel,
    );
    componentElements.uninstallButton.hidden = !component.installed;

    let meta = "";
    if (["companion", "cursorCompanion", "antigravityIde"].includes(component.kind)) {
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
    componentElements.meta.textContent = "";
    componentElements.meta.hidden = true;
    const helpDetails = [component.detail, meta].filter(Boolean);
    componentElements.helpDetail.textContent = helpDetails.length
      ? `Current status: ${helpDetails.join(" · ")}`
      : "Current status details are unavailable.";
  }

  function integrationComponentName(kind: IntegrationKind): string {
    switch (kind) {
      case "companion":
        return "VS Code companion";
      case "cursorCompanion":
        return "Cursor companion";
      case "antigravityIde":
        return "Antigravity IDE companion";
      case "cursor":
        return "Cursor activity hooks";
      case "antigravity":
        return "Antigravity activity hooks";
      case "gemini":
        return "Gemini usage hook";
      case "codex":
        return "Codex activity hooks";
      case "claude":
        return "Claude Code activity hooks";
    }
  }

  function combineIntegrationComponents(
    kind: "cursorCompanion" | "antigravityIde",
    components: readonly IntegrationComponent[],
  ): IntegrationComponent {
    const manualComponent = components.find(
      (component) => component.token === "manual_action_required",
    );
    const manualActionOnly = Boolean(manualComponent) && components.every(
      (component) => component.token === "manual_action_required" || component.visualState === "ready",
    );
    const allReady = components.every((component) => component.visualState === "ready");
    const allMissing = components.every((component) => component.visualState === "missing");
    const installed = components.some((component) => component.installed);
    const anyUnavailable = components.some(
      (component) => component.visualState === "error" || component.token === "unavailable",
    );
    const visualState: IntegrationVisualState = allReady
      ? "ready"
      : allMissing
        ? "missing"
        : anyUnavailable && !installed
          ? "error"
          : "warning";
    const actionLabel = components.some((component) => component.actionLabel === "Update")
      ? "Update"
      : allMissing
        ? "Install"
        : "Repair";
    const detail = components
      .map((component) => `${integrationComponentName(component.kind)}: ${component.detail}`)
      .join(" · ");

    return {
      ...components[0],
      kind,
      optional: false,
      token: manualActionOnly ? "manual_action_required" : components[0].token,
      visualState,
      installed,
      actionLabel,
      label: manualActionOnly
        ? manualComponent?.label ?? "Manual action required"
        : visualState === "ready"
          ? "Installed"
          : visualState === "missing"
            ? "Not installed"
            : visualState === "error"
              ? "Unavailable"
              : "Needs attention",
      detail,
      installedVersion: "",
      targetVersion: "",
      configPath: "",
    };
  }

  function visibleIntegrationComponent(
    status: IntegrationStatus,
    kind: VisibleIntegrationKind,
  ): IntegrationComponent {
    if (kind === "cursorCompanion") {
      return combineIntegrationComponents(kind, [status.cursorCompanion, status.cursor]);
    }
    if (kind === "antigravityIde") {
      return combineIntegrationComponents(kind, [status.antigravityIde, status.antigravity]);
    }
    return status[kind];
  }

  function integrationPurpose(kind: IntegrationKind): string {
    switch (kind) {
      case "companion":
        return "Live workspace detection";
      case "cursorCompanion":
        return "Live workspaces and agent activity";
      case "antigravityIde":
        return "Live workspaces and agent activity";
      case "cursor":
        return "Recent workspace and agent activity";
      case "antigravity":
        return "Recent agent activity · start a turn";
      case "gemini":
        return "Local model-call token totals";
      case "codex":
      case "claude":
        return "Lifecycle hooks · CLI usage";
    }
  }

  function integrationInstallButtonLabel(component: IntegrationComponent): string {
    if (component.kind === "cursor") {
      return component.actionLabel === "Install"
        ? "Install hooks only"
        : "Repair hooks only";
    }
    return component.actionLabel;
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
      && state.integrationAction.operation === "install"
      ? "Setting up…"
      : "Set up monitoring";

    VISIBLE_INTEGRATION_KINDS.forEach((kind) => {
      const component = status ? visibleIntegrationComponent(status, kind) : null;
      const componentElements = getIntegrationElements(kind);
      const isCurrentAction = action?.kind === kind;
      componentElements.card.setAttribute("aria-busy", String(isCurrentAction));
      componentElements.installButton.disabled = busy || !component;
      componentElements.uninstallButton.disabled = busy || !component?.installed;

      if (component) {
        componentElements.installButton.textContent =
          isCurrentAction && action?.operation === "install"
            ? integrationProgressLabel(component, "install")
            : integrationInstallButtonLabel(component);
        const uninstallLabel = "Uninstall";
        componentElements.uninstallButton.textContent =
          isCurrentAction && action?.operation === "uninstall"
            ? "Uninstalling…"
            : uninstallLabel;
      }
    });
    // Keep the global stop-tracking action available even when an editor CLI
    // is unavailable or external status says "not installed". Those are the
    // cases where lingering records from a still-running process most need the
    // backend's source suppression and purge fallback.
    elements.uninstallAllButton.disabled = busy || !status;
    elements.uninstallAllButton.textContent = action?.kind === "all"
      && action.operation === "uninstall"
      ? "Uninstalling…"
      : "Uninstall all";
  }

  function summarizeEditorCompanions(status: IntegrationStatus): {
    installed: boolean;
    warningCount: number;
  } {
    const companions = [
      visibleIntegrationComponent(status, "companion"),
      visibleIntegrationComponent(status, "cursorCompanion"),
      visibleIntegrationComponent(status, "antigravityIde"),
    ];
    return {
      installed: companions.some((component) => component.installed),
      warningCount: companions.filter((component) =>
        ["warning", "error"].includes(component.visualState)
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
    const optionalComponents = [status.gemini, status.codex, status.claude];
    const optionalMissing = optionalComponents.some(
      (component) => component.visualState === "missing",
    );
    const optionalWarnings = optionalComponents.filter((component) =>
      ["warning", "error"].includes(component.visualState)
    ).length;
    const totalWarnings = editorSummary.warningCount
      + optionalWarnings
      + diagnosticWarningCount
      + Number(diagnosticsUnavailable);

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
    elements.diagnosticsSummary.dataset.attention = String(attention);
    elements.diagnosticsSummary.setAttribute("aria-label", `Setup status: ${summary}`);
    const details: string[] = [];
    if (state.integrationStatus) {
      const integrationGroups: Array<{
        name: string;
        kind: VisibleIntegrationKind;
        components: readonly IntegrationComponent[];
      }> = [
        {
          name: "VS Code",
          kind: "companion",
          components: [state.integrationStatus.companion],
        },
        {
          name: "Cursor",
          kind: "cursorCompanion",
          components: [state.integrationStatus.cursorCompanion, state.integrationStatus.cursor],
        },
        {
          name: "Antigravity",
          kind: "antigravityIde",
          components: [state.integrationStatus.antigravityIde, state.integrationStatus.antigravity],
        },
        { name: "Gemini", kind: "gemini", components: [state.integrationStatus.gemini] },
        { name: "Codex", kind: "codex", components: [state.integrationStatus.codex] },
        {
          name: "Claude Code",
          kind: "claude",
          components: [state.integrationStatus.claude],
        },
      ];
      integrationGroups.forEach(({ name, kind, components }) => {
        const visibleComponent = visibleIntegrationComponent(state.integrationStatus!, kind);
        if (["warning", "error"].includes(visibleComponent.visualState)) {
          const warnings = components.filter(
            (component) => component.visualState !== "ready",
          );
          details.push(
            `${name}: ${warnings.map((component) =>
              `${integrationComponentName(component.kind)} — ${component.detail}`
            ).join("; ")}`,
          );
        }
      });
      if (!summarizeEditorCompanions(state.integrationStatus).installed) {
        details.push("Install at least one editor integration to begin live monitoring.");
      }
    }
    details.push(...state.diagnosticWarnings);
    if (state.diagnosticsUnavailable) {
      details.push("Advanced diagnostics could not be loaded.");
    }
    elements.diagnosticsSummaryDetail.textContent = details.length
      ? details.join(" ")
      : summary === "Local only"
        ? "Setup status checks have not completed yet."
        : summary === "Optional setup"
          ? "Editor monitoring is ready. Optional provider hooks can be added if wanted."
          : "Editor monitoring and local diagnostics are ready.";
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
    renderIntegrationComponent(status.cursorCompanion);
    renderIntegrationComponent(status.antigravityIde);
    renderIntegrationComponent(status.gemini);
    renderIntegrationComponent(status.codex);
    renderIntegrationComponent(status.claude);
    renderIntegrationComponent(visibleIntegrationComponent(status, "cursorCompanion"));
    renderIntegrationComponent(visibleIntegrationComponent(status, "antigravityIde"));
    elements.restartNotice.hidden = !status.requiresRestart;
    const codexReviewRequired = status.codex.reviewRequired === true;
    elements.codexTrustGuidance.dataset.active = String(codexReviewRequired);
    elements.codexTrustGuidance.hidden = !codexReviewRequired;
    updateIntegrationControls();
    updateSetupSummary();
  }

  function describeCursorAgentsBridgeStatus(
    status: CursorAgentsBridgeStatus,
  ): { state: "neutral" | "ready" | "warning" | "error"; label: string; detail: string } {
    if (status.availability === "disabled") {
      return {
        state: "neutral",
        label: "Off",
        detail: "Off by default",
      };
    }

    if (status.availability === "connected" && status.connected) {
      const instances = `${status.instanceCount} instance${status.instanceCount === 1 ? "" : "s"}`;
      const threads = `${status.threadCount} thread${status.threadCount === 1 ? "" : "s"}`;
      const checked = status.lastCheckedAtMs === null
        ? ""
        : formatRelativeTime(status.lastCheckedAtMs).toLowerCase();
      return {
        state: "ready",
        label: "Connected",
        detail: [`${instances} · ${threads}`, checked].filter(Boolean).join(" · "),
      };
    }

    if (status.availability === "waiting") {
      return {
        state: "warning",
        label: "Not connected",
        detail: "Desktop Bridge is not available",
      };
    }

    if (status.availability === "unsupported") {
      return {
        state: "warning",
        label: "Unavailable",
        detail: "Unsupported in this build or platform",
      };
    }

    return {
      state: "error",
      label: "Check failed",
      detail: "Could not safely check Desktop Bridge",
    };
  }

  function updateCursorAgentsBridgeControls(): void {
    const pending = state.cursorAgentsBridgePending;
    elements.cursorAgentsBridgeCard.setAttribute("aria-busy", String(pending));
    elements.cursorAgentsMonitoringEnabled.disabled = pending;
  }

  function renderCursorAgentsBridgeStatus(status: CursorAgentsBridgeStatus): void {
    state.cursorAgentsBridgeStatus = status;
    const description = describeCursorAgentsBridgeStatus(status);
    elements.cursorAgentsBridgeCard.dataset.state = description.state;
    elements.cursorAgentsBridgeStatus.dataset.state = description.state;
    elements.cursorAgentsBridgeStatus.textContent = description.label;
    elements.cursorAgentsBridgeDetail.textContent = description.detail;
    elements.cursorAgentsBridgeHelpStatus.textContent = status.detail
      ? `Current status: ${status.detail}`
      : "Current status details are unavailable.";
    elements.cursorAgentsMonitoringEnabled.checked = status.enabled;
    updateCursorAgentsBridgeControls();
    updateIntegrationControls();
  }

  function setCursorAgentsBridgeMessage(message: string, error = false): void {
    elements.cursorAgentsBridgeMessage.textContent = message;
    elements.cursorAgentsBridgeMessage.hidden = !message;
    elements.cursorAgentsBridgeMessage.classList.toggle("has-error", error);
    elements.cursorAgentsBridgeMessage.classList.toggle("has-success", Boolean(message) && !error);
  }

  async function refreshCursorAgentsBridgeStatus(): Promise<void> {
    if (state.cursorAgentsBridgePending) {
      return;
    }

    state.cursorAgentsBridgePending = true;
    updateCursorAgentsBridgeControls();
    try {
      const raw = await invoke("get_cursor_agents_bridge_status", {});
      renderCursorAgentsBridgeStatus(normalizeCursorAgentsBridgeStatus(raw));
      setCursorAgentsBridgeMessage("");
    } catch (_error) {
      const previous = state.cursorAgentsBridgeStatus;
      renderCursorAgentsBridgeStatus({
        schemaVersion: SCHEMA_VERSION,
        enabled: previous?.enabled ?? false,
        availability: "error",
        connected: false,
        instanceCount: 0,
        threadCount: 0,
        lastCheckedAtMs: null,
        errorCode: "",
        detail: "VSParallel could not check experimental Cursor agent-thread monitoring.",
      });
    } finally {
      state.cursorAgentsBridgePending = false;
      updateCursorAgentsBridgeControls();
    }
  }

  async function setCursorAgentsMonitoringEnabled(enabled: boolean): Promise<void> {
    if (state.cursorAgentsBridgePending) {
      return;
    }

    const previous = state.cursorAgentsBridgeStatus;
    state.cursorAgentsBridgePending = true;
    setCursorAgentsBridgeMessage("");
    updateCursorAgentsBridgeControls();
    try {
      const raw = await invoke("set_cursor_agents_monitoring_enabled", { enabled });
      const status = normalizeCursorAgentsBridgeStatus(raw);
      renderCursorAgentsBridgeStatus(status);
      setCursorAgentsBridgeMessage(
        status.enabled
          ? "Experimental Cursor agent-thread monitoring enabled."
          : "Experimental Cursor agent-thread monitoring disabled.",
      );
      await refreshSnapshot();
    } catch (_error) {
      if (previous) {
        renderCursorAgentsBridgeStatus(previous);
      } else {
        elements.cursorAgentsMonitoringEnabled.checked = false;
      }
      setCursorAgentsBridgeMessage(
        "VSParallel could not save the Cursor agent-thread monitoring preference.",
        true,
      );
    } finally {
      state.cursorAgentsBridgePending = false;
      updateCursorAgentsBridgeControls();
    }
  }

  function setIntegrationMessage(
    message: string,
    kind: IntegrationMessageKind = "neutral",
  ): void {
    elements.integrationMessage.textContent = message;
    elements.integrationMessage.hidden = !message;
    elements.integrationMessage.classList.toggle("has-warning", kind === "warning");
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
          cursorCompanion: { state: "error", label: "Check failed", detail: message },
          antigravityIde: { state: "error", label: "Check failed", detail: message },
          cursor: { state: "error", label: "Check failed", detail: message },
          antigravity: { state: "error", label: "Check failed", detail: message },
          gemini: { state: "error", label: "Check failed", detail: message },
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
        return "VS Code integration uninstalled.";
      }
      if (kind === "cursorCompanion") {
        return "Cursor integration uninstalled.";
      }
      if (kind === "antigravityIde") {
        return "Antigravity integration uninstalled.";
      }
      if (kind === "gemini") {
        return "Gemini usage hook uninstalled.";
      }
      const provider = kind === "cursor"
        ? "Cursor"
        : kind === "antigravity"
          ? "Antigravity"
          : kind === "codex"
            ? "Codex"
            : "Claude Code";
      return `${provider} activity hooks uninstalled.`;
    }
    if (kind === "companion") {
      return "VS Code companion installed. Reload open VS Code windows.";
    }
    if (kind === "cursorCompanion") {
      return "Cursor monitoring installed. Reload open Cursor IDE windows or open a new Cursor Agent CLI session, then start a turn.";
    }
    if (kind === "antigravityIde") {
      return "Antigravity integration installed. Reload open Antigravity IDE windows and start a new agent turn.";
    }
    if (kind === "antigravity") {
      return "Antigravity hooks installed. Start a new agent turn.";
    }
    if (kind === "cursor") {
      return "Cursor hooks installed. Open a workspace or start a new turn.";
    }
    if (kind === "gemini") {
      return "Gemini usage hook installed. Open a new Gemini CLI session and start a turn.";
    }
    return kind === "codex"
      ? "Codex hooks installed. Review /hooks in Codex."
      : "Claude Code hooks installed. Restart active sessions.";
  }

  function integrationActionComponent(
    status: IntegrationStatus,
    kind: IntegrationKind,
  ): IntegrationComponent {
    if (kind === "cursorCompanion" || kind === "antigravityIde") {
      return visibleIntegrationComponent(status, kind);
    }
    return status[kind];
  }

  function integrationManualActionSummary(
    name: string,
    component: IntegrationComponent,
  ): string {
    const detail = trimTerminalPunctuation(component.detail || component.label);
    return `${name}: ${detail || "manual action is required"}`;
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
      cursorCompanion: {
        install: "install_cursor_monitoring",
        uninstall: "uninstall_cursor_monitoring",
        name: "Cursor monitoring",
      },
      antigravityIde: {
        install: "install_antigravity_monitoring",
        uninstall: "uninstall_antigravity_monitoring",
        name: "Antigravity monitoring",
      },
      antigravity: {
        install: "install_antigravity_hooks",
        uninstall: "uninstall_antigravity_hooks",
        name: "Antigravity activity hooks",
      },
      cursor: {
        install: "install_cursor_hooks",
        uninstall: "uninstall_cursor_hooks",
        name: "Cursor hooks only",
      },
      gemini: {
        install: "install_gemini_usage",
        uninstall: "uninstall_gemini_usage",
        name: "Gemini usage hook",
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
      const status = normalizeIntegrationStatus(raw);
      const resultComponent = integrationActionComponent(status, kind);
      renderIntegrationStatus(status);
      if (operation === "install" && resultComponent.token === "manual_action_required") {
        setIntegrationMessage(
          `Setup needs attention. ${integrationManualActionSummary(componentName, resultComponent)}.`,
          "warning",
        );
      } else {
        setIntegrationMessage(integrationActionSuccess(kind, operation), "success");
      }
      if (operation === "uninstall") {
        await Promise.all([refreshSnapshot(), refreshUsage(true)]);
      }
    } catch (error) {
      const message = readableError(error, `Could not ${operation} ${componentName}.`);
      try {
        const raw = await invoke("get_integration_status", {});
        renderIntegrationStatus(normalizeIntegrationStatus(raw));
      } catch (_statusError) {
        // Preserve the operation's actionable error if the follow-up status
        // query is also unavailable.
      }
      setIntegrationMessage(
        message,
        "error",
      );
      if (operation === "uninstall") {
        await Promise.all([refreshSnapshot(), refreshUsage(true)]);
      }
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
    if (status.cursorCompanion.token !== "unavailable") {
      kinds.push("cursorCompanion");
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
    const editorSteps = editorKinds.map((kind) => {
      if (kind === "companion") {
        return {
          kind,
          name: "VS Code companion",
          editorName: "VS Code",
          command: "install_companion" as const,
        };
      }
      if (kind === "cursorCompanion") {
        return {
          kind,
          name: "Cursor monitoring",
          editorName: "Cursor IDE",
          command: "install_cursor_monitoring" as const,
        };
      }
      return {
        kind,
        name: "Antigravity monitoring",
        editorName: "Antigravity IDE",
        command: "install_antigravity_monitoring" as const,
      };
    });
    const steps: Array<{
      kind: IntegrationKind;
      name: string;
      command: TauriCommand;
    }> = [
      ...editorSteps,
      {
        kind: "gemini",
        name: "Gemini usage hook",
        command: "install_gemini_usage",
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
    const manualActions: string[] = [];
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
        const status = normalizeIntegrationStatus(raw);
        const resultComponent = integrationActionComponent(status, step.kind);
        renderIntegrationStatus(status);
        if (resultComponent.token === "manual_action_required") {
          manualActions.push(integrationManualActionSummary(step.name, resultComponent));
        } else {
          completed.push(step.name);
        }
      } catch (error) {
        unconfirmed.push({
          name: step.name,
          error: readableError(error, "The operation did not return a confirmed result."),
        });
      }
    }

    if (unconfirmed.length === 0 && manualActions.length === 0) {
      const editorNames = editorSteps.map((step) => step.editorName);
      const successMessage = editorNames.length
        ? "Monitoring installed. Reload affected editors, restart provider sessions, and review /hooks in Codex."
        : "Activity hooks installed. No editor companion was available; restart provider sessions and review /hooks in Codex.";
      setIntegrationMessage(
        successMessage,
        "success",
      );
    } else if (unconfirmed.length === 0) {
      const completedSummary = completed.length
        ? `${formatNaturalList(completed)} completed. `
        : "";
      setIntegrationMessage(
        `Setup needs attention. ${completedSummary}Manual action remains: ${manualActions.join(" · ")}.`,
        "warning",
      );
    } else if (completed.length || manualActions.length) {
      const completedSummary = completed.length
        ? `${formatNaturalList(completed)} completed. `
        : "";
      const manualSummary = manualActions.length
        ? `Manual action remains: ${manualActions.join(" · ")}. `
        : "";
      const failureDetails = unconfirmed
        .map((failure) => `${failure.name}: ${trimTerminalPunctuation(failure.error)}`)
        .join(" · ");
      setIntegrationMessage(
        `Partial setup: ${completedSummary}${manualSummary}The following could not be completed or verified: ${failureDetails}. Successful changes were kept; retry to finish setup.`,
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

  async function uninstallAllIntegrations(): Promise<void> {
    if (state.integrationAction || state.integrationPending) {
      return;
    }

    state.integrationAction = { kind: "all", operation: "uninstall" };
    updateIntegrationControls();
    setIntegrationMessage("Uninstalling all VSParallel integrations…");

    try {
      const raw = await invoke("uninstall_all_integrations", {});
      const status = normalizeIntegrationStatus(raw);
      renderIntegrationStatus(status);
      const unverifiedEditors = [
        { name: "VS Code", component: status.companion },
        { name: "Cursor", component: status.cursorCompanion },
        { name: "Antigravity IDE", component: status.antigravityIde },
      ]
        .filter(({ component }) => component.token === "unavailable")
        .map(({ name }) => name);
      if (unverifiedEditors.length) {
        setIntegrationMessage(
          `Integration-backed editor monitoring was disabled and its saved observations were cleared. Physical companion removal could not be verified for ${formatNaturalList(unverifiedEditors)} because ${unverifiedEditors.length === 1 ? "its editor CLI was" : "their editor CLIs were"} unavailable. Make the affected CLI available and run Uninstall all again, or remove the VSParallel companion manually. Automatic Zed discovery and supported provider quota checks remain available.`,
          "warning",
        );
      } else {
        setIntegrationMessage(
          "Integration-backed editor monitoring was disabled and its saved observations were cleared. VSParallel removed or verified the absence of each companion, activity hook, and local usage hook. Automatic Zed discovery and supported provider quota checks remain available.",
          "success",
        );
      }
      await Promise.all([
        refreshSnapshot(),
        refreshUsage(true),
        refreshCursorAgentsBridgeStatus(),
      ]);
    } catch (error) {
      const message = readableError(error, "Could not uninstall all integrations.");
      try {
        const raw = await invoke("get_integration_status", {});
        renderIntegrationStatus(normalizeIntegrationStatus(raw));
      } catch (_statusError) {
        // Keep the original uninstall error when status refresh also fails.
      }
      setIntegrationMessage(message, "error");
      // The backend still applies its integration-source suppression markers
      // and purges retained observations when physical removal is only partial.
      // Reflect that fail-safe immediately while preserving the actionable error.
      await Promise.all([
        refreshSnapshot(),
        refreshUsage(true),
        refreshCursorAgentsBridgeStatus(),
      ]);
    } finally {
      state.integrationAction = null;
      updateIntegrationControls();
    }
  }

  function openSettingsDialog(): void {
    if (showAccessibleDialog(elements.settingsDialog, elements.settingsCloseButton)) {
      refreshSetup();
    }
  }

  function closeSettingsDialog(): void {
    closeActiveHelpPopover();
    closeAccessibleDialog(elements.settingsDialog);
  }

  function requestUninstall(kind: IntegrationActionKind): void {
    if (state.integrationAction) {
      return;
    }

    const componentNames: Record<IntegrationActionKind, string> = {
      companion: "VS Code companion",
      cursorCompanion: "Cursor integration",
      antigravityIde: "Antigravity integration",
      cursor: "Cursor hooks only",
      antigravity: "Antigravity activity hooks",
      gemini: "Gemini usage hook",
      codex: "Codex activity hooks",
      claude: "Claude Code activity hooks",
      all: "all integrations",
    };
    state.pendingUninstall = kind;
    elements.uninstallTitle.textContent = kind === "all"
      ? "Uninstall all integrations?"
      : `Uninstall ${componentNames[kind]}?`;
    if (kind === "all") {
      elements.uninstallDescription.textContent = "VSParallel will stop integration-backed editor monitoring, clear its saved observations, disable its experimental bridge preference, and attempt to remove its editor companions, activity hooks, and local usage hooks. Automatic Zed discovery and supported provider quota checks remain available. Project files and unrelated hooks are not changed.";
    } else if (["companion", "cursorCompanion", "antigravityIde"].includes(kind)) {
      const editorName = kind === "companion"
        ? "VS Code"
        : kind === "cursorCompanion"
          ? "Cursor"
          : "Antigravity IDE";
      elements.uninstallDescription.textContent = `VSParallel will stop displaying and accepting ${editorName} monitoring immediately. Reload open ${editorName} windows to stop their old extension host. No project files are removed.`;
    } else if (kind === "gemini") {
      elements.uninstallDescription.textContent = "VSParallel will remove only its Gemini token-usage hook. Other Gemini hooks and project files remain unchanged.";
    } else {
      const provider = kind === "cursor"
        ? "Cursor"
        : kind === "antigravity"
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
    if (kind === "all") {
      await uninstallAllIntegrations();
    } else {
      await runIntegrationAction(kind, "uninstall");
    }
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

  function describeCursorHeartbeatDiagnostic(
    activeRecords: number,
    retainedRecords: number,
    latestDescription: string,
    recentWorkspaceOpens: number,
  ): string {
    if (activeRecords > 0) {
      return `Active · latest ${latestDescription}`;
    }
    if (retainedRecords > 0) {
      return `Inactive · latest ${latestDescription} · unmatched Agents Window activity is hook-only`;
    }
    if (recentWorkspaceOpens > 0) {
      return "Hook activity observed · no live Cursor IDE heartbeat; an exact experimental bridge match is required for live thread status";
    }
    return "Not observed · set up Cursor monitoring and reload Cursor IDE";
  }

  function describeAntigravityHookExecution(
    raw: JsonObject,
    fieldPrefix: "antigravityTwoHook" | "antigravityIdeHook",
    surfaceName: "Antigravity 2.0" | "Antigravity IDE",
  ): { outcome: string; warning: boolean; detail: string } {
    const rawOutcome = raw[`${fieldPrefix}Outcome`];
    const outcome = rawOutcome === undefined
      ? "not_observed"
      : normalizeStateToken(rawOutcome);
    const event = asString(raw[`${fieldPrefix}Event`]).replaceAll("-", " ");
    const observedAt = asTimestamp(raw[`${fieldPrefix}ObservedAtMs`]);
    const warning = !["not_observed", "recorded"].includes(outcome);
    let detail = `Not observed · start an ${surfaceName} agent turn`;

    if (outcome === "recorded") {
      const observed = formatRelativeTime(observedAt).toLowerCase();
      detail = [
        event || "agent event",
        observed,
        "workspace activity recorded",
      ].join(" · ");
    } else if (outcome === "no_workspace") {
      detail = "Observed, but no usable local workspace path was reported";
    } else if (outcome === "missing_conversation") {
      detail = "Observed, but no conversation identifier was reported";
    } else if (outcome === "persist_failed") {
      detail = "Observed, but VSParallel could not save workspace activity";
    } else if (outcome === "health_unreadable") {
      detail = "Local hook execution health could not be read";
    } else if (outcome !== "not_observed") {
      detail = "Observed, but the event could not be used for workspace activity";
    }

    return { outcome, warning, detail };
  }

  function antigravityHookWasObserved(outcome: string): boolean {
    return !["not_observed", "health_unreadable"].includes(outcome);
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
    const malformedCursor = asNonNegativeInteger(raw.malformedCursorRecords);
    const malformedZed = asNonNegativeInteger(raw.malformedZedRecords);
    const omittedInstances = asNonNegativeInteger(raw.omittedInstanceRecords);
    const omittedCodex = asNonNegativeInteger(raw.omittedCodexRecords);
    const omittedClaude = asNonNegativeInteger(raw.omittedClaudeRecords);
    const omittedAntigravity = asNonNegativeInteger(raw.omittedAntigravityRecords);
    const omittedCursor = asNonNegativeInteger(raw.omittedCursorRecords);
    const omittedZed = asNonNegativeInteger(raw.omittedZedRecords);
    const validInstances = asNonNegativeInteger(raw.validInstanceRecords);
    const validCodex = asNonNegativeInteger(raw.validCodexRecords);
    const validClaude = asNonNegativeInteger(raw.validClaudeRecords);
    const validAntigravity = asNonNegativeInteger(raw.validAntigravityRecords);
    const validCursor = asNonNegativeInteger(raw.validCursorRecords);
    const validZedWorkspaces = asNonNegativeInteger(raw.validZedWorkspaceRecords);
    const activeZedWorkspaces = asNonNegativeInteger(raw.activeZedWorkspaceRecords);
    const validZedAgents = asNonNegativeInteger(raw.validZedAgentRecords);
    const zedChannelsLoaded = asNonNegativeInteger(raw.zedChannelsLoaded);
    const zedModelsLoaded = asNonNegativeInteger(raw.zedModelsLoaded);
    const zedAgentMetadataChannels = asNonNegativeInteger(raw.zedAgentMetadataChannels);
    const zedModelRowsConsidered = asNonNegativeInteger(raw.zedModelRowsConsidered);
    const ambiguousZedChannels = asNonNegativeInteger(raw.ambiguousZedLiveChannels);
    const activeCursorInstances = asNonNegativeInteger(raw.activeCursorInstanceRecords);
    const retainedCursorInstances = asNonNegativeInteger(raw.retainedCursorInstanceRecords);
    const latestCursorInstance = asTimestamp(raw.latestCursorInstanceAtMs);
    const recentCursorWorkspaceOpens = asNonNegativeInteger(
      raw.recentCursorWorkspaceOpenRecords,
    );
    const latestCursorWorkspaceOpen = asTimestamp(raw.latestCursorWorkspaceOpenedAtMs);
    const antigravityTwoHook = describeAntigravityHookExecution(
      raw,
      "antigravityTwoHook",
      "Antigravity 2.0",
    );
    const antigravityIdeHook = describeAntigravityHookExecution(
      raw,
      "antigravityIdeHook",
      "Antigravity IDE",
    );
    const antigravityHookWarning = antigravityTwoHook.warning || antigravityIdeHook.warning;
    const antigravityHookHealthUnreadable = [antigravityTwoHook, antigravityIdeHook]
      .some((hook) => hook.outcome === "health_unreadable");
    const antigravityHookObserved = [antigravityTwoHook, antigravityIdeHook]
      .some((hook) => antigravityHookWasObserved(hook.outcome));
    const totalMalformed = malformedInstances
      + malformedCodex
      + malformedClaude
      + malformedAntigravity
      + malformedCursor
      + malformedZed;
    const totalOmitted = omittedInstances
      + omittedCodex
      + omittedClaude
      + omittedAntigravity
      + omittedCursor
      + omittedZed;

    elements.diagnosticsList.replaceChildren();
    appendDiagnostic("State directory", asString(raw.stateDirectory, "Unavailable"));
    appendDiagnostic("VS Code command", asString(raw.codeCommand, "code"));
    appendDiagnostic("Cursor command", asString(raw.cursorCommand, "cursor"));
    appendDiagnostic("Zed command", asString(raw.zedCommand, "zed"));
    appendDiagnostic(
      "Antigravity IDE command",
      asString(raw.antigravityIdeCommand, "antigravity-ide"),
    );
    appendDiagnostic(
      "Cursor live heartbeat",
      describeCursorHeartbeatDiagnostic(
        activeCursorInstances,
        retainedCursorInstances,
        formatRelativeTime(latestCursorInstance).toLowerCase(),
        recentCursorWorkspaceOpens,
      ),
    );
    appendDiagnostic(
      "Cursor workspace hook",
      recentCursorWorkspaceOpens
        ? `Observed · latest ${formatRelativeTime(latestCursorWorkspaceOpen).toLowerCase()}`
        : "Not observed · open a local Cursor workspace after installing hooks",
    );
    appendDiagnostic(
      "Antigravity 2.0 hook execution",
      antigravityTwoHook.detail,
      antigravityTwoHook.warning,
    );
    appendDiagnostic(
      "Antigravity IDE hook execution",
      antigravityIdeHook.detail,
      antigravityIdeHook.warning,
    );
    appendDiagnostic("Active heartbeat window", formatDuration(raw.activeTtlMs));
    appendDiagnostic("Inactive record retention", formatDuration(raw.staleRetentionMs));
    appendDiagnostic(
      "Activity freshness",
      formatDuration(raw.activityStaleMs ?? raw.codexStaleMs),
    );

    state.diagnosticWarningCount = Number(totalMalformed > 0)
      + Number(totalOmitted > 0)
      + Number(antigravityTwoHook.warning)
      + Number(antigravityIdeHook.warning);
    state.diagnosticWarnings = [];
    if (totalMalformed) {
      state.diagnosticWarnings.push(
        `${totalMalformed} malformed local ${totalMalformed === 1 ? "record was" : "records were"} ignored.`,
      );
    }
    if (totalOmitted) {
      state.diagnosticWarnings.push(
        `${totalOmitted} local ${totalOmitted === 1 ? "record was" : "records were"} omitted for safety.`,
      );
    }
    if (antigravityTwoHook.warning) {
      state.diagnosticWarnings.push(`Antigravity 2.0: ${antigravityTwoHook.detail}.`);
    }
    if (antigravityIdeHook.warning) {
      state.diagnosticWarnings.push(`Antigravity IDE: ${antigravityIdeHook.detail}.`);
    }
    state.diagnosticsLoaded = true;
    state.diagnosticsUnavailable = false;
    updateSetupSummary();
    elements.diagnosticsStatus.classList.remove("has-error");
    elements.diagnosticsStatus.textContent = validInstances
      || validAntigravity
      || validCursor
      || validZedWorkspaces
      ? "Local monitoring data is available."
      : antigravityHookHealthUnreadable
        ? "Some Antigravity hook health records are unreadable."
        : !antigravityHookObserved
          ? "No workspace activity observed yet."
          : antigravityHookWarning
            ? "The latest Antigravity event was not recorded."
            : "No valid workspace source found.";
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
      state.diagnosticsLoaded = false;
      state.diagnosticWarningCount = 0;
      state.diagnosticWarnings = [];
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

  async function refreshSetup(): Promise<[void, void, void, void] | undefined> {
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
      refreshCursorAgentsBridgeStatus(),
      refreshDiagnostics(),
      refreshDisplayPreferences(),
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
    if (closeActiveHelpPopover()) {
      return;
    }
    if (isDialogOpen(elements.uninstallDialog)) {
      closeUninstallDialog();
    } else if (isDialogOpen(elements.settingsDialog)) {
      closeSettingsDialog();
    } else {
      const hideAction = state.windowChrome?.floating ? hideFloatingPanel : hideWindow;
      hideAction();
    }
  }

  elements.experimentalIntegrations.append(elements.cursorAgentsBridgeCard);
  elements.cursorAgentsBridgeCard.hidden = false;
  initializeHelpPopovers();

  elements.refreshButton.addEventListener("click", refreshAll);
  elements.emptySetupButton.addEventListener("click", openSettingsDialog);
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
  elements.editorVisibilityInputs.forEach((input) => {
    input.addEventListener("change", () => {
      void updateEditorVisibility(input);
    });
  });
  elements.usageVisibilityInput.addEventListener("change", () => {
    void updateUsageVisibility(elements.usageVisibilityInput.checked);
  });
  elements.cursorAgentsMonitoringEnabled.addEventListener("change", () => {
    void setCursorAgentsMonitoringEnabled(elements.cursorAgentsMonitoringEnabled.checked);
  });
  elements.diagnosticsRefreshButton.addEventListener("click", refreshSetup);
  elements.setupAllButton.addEventListener("click", setupAllIntegrations);
  elements.uninstallAllButton.addEventListener("click", () => requestUninstall("all"));
  elements.companionInstallButton.addEventListener("click", () =>
    runIntegrationAction("companion", "install"),
  );
  elements.cursorCompanionInstallButton.addEventListener("click", () =>
    runIntegrationAction("cursorCompanion", "install"),
  );
  elements.antigravityIdeInstallButton.addEventListener("click", () =>
    runIntegrationAction("antigravityIde", "install"),
  );
  elements.geminiInstallButton.addEventListener("click", () =>
    runIntegrationAction("gemini", "install"),
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
  elements.cursorCompanionUninstallButton.addEventListener("click", () =>
    requestUninstall("cursorCompanion"),
  );
  elements.antigravityIdeUninstallButton.addEventListener("click", () =>
    requestUninstall("antigravityIde"),
  );
  elements.geminiUninstallButton.addEventListener("click", () => requestUninstall("gemini"));
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
      refreshCursorAgentsBridgeStatus();
    }
    scheduleWindowChromeRefresh();
  });
  window.addEventListener("blur", () => {
    document.documentElement.dataset.windowFocused = "false";
    closeActiveHelpPopover();
    scheduleWindowChromeRefresh();
  });
  window.addEventListener("resize", scheduleWindowChromeRefresh);
  window.addEventListener("resize", () => {
    if (activeHelpPopover) {
      positionHelpPopover(activeHelpPopover.trigger, activeHelpPopover.content);
    }
  });
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
  applyVisibilityPreferences(false);
  renderUpdateState();
  refreshIntegrationStatus();
  void refreshDisplayPreferences().then(() => {
    void refreshAll();
  });
  window.setTimeout(() => {
    void checkForUpdates(false);
  }, UPDATE_CHECK_DELAY_MS);
  window.setInterval(refreshSnapshot, REFRESH_INTERVAL_MS);
  window.setInterval(refreshUsage, USAGE_REFRESH_INTERVAL_MS);
})();
