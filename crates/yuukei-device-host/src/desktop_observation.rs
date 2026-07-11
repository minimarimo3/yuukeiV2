use super::*;

const DESKTOP_WINDOW_FOCUS_DEBOUNCE: Duration = Duration::from_secs(1);
pub(crate) const DESKTOP_OBSERVATION_PRIVACY_CATEGORY: &str = "desktop-observation";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopWindowFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopWindowObservation {
    pub window_key: String,
    pub app: String,
    pub frame: DesktopWindowFrame,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopWindowTransitionKind {
    Appeared,
    Closed,
    Focused,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopWindowTransition {
    pub kind: DesktopWindowTransitionKind,
    pub window_key: String,
    pub app: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DesktopFolderCategory {
    Downloads,
    Desktop,
    Documents,
    Pictures,
    Trash,
    Other,
}

impl DesktopFolderCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Downloads => "downloads",
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Pictures => "pictures",
            Self::Trash => "trash",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KnownDesktopFolders {
    pub downloads: Option<String>,
    pub desktop: Option<String>,
    pub documents: Option<String>,
    pub pictures: Option<String>,
    pub trash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopFolderObservation {
    pub folder_key: String,
    pub category: DesktopFolderCategory,
    pub app: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopFolderTransition {
    pub category: DesktopFolderCategory,
    pub app: String,
}

#[derive(Clone, Debug, Default)]
pub struct DesktopFolderObservationState {
    folders: BTreeMap<String, DesktopFolderSeen>,
}

#[derive(Clone, Debug)]
struct DesktopFolderSeen {
    category: DesktopFolderCategory,
    last_emitted_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadFileCategory {
    Image,
    Video,
    Audio,
    Document,
    Archive,
    App,
    Other,
}

impl DownloadFileCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::Archive => "archive",
            Self::App => "app",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopDownloadObservation {
    pub file_name: String,
    pub file_category: DownloadFileCategory,
}

#[derive(Clone, Debug, Default)]
pub struct DesktopWindowObservationState {
    windows: BTreeMap<String, DesktopWindowObservation>,
    focused_key: Option<String>,
    pending_focus: Option<PendingDesktopWindowFocus>,
}

#[derive(Clone, Debug)]
struct PendingDesktopWindowFocus {
    window_key: String,
    since: DateTime<Utc>,
}

impl DesktopWindowObservationState {
    pub fn update(
        &mut self,
        now: DateTime<Utc>,
        observations: Vec<DesktopWindowObservation>,
    ) -> Vec<DesktopWindowTransition> {
        let next = observations
            .into_iter()
            .map(|observation| (observation.window_key.clone(), observation))
            .collect::<BTreeMap<_, _>>();
        let mut transitions = Vec::new();

        for (window_key, observation) in &next {
            if !self.windows.contains_key(window_key) {
                transitions.push(DesktopWindowTransition {
                    kind: DesktopWindowTransitionKind::Appeared,
                    window_key: window_key.clone(),
                    app: observation.app.clone(),
                });
            }
        }

        for (window_key, previous) in &self.windows {
            if !next.contains_key(window_key) {
                transitions.push(DesktopWindowTransition {
                    kind: DesktopWindowTransitionKind::Closed,
                    window_key: window_key.clone(),
                    app: previous.app.clone(),
                });
            }
        }

        let current_focus = next
            .values()
            .find(|observation| observation.focused)
            .map(|observation| observation.window_key.clone());
        self.windows = next;
        self.retain_focus_after_closures();

        if let Some(focused) = self.evaluate_focused_transition(now, current_focus) {
            transitions.push(focused);
        }

        transitions
    }

    fn retain_focus_after_closures(&mut self) {
        if self
            .focused_key
            .as_ref()
            .is_some_and(|key| !self.windows.contains_key(key))
        {
            self.focused_key = None;
        }
        if self
            .pending_focus
            .as_ref()
            .is_some_and(|pending| !self.windows.contains_key(&pending.window_key))
        {
            self.pending_focus = None;
        }
    }

    fn evaluate_focused_transition(
        &mut self,
        now: DateTime<Utc>,
        current_focus: Option<String>,
    ) -> Option<DesktopWindowTransition> {
        let Some(current_focus) = current_focus else {
            self.pending_focus = None;
            return None;
        };
        if self.focused_key.as_deref() == Some(current_focus.as_str()) {
            self.pending_focus = None;
            return None;
        }
        match &mut self.pending_focus {
            Some(pending) if pending.window_key == current_focus => {
                let elapsed = now
                    .signed_duration_since(pending.since)
                    .to_std()
                    .unwrap_or_default();
                if elapsed < DESKTOP_WINDOW_FOCUS_DEBOUNCE {
                    return None;
                }
            }
            _ => {
                self.pending_focus = Some(PendingDesktopWindowFocus {
                    window_key: current_focus,
                    since: now,
                });
                return None;
            }
        }
        let window_key = self
            .pending_focus
            .take()
            .map(|pending| pending.window_key)
            .unwrap_or_default();
        let app = self
            .windows
            .get(&window_key)
            .map(|window| window.app.clone())
            .unwrap_or_default();
        self.focused_key = Some(window_key.clone());
        Some(DesktopWindowTransition {
            kind: DesktopWindowTransitionKind::Focused,
            window_key,
            app,
        })
    }
}

impl DesktopFolderObservationState {
    pub fn update(
        &mut self,
        now: DateTime<Utc>,
        observations: Vec<DesktopFolderObservation>,
    ) -> Vec<DesktopFolderTransition> {
        let mut transitions = Vec::new();
        let current_keys = observations
            .iter()
            .map(|observation| observation.folder_key.clone())
            .collect::<Vec<_>>();
        for observation in observations {
            let emit = match self.folders.get(&observation.folder_key) {
                Some(seen) if seen.category == observation.category => now
                    .signed_duration_since(seen.last_emitted_at)
                    .to_std()
                    .map(|elapsed| elapsed >= Duration::from_secs(60))
                    .unwrap_or(false),
                Some(_) | None => true,
            };
            if emit {
                transitions.push(DesktopFolderTransition {
                    category: observation.category,
                    app: observation.app.clone(),
                });
                self.folders.insert(
                    observation.folder_key,
                    DesktopFolderSeen {
                        category: observation.category,
                        last_emitted_at: now,
                    },
                );
            }
        }
        self.folders
            .retain(|folder_key, _| current_keys.contains(folder_key));
        transitions
    }
}

impl DesktopWindowTransition {
    pub fn signal(&self) -> &'static str {
        match self.kind {
            DesktopWindowTransitionKind::Appeared => "desktop.window.appeared",
            DesktopWindowTransitionKind::Closed => "desktop.window.closed",
            DesktopWindowTransitionKind::Focused => "desktop.window.focused",
        }
    }

    pub(crate) fn payload(&self) -> JsonMap {
        JsonMap::from([
            ("windowKey".to_string(), json!(self.window_key)),
            ("app".to_string(), json!(self.app)),
        ])
    }
}

impl DesktopFolderTransition {
    pub fn signal(&self) -> &'static str {
        "desktop.folder.opened"
    }

    pub(crate) fn payload(&self) -> JsonMap {
        JsonMap::from([
            ("category".to_string(), json!(self.category.as_str())),
            ("app".to_string(), json!(self.app)),
        ])
    }
}

impl DesktopDownloadObservation {
    pub fn signal(&self) -> &'static str {
        "desktop.download.completed"
    }

    pub(crate) fn payload(&self) -> JsonMap {
        JsonMap::from([
            ("fileName".to_string(), json!(self.file_name)),
            (
                "fileCategory".to_string(),
                json!(self.file_category.as_str()),
            ),
        ])
    }
}

pub fn categorize_desktop_folder_path(
    path: &str,
    known: &KnownDesktopFolders,
) -> DesktopFolderCategory {
    if is_windows_trash_shell_path(path) {
        return DesktopFolderCategory::Trash;
    }
    let normalized = normalize_observed_path(path);
    for (category, candidate) in [
        (DesktopFolderCategory::Downloads, known.downloads.as_deref()),
        (DesktopFolderCategory::Desktop, known.desktop.as_deref()),
        (DesktopFolderCategory::Documents, known.documents.as_deref()),
        (DesktopFolderCategory::Pictures, known.pictures.as_deref()),
        (DesktopFolderCategory::Trash, known.trash.as_deref()),
    ] {
        if candidate
            .map(normalize_observed_path)
            .is_some_and(|candidate| candidate == normalized)
        {
            return category;
        }
    }
    DesktopFolderCategory::Other
}

pub fn classify_download_file_name(file_name: &str) -> DownloadFileCategory {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "bmp" | "tiff" | "svg" => {
            DownloadFileCategory::Image
        }
        "mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" | "wmv" => DownloadFileCategory::Video,
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus" => DownloadFileCategory::Audio,
        "pdf" | "txt" | "md" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "csv" | "rtf" => {
            DownloadFileCategory::Document
        }
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" => DownloadFileCategory::Archive,
        "dmg" | "pkg" | "app" | "exe" | "msi" | "deb" | "rpm" | "apk" => DownloadFileCategory::App,
        _ => DownloadFileCategory::Other,
    }
}

pub fn download_file_observation_from_name(file_name: &str) -> Option<DesktopDownloadObservation> {
    if should_ignore_download_file_name(file_name) {
        return None;
    }
    Some(DesktopDownloadObservation {
        file_name: file_name.to_string(),
        file_category: classify_download_file_name(file_name),
    })
}

pub fn should_ignore_download_file_name(file_name: &str) -> bool {
    if file_name.is_empty() || file_name.starts_with('.') {
        return true;
    }
    let lower = file_name.to_ascii_lowercase();
    [".crdownload", ".part", ".download", ".tmp", ".aria2"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn normalize_observed_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized.to_ascii_lowercase()
}

fn is_windows_trash_shell_path(path: &str) -> bool {
    path.to_ascii_uppercase()
        .contains("645FF040-5081-101B-9F08-00AA002F954E")
}

pub(crate) fn desktop_observation_privacy() -> Privacy {
    Privacy {
        category: DESKTOP_OBSERVATION_PRIVACY_CATEGORY.to_string(),
        retention: RetentionPolicy::Short,
        extension_readable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_window_observation_emits_appeared_closed_and_debounced_focus() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = DesktopWindowObservationState::default();

        let first = state.update(now, vec![window_observation("a", "Finder", true)]);
        assert_eq!(
            first,
            vec![DesktopWindowTransition {
                kind: DesktopWindowTransitionKind::Appeared,
                window_key: "a".to_string(),
                app: "Finder".to_string(),
            }]
        );

        assert!(state
            .update(
                now + chrono::Duration::milliseconds(500),
                vec![window_observation("a", "Finder", true)]
            )
            .is_empty());

        let focused = state.update(
            now + chrono::Duration::milliseconds(1000),
            vec![window_observation("a", "Finder", true)],
        );
        assert_eq!(
            focused,
            vec![DesktopWindowTransition {
                kind: DesktopWindowTransitionKind::Focused,
                window_key: "a".to_string(),
                app: "Finder".to_string(),
            }]
        );

        let second = state.update(
            now + chrono::Duration::milliseconds(1500),
            vec![
                window_observation("a", "Finder", false),
                window_observation("b", "Safari", true),
            ],
        );
        assert_eq!(
            second,
            vec![DesktopWindowTransition {
                kind: DesktopWindowTransitionKind::Appeared,
                window_key: "b".to_string(),
                app: "Safari".to_string(),
            }]
        );

        let changed_focus = state.update(
            now + chrono::Duration::milliseconds(2600),
            vec![
                window_observation("a", "Finder", false),
                window_observation("b", "Safari", true),
            ],
        );
        assert_eq!(
            changed_focus,
            vec![DesktopWindowTransition {
                kind: DesktopWindowTransitionKind::Focused,
                window_key: "b".to_string(),
                app: "Safari".to_string(),
            }]
        );

        let closed = state.update(
            now + chrono::Duration::milliseconds(3000),
            vec![window_observation("b", "Safari", true)],
        );
        assert_eq!(
            closed,
            vec![DesktopWindowTransition {
                kind: DesktopWindowTransitionKind::Closed,
                window_key: "a".to_string(),
                app: "Finder".to_string(),
            }]
        );
    }

    #[test]
    fn desktop_window_focus_candidate_resets_before_debounce() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = DesktopWindowObservationState::default();

        state.update(now, vec![window_observation("a", "Finder", true)]);
        state.update(
            now + chrono::Duration::milliseconds(500),
            vec![
                window_observation("a", "Finder", false),
                window_observation("b", "Safari", true),
            ],
        );
        let transitions = state.update(
            now + chrono::Duration::milliseconds(900),
            vec![
                window_observation("a", "Finder", true),
                window_observation("b", "Safari", false),
            ],
        );

        assert!(transitions.is_empty());
    }

    #[test]
    fn desktop_folder_paths_are_normalized_to_known_categories() {
        let known = KnownDesktopFolders {
            downloads: Some("/Users/example/Downloads".to_string()),
            desktop: Some("/Users/example/Desktop".to_string()),
            documents: Some("/Users/example/Documents".to_string()),
            pictures: Some("/Users/example/Pictures".to_string()),
            trash: Some("/Users/example/.Trash".to_string()),
        };

        assert_eq!(
            categorize_desktop_folder_path("/Users/example/Downloads/", &known),
            DesktopFolderCategory::Downloads
        );
        assert_eq!(
            categorize_desktop_folder_path("\\Users\\example\\Documents", &known),
            DesktopFolderCategory::Documents
        );
        assert_eq!(
            categorize_desktop_folder_path("::{645FF040-5081-101B-9F08-00AA002F954E}", &known),
            DesktopFolderCategory::Trash
        );
        assert_eq!(
            categorize_desktop_folder_path("/Users/example/Projects", &known),
            DesktopFolderCategory::Other
        );
    }

    #[test]
    fn desktop_folder_observation_debounces_same_category_for_one_minute() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = DesktopFolderObservationState::default();

        assert_eq!(
            state.update(
                now,
                vec![folder_observation(
                    "front",
                    DesktopFolderCategory::Downloads,
                    "finder"
                )]
            ),
            vec![DesktopFolderTransition {
                category: DesktopFolderCategory::Downloads,
                app: "finder".to_string(),
            }]
        );
        assert!(state
            .update(
                now + chrono::Duration::seconds(30),
                vec![folder_observation(
                    "front",
                    DesktopFolderCategory::Downloads,
                    "finder"
                )]
            )
            .is_empty());
        assert_eq!(
            state.update(
                now + chrono::Duration::seconds(31),
                vec![folder_observation(
                    "front",
                    DesktopFolderCategory::Documents,
                    "finder"
                )]
            ),
            vec![DesktopFolderTransition {
                category: DesktopFolderCategory::Documents,
                app: "finder".to_string(),
            }]
        );
        assert_eq!(
            state.update(
                now + chrono::Duration::seconds(91),
                vec![folder_observation(
                    "front",
                    DesktopFolderCategory::Documents,
                    "finder"
                )]
            ),
            vec![DesktopFolderTransition {
                category: DesktopFolderCategory::Documents,
                app: "finder".to_string(),
            }]
        );
    }

    #[test]
    fn download_file_names_classify_and_filter_private_or_temporary_files() {
        assert_eq!(
            classify_download_file_name("photo.HEIC"),
            DownloadFileCategory::Image
        );
        assert_eq!(
            classify_download_file_name("movie.webm"),
            DownloadFileCategory::Video
        );
        assert_eq!(
            classify_download_file_name("song.flac"),
            DownloadFileCategory::Audio
        );
        assert_eq!(
            classify_download_file_name("report.pdf"),
            DownloadFileCategory::Document
        );
        assert_eq!(
            classify_download_file_name("bundle.tar.gz"),
            DownloadFileCategory::Archive
        );
        assert_eq!(
            classify_download_file_name("installer.msi"),
            DownloadFileCategory::App
        );
        assert_eq!(
            classify_download_file_name("unknown.asset"),
            DownloadFileCategory::Other
        );

        assert!(download_file_observation_from_name(".secret.pdf").is_none());
        assert!(download_file_observation_from_name("movie.mp4.crdownload").is_none());
        assert!(download_file_observation_from_name("archive.zip.part").is_none());
        assert!(download_file_observation_from_name("file.download").is_none());
        assert!(download_file_observation_from_name("sync.tmp").is_none());
        assert!(download_file_observation_from_name("aria.aria2").is_none());
        assert_eq!(
            download_file_observation_from_name("report.pdf"),
            Some(DesktopDownloadObservation {
                file_name: "report.pdf".to_string(),
                file_category: DownloadFileCategory::Document,
            })
        );
    }

    fn window_observation(window_key: &str, app: &str, focused: bool) -> DesktopWindowObservation {
        DesktopWindowObservation {
            window_key: window_key.to_string(),
            app: app.to_string(),
            focused,
            frame: DesktopWindowFrame {
                x: 10.0,
                y: 20.0,
                width: 640.0,
                height: 480.0,
            },
        }
    }

    fn folder_observation(
        folder_key: &str,
        category: DesktopFolderCategory,
        app: &str,
    ) -> DesktopFolderObservation {
        DesktopFolderObservation {
            folder_key: folder_key.to_string(),
            category,
            app: app.to_string(),
        }
    }

}
