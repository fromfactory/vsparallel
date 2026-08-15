use crate::state::{ActivityView, Snapshot};
use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Wry};

const TRAY_REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const TRAY_ID: &str = "vsparallel-tray";
const TRAY_OPEN_ID: &str = "tray-open-vsparallel";
const TRAY_QUIT_ID: &str = "tray-quit-vsparallel";
const TRAY_WORKSPACE_PREFIX: &str = "tray-workspace-";
const MAX_WORKSPACE_NAME_CHARS: usize = 48;

#[cfg(target_os = "linux")]
struct TrayIconTempDirectory {
    directory: tempfile::TempDir,
}

#[cfg(target_os = "linux")]
impl TrayIconTempDirectory {
    fn create(variant: TrayIconVariant) -> Result<Self, String> {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        let prefix = format!("vsparallel-tray-{}-", variant.label());
        let directory = tempfile::Builder::new()
            .prefix(&prefix)
            .permissions(Permissions::from_mode(0o700))
            .tempdir()
            .map_err(|error| format!("could not create a private tray icon directory: {error}"))?;
        Ok(Self { directory })
    }

    fn path(&self) -> &std::path::Path {
        self.directory.path()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayIconVariant {
    Linux,
    Macos,
    Windows,
}

impl TrayIconVariant {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Linux => include_bytes!("../icons/tray-icon-linux.png"),
            Self::Macos => include_bytes!("../icons/tray-icon-macos.png"),
            Self::Windows => include_bytes!("../icons/tray-icon-windows.png"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }

    fn is_template(self) -> bool {
        matches!(self, Self::Macos)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayActivityStatus {
    Activity,
    Failure,
    Finished,
    Recent,
    Unknown,
}

impl TrayActivityStatus {
    fn priority(self) -> u8 {
        match self {
            Self::Activity => 4,
            Self::Failure => 3,
            Self::Finished => 2,
            Self::Recent => 1,
            Self::Unknown => 0,
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Activity => "●",
            Self::Failure => "!",
            Self::Finished => "✓",
            Self::Recent => "◷",
            Self::Unknown => "○",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Activity => "Activity detected",
            Self::Failure => "Failed/interrupted",
            Self::Finished => "Turn finished",
            Self::Recent => "Recent agent activity",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayWorkspaceEntry {
    menu_id: String,
    instance_id: String,
    name: String,
    editor_name: String,
    status: TrayActivityStatus,
    openable: bool,
}

impl TrayWorkspaceEntry {
    fn menu_label(&self) -> String {
        format!(
            "{} {} · {} — {}",
            self.status.marker(),
            escape_menu_mnemonics(&self.name),
            self.editor_name,
            self.status.label()
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrayMenuModel {
    entries: Vec<TrayWorkspaceEntry>,
    refresh_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayMenuAction {
    None,
    OpenMain,
    Quit,
    OpenWorkspace(String),
}

pub(crate) struct TrayMenuController {
    menu: Menu<Wry>,
    model: Mutex<Option<TrayMenuModel>>,
    workspace_ids: Mutex<HashMap<String, String>>,
}

pub(crate) struct TrayAvailability {
    available: bool,
}

impl TrayAvailability {
    pub(crate) fn new(available: bool) -> Self {
        Self { available }
    }

    pub(crate) fn is_available(&self) -> bool {
        self.available
    }
}

impl TrayMenuController {
    fn new(menu: Menu<Wry>) -> Self {
        Self {
            menu,
            model: Mutex::new(None),
            workspace_ids: Mutex::new(HashMap::new()),
        }
    }

    fn replace_model(&self, app: &AppHandle, model: TrayMenuModel) -> Result<(), String> {
        {
            let current = self
                .model
                .lock()
                .map_err(|_| "the tray menu state is unavailable".to_string())?;
            if current.as_ref() == Some(&model) {
                return Ok(());
            }
        }

        let mut item_map = HashMap::new();
        let mut items = Vec::new();
        items.push(
            MenuItem::with_id(app, "tray-heading", "Workspaces", false, None::<&str>)
                .map_err(|error| format!("could not create the tray heading: {error}"))?,
        );

        if model.refresh_failed {
            items.push(
                MenuItem::with_id(
                    app,
                    "tray-refresh-failed",
                    "Update failed · showing recent data",
                    false,
                    None::<&str>,
                )
                .map_err(|error| format!("could not create the tray warning: {error}"))?,
            );
        }

        if model.entries.is_empty() {
            items.push(
                MenuItem::with_id(
                    app,
                    "tray-empty",
                    "No active editor workspaces",
                    false,
                    None::<&str>,
                )
                .map_err(|error| format!("could not create the tray empty state: {error}"))?,
            );
        } else {
            for entry in &model.entries {
                let item = MenuItem::with_id(
                    app,
                    &entry.menu_id,
                    entry.menu_label(),
                    entry_is_enabled(&model, entry),
                    None::<&str>,
                )
                .map_err(|error| format!("could not create a tray workspace item: {error}"))?;
                item_map.insert(entry.menu_id.clone(), entry.instance_id.clone());
                items.push(item);
            }
        }

        let old_count = self
            .menu
            .items()
            .map_err(|error| format!("could not read the tray menu: {error}"))?
            .len();
        for index in (0..old_count).rev() {
            self.menu
                .remove_at(index)
                .map_err(|error| format!("could not refresh the tray menu: {error}"))?;
        }

        for item in &items {
            self.menu
                .append(item)
                .map_err(|error| format!("could not add a tray menu item: {error}"))?;
        }
        self.menu
            .append(
                &PredefinedMenuItem::separator(app)
                    .map_err(|error| format!("could not create a tray separator: {error}"))?,
            )
            .map_err(|error| format!("could not add a tray separator: {error}"))?;
        self.menu
            .append(
                &MenuItem::with_id(app, TRAY_OPEN_ID, "Open VSParallel…", true, None::<&str>)
                    .map_err(|error| format!("could not create the open-app tray item: {error}"))?,
            )
            .map_err(|error| format!("could not add the open-app tray item: {error}"))?;
        self.menu
            .append(
                &MenuItem::with_id(app, TRAY_QUIT_ID, "Quit VSParallel", true, None::<&str>)
                    .map_err(|error| format!("could not create the quit tray item: {error}"))?,
            )
            .map_err(|error| format!("could not add the quit tray item: {error}"))?;

        *self
            .workspace_ids
            .lock()
            .map_err(|_| "the tray workspace map is unavailable".to_string())? = item_map;
        *self
            .model
            .lock()
            .map_err(|_| "the tray menu state is unavailable".to_string())? = Some(model);
        Ok(())
    }

    fn replace_entries(
        &self,
        app: &AppHandle,
        entries: Vec<TrayWorkspaceEntry>,
    ) -> Result<(), String> {
        self.replace_model(app, successful_refresh_model(entries))
    }

    fn mark_refresh_failed(&self, app: &AppHandle) -> Result<(), String> {
        let entries = self
            .model
            .lock()
            .map_err(|_| "the tray menu state is unavailable".to_string())?
            .as_ref()
            .map(|model| model.entries.clone())
            .unwrap_or_default();
        self.replace_model(app, failed_refresh_model(entries))
    }

    fn action_for(&self, id: &str) -> TrayMenuAction {
        let workspace_ids = match self.workspace_ids.lock() {
            Ok(workspace_ids) => workspace_ids,
            Err(_) => return TrayMenuAction::None,
        };
        resolve_menu_action(id, &workspace_ids)
    }
}

pub(crate) fn setup(app: &mut App) -> Result<(), String> {
    let menu =
        Menu::new(app).map_err(|error| format!("could not create the tray menu: {error}"))?;
    let controller = TrayMenuController::new(menu.clone());
    controller.apply_update(app.handle(), load_update(app.handle()))?;
    app.manage(controller);

    let icon_variant = TrayIconVariant::current();
    let icon = tray_icon_image(icon_variant)?;
    let tray_builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("VSParallel workspaces")
        .icon(icon)
        .icon_as_template(icon_variant.is_template())
        .on_menu_event(handle_menu_event);

    #[cfg(target_os = "linux")]
    let tray_builder = {
        let temp_directory = TrayIconTempDirectory::create(icon_variant)?;
        let temp_path = temp_directory.path().to_path_buf();
        if !app.manage(temp_directory) {
            return Err("the private tray icon directory is already initialized".to_string());
        }
        tray_builder.temp_dir_path(temp_path)
    };

    tray_builder
        .build(app)
        .map_err(|error| format!("could not create the VSParallel tray icon: {error}"))?;

    let app_handle = app.handle().clone();
    thread::spawn(move || loop {
        thread::sleep(TRAY_REFRESH_INTERVAL);
        schedule_refresh(&app_handle);
    });
    Ok(())
}

fn tray_icon_image(variant: TrayIconVariant) -> Result<Image<'static>, String> {
    Image::from_bytes(variant.bytes()).map_err(|error| {
        format!(
            "could not decode the {} tray icon: {error}",
            variant.label()
        )
    })
}

enum TrayUpdate {
    Entries(Vec<TrayWorkspaceEntry>),
    Failed,
}

impl TrayMenuController {
    fn apply_update(&self, app: &AppHandle, update: TrayUpdate) -> Result<(), String> {
        match update {
            TrayUpdate::Entries(entries) => self.replace_entries(app, entries),
            TrayUpdate::Failed => self.mark_refresh_failed(app),
        }
    }
}

fn load_update(app: &AppHandle) -> TrayUpdate {
    match crate::current_snapshot(&app.state::<crate::SnapshotCache>(), false) {
        Ok(snapshot) => TrayUpdate::Entries(tray_workspace_entries(&snapshot)),
        Err(_) => TrayUpdate::Failed,
    }
}

fn schedule_refresh(app: &AppHandle) {
    let update = load_update(app);
    let app_handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let controller = app_handle.state::<TrayMenuController>();
        let _ = controller.apply_update(&app_handle, update);
    });
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let action = app
        .state::<TrayMenuController>()
        .action_for(event.id().as_ref());
    match action {
        TrayMenuAction::OpenMain => {
            let _ = crate::show_main_window(app);
        }
        TrayMenuAction::Quit => app.exit(0),
        TrayMenuAction::OpenWorkspace(instance_id) => {
            if crate::activate_tray_workspace(&instance_id).is_err() {
                schedule_refresh(app);
            }
        }
        TrayMenuAction::None => {}
    }
}

fn resolve_menu_action(id: &str, workspace_ids: &HashMap<String, String>) -> TrayMenuAction {
    match id {
        TRAY_OPEN_ID => TrayMenuAction::OpenMain,
        TRAY_QUIT_ID => TrayMenuAction::Quit,
        _ => workspace_ids
            .get(id)
            .cloned()
            .map(TrayMenuAction::OpenWorkspace)
            .unwrap_or(TrayMenuAction::None),
    }
}

fn successful_refresh_model(entries: Vec<TrayWorkspaceEntry>) -> TrayMenuModel {
    TrayMenuModel {
        entries,
        refresh_failed: false,
    }
}

fn failed_refresh_model(entries: Vec<TrayWorkspaceEntry>) -> TrayMenuModel {
    TrayMenuModel {
        entries,
        refresh_failed: true,
    }
}

fn entry_is_enabled(model: &TrayMenuModel, entry: &TrayWorkspaceEntry) -> bool {
    entry.openable && !model.refresh_failed
}

fn tray_workspace_entries(snapshot: &Snapshot) -> Vec<TrayWorkspaceEntry> {
    let mut entries: Vec<_> = snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace.active)
        .map(|workspace| TrayWorkspaceEntry {
            menu_id: stable_workspace_menu_id(&workspace.instance_id),
            instance_id: workspace.instance_id.clone(),
            name: sanitize_workspace_name(&workspace.name),
            editor_name: workspace.editor_name.clone(),
            status: aggregate_status(
                &workspace.codex,
                &workspace.claude,
                workspace.antigravity.as_ref(),
                workspace.cursor.as_ref(),
                workspace.zed.as_ref(),
            ),
            openable: workspace.openable,
        })
        .collect();
    entries.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.instance_id.cmp(&right.instance_id))
    });
    entries
}

fn stable_workspace_menu_id(instance_id: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut menu_id = String::with_capacity(TRAY_WORKSPACE_PREFIX.len() + instance_id.len() * 2);
    menu_id.push_str(TRAY_WORKSPACE_PREFIX);
    for byte in instance_id.as_bytes() {
        menu_id.push(HEX[(byte >> 4) as usize] as char);
        menu_id.push(HEX[(byte & 0x0f) as usize] as char);
    }
    menu_id
}

fn aggregate_status(
    codex: &ActivityView,
    claude: &ActivityView,
    antigravity: Option<&ActivityView>,
    cursor: Option<&ActivityView>,
    zed: Option<&ActivityView>,
) -> TrayActivityStatus {
    [Some(codex), Some(claude), antigravity, cursor, zed]
        .into_iter()
        .flatten()
        .map(activity_status)
        .max_by_key(|status| status.priority())
        .unwrap_or(TrayActivityStatus::Unknown)
}

fn activity_status(activity: &ActivityView) -> TrayActivityStatus {
    match activity.state.as_str() {
        "activity_detected" => TrayActivityStatus::Activity,
        "failed_or_interrupted" => TrayActivityStatus::Failure,
        "turn_finished" => TrayActivityStatus::Finished,
        "recent_activity" => TrayActivityStatus::Recent,
        _ => TrayActivityStatus::Unknown,
    }
}

fn sanitize_workspace_name(name: &str) -> String {
    let mut normalized = String::new();
    let mut previous_was_space = false;
    for character in name.chars().filter(|character| !character.is_control()) {
        if character.is_whitespace() {
            if !previous_was_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            previous_was_space = true;
        } else {
            normalized.push(character);
            previous_was_space = false;
        }
    }

    let normalized = normalized.trim();
    if normalized.is_empty() {
        return "Untitled workspace".to_string();
    }
    if normalized.chars().count() <= MAX_WORKSPACE_NAME_CHARS {
        return normalized.to_string();
    }
    let mut truncated: String = normalized
        .chars()
        .take(MAX_WORKSPACE_NAME_CHARS - 1)
        .collect();
    truncated.push('…');
    truncated
}

fn escape_menu_mnemonics(value: &str) -> String {
    value.replace('&', "&&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ActivityView, WorkspaceSurface, WorkspaceView};

    fn icon_pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> &[u8] {
        let offset = ((y * width + x) * 4) as usize;
        &rgba[offset..offset + 4]
    }

    fn activity(state: &str, detail: &str) -> ActivityView {
        ActivityView {
            state: state.to_string(),
            label: state.to_string(),
            changed_at_ms: None,
            detail: detail.to_string(),
            model_kind: None,
            model_name: None,
            agent_kind: None,
            extension_detection_available: None,
            extension_installed: None,
            extension_active: None,
            extension_remote: None,
        }
    }

    fn workspace(
        instance_id: &str,
        name: &str,
        active: bool,
        focused: bool,
        openable: bool,
        codex: &str,
        claude: &str,
    ) -> WorkspaceView {
        WorkspaceView {
            instance_id: instance_id.to_string(),
            editor: crate::opener::EditorKind::VsCode,
            editor_name: "VS Code".to_string(),
            surface: WorkspaceSurface::EditorWorkspace,
            name: name.to_string(),
            path: Some("/private/project-path".to_string()),
            openable,
            active,
            focused,
            recently_active: false,
            remote_window: false,
            last_seen_at_ms: 1,
            started_at_ms: 0,
            antigravity: None,
            cursor: None,
            zed: None,
            codex: activity(codex, "PRIVATE CODEX DETAIL"),
            claude: activity(claude, "PRIVATE CLAUDE DETAIL"),
        }
    }

    fn snapshot(workspaces: Vec<WorkspaceView>) -> Snapshot {
        Snapshot {
            schema_version: 1,
            generated_at_ms: 1,
            workspaces,
        }
    }

    #[test]
    fn tray_lists_only_active_workspaces_and_keeps_unopenable_status_visible() {
        let entries = tray_workspace_entries(&snapshot(vec![
            workspace(
                "closed",
                "closed",
                false,
                false,
                true,
                "activity_detected",
                "unknown",
            ),
            workspace(
                "remote",
                "remote",
                true,
                false,
                false,
                "unknown",
                "turn_finished",
            ),
        ]));

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].instance_id, "remote");
        assert_eq!(entries[0].status, TrayActivityStatus::Finished);
        assert!(!entries[0].openable);
    }

    #[test]
    fn color_tray_icons_use_high_resolution_reference_artwork() {
        for variant in [TrayIconVariant::Linux, TrayIconVariant::Windows] {
            let icon = tray_icon_image(variant).expect("the color tray icon should decode");
            assert_eq!((icon.width(), icon.height()), (64, 64));
            let rgba = icon.rgba();
            assert_eq!(rgba.len(), 64 * 64 * 4);
            assert_eq!(icon_pixel(rgba, 64, 0, 0), [0, 0, 0, 0]);

            let glyph = icon_pixel(rgba, 64, 22, 32);
            assert_eq!(glyph[3], 255);
            assert!(glyph[2] > 220 && glyph[1] > glyph[0] * 4);
        }
    }

    #[test]
    fn macos_tray_icon_is_a_retina_template() {
        let icon =
            tray_icon_image(TrayIconVariant::Macos).expect("the macOS tray template should decode");
        assert_eq!((icon.width(), icon.height()), (36, 36));
        let rgba = icon.rgba();
        assert_eq!(rgba.len(), 36 * 36 * 4);
        assert_eq!(icon_pixel(rgba, 36, 0, 0)[3], 0);
        assert_eq!(icon_pixel(rgba, 36, 18, 18)[3], 0);
        let template_glyph = icon_pixel(rgba, 36, 12, 18);
        assert_eq!(&template_glyph[..3], [255, 255, 255]);
        assert!(template_glyph[3] > 220);
    }

    #[test]
    fn tray_icon_selection_is_platform_specific() {
        let current = TrayIconVariant::current();
        if cfg!(target_os = "macos") {
            assert_eq!(current, TrayIconVariant::Macos);
        } else if cfg!(target_os = "windows") {
            assert_eq!(current, TrayIconVariant::Windows);
        } else {
            assert_eq!(current, TrayIconVariant::Linux);
        }

        assert_eq!(TRAY_ID, "vsparallel-tray");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tray_icon_temp_directories_are_private_randomized_and_lifetime_bound() {
        use std::os::unix::fs::PermissionsExt;

        let first = TrayIconTempDirectory::create(TrayIconVariant::Linux)
            .expect("the first private tray directory should be created");
        let first_path = first.path().to_path_buf();
        let second = TrayIconTempDirectory::create(TrayIconVariant::Linux)
            .expect("the second private tray directory should be created");

        assert_ne!(first.path(), second.path());
        assert!(first
            .path()
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("vsparallel-tray-linux-")));
        assert_eq!(
            first
                .path()
                .metadata()
                .expect("the private tray directory should have metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        drop(first);
        assert!(!first_path.exists());
        assert!(second.path().exists());
    }

    #[test]
    fn tray_order_and_ids_stay_stable_when_window_focus_changes() {
        let entries = tray_workspace_entries(&snapshot(vec![
            workspace("b", "alpha", true, false, true, "unknown", "unknown"),
            workspace("c", "Zulu", true, true, true, "unknown", "unknown"),
            workspace("a", "Alpha", true, false, true, "unknown", "unknown"),
        ]));
        let after_focus_change = tray_workspace_entries(&snapshot(vec![
            workspace("b", "alpha", true, true, true, "unknown", "unknown"),
            workspace("c", "Zulu", true, false, true, "unknown", "unknown"),
            workspace("a", "Alpha", true, false, true, "unknown", "unknown"),
        ]));

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.instance_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(entries, after_focus_change);
        assert!(entries
            .iter()
            .all(|entry| entry.menu_id.starts_with(TRAY_WORKSPACE_PREFIX)));
        assert_ne!(entries[0].menu_id, entries[1].menu_id);
    }

    #[test]
    fn combined_status_uses_the_same_honest_priority_as_the_main_ui() {
        let cases = [
            (
                "activity_detected",
                "failed_or_interrupted",
                TrayActivityStatus::Activity,
            ),
            (
                "turn_finished",
                "failed_or_interrupted",
                TrayActivityStatus::Failure,
            ),
            ("unknown", "turn_finished", TrayActivityStatus::Finished),
            ("unknown", "unsupported", TrayActivityStatus::Unknown),
        ];
        for (codex, claude, expected) in cases {
            assert_eq!(
                aggregate_status(
                    &activity(codex, ""),
                    &activity(claude, ""),
                    None,
                    None,
                    None,
                ),
                expected
            );
        }

        assert_eq!(
            aggregate_status(
                &activity("turn_finished", ""),
                &activity("unknown", ""),
                None,
                Some(&activity("activity_detected", "")),
                None,
            ),
            TrayActivityStatus::Activity,
        );

        assert_eq!(
            aggregate_status(
                &activity("unknown", ""),
                &activity("unknown", ""),
                None,
                None,
                Some(&activity("recent_activity", "")),
            ),
            TrayActivityStatus::Recent,
        );
    }

    #[test]
    fn tray_labels_are_bounded_safe_and_contain_no_private_metadata() {
        let hostile_name = format!("  repo&name\n{}  ", "x".repeat(80));
        let entries = tray_workspace_entries(&snapshot(vec![workspace(
            "opaque-id",
            &hostile_name,
            true,
            false,
            true,
            "activity_detected",
            "unknown",
        )]));
        let label = entries[0].menu_label();

        assert!(label.contains("repo&&name"));
        assert!(label.contains("Activity detected"));
        assert!(!label.contains('\n'));
        assert!(!label.contains("/private/project-path"));
        assert!(!label.contains("PRIVATE"));
        assert!(entries[0].name.chars().count() <= MAX_WORKSPACE_NAME_CHARS);
    }

    #[test]
    fn tray_labels_identify_antigravity_ide_and_include_its_activity() {
        let mut antigravity_workspace = workspace(
            "antigravity-window",
            "shared-project",
            true,
            true,
            true,
            "unknown",
            "unknown",
        );
        antigravity_workspace.editor = crate::opener::EditorKind::AntigravityIde;
        antigravity_workspace.editor_name = "Antigravity IDE".to_string();
        antigravity_workspace.antigravity =
            Some(activity("activity_detected", "PRIVATE ANTIGRAVITY DETAIL"));

        let entries = tray_workspace_entries(&snapshot(vec![antigravity_workspace]));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, TrayActivityStatus::Activity);
        let label = entries[0].menu_label();
        assert!(label.contains("Antigravity IDE"));
        assert!(label.contains("Activity detected"));
        assert!(!label.contains("PRIVATE"));
    }

    #[test]
    fn menu_events_use_an_opaque_id_map_instead_of_parsing_labels() {
        let first_id = stable_workspace_menu_id("instance-a");
        let second_id = stable_workspace_menu_id("instance-b");
        let workspace_ids = HashMap::from([
            (first_id, "instance-a".to_string()),
            (second_id.clone(), "instance-b".to_string()),
        ]);
        assert_eq!(
            resolve_menu_action(&second_id, &workspace_ids),
            TrayMenuAction::OpenWorkspace("instance-b".to_string())
        );
        assert_eq!(
            resolve_menu_action(TRAY_OPEN_ID, &workspace_ids),
            TrayMenuAction::OpenMain
        );
        assert_eq!(
            resolve_menu_action(TRAY_QUIT_ID, &workspace_ids),
            TrayMenuAction::Quit
        );
        assert_eq!(
            resolve_menu_action("example-workspace", &workspace_ids),
            TrayMenuAction::None
        );
    }

    #[test]
    fn refresh_failure_preserves_last_good_entries_and_success_clears_the_warning() {
        let entries = tray_workspace_entries(&snapshot(vec![workspace(
            "current", "current", true, false, true, "unknown", "unknown",
        )]));
        let failed = failed_refresh_model(entries.clone());
        assert!(failed.refresh_failed);
        assert_eq!(failed.entries, entries);
        assert!(!entry_is_enabled(&failed, &failed.entries[0]));

        let recovered = successful_refresh_model(failed.entries);
        assert!(!recovered.refresh_failed);
        assert_eq!(recovered.entries.len(), 1);
        assert!(entry_is_enabled(&recovered, &recovered.entries[0]));
    }
}
