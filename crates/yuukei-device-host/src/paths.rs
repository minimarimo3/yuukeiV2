use super::*;

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("yuukei-device-host is nested under crates")
        .to_path_buf()
}

pub(crate) fn default_data_dir() -> PathBuf {
    resolve_default_data_dir(
        std::env::var_os("YUUKEI_DATA_DIR"),
        dirs::data_dir(),
        std::env::temp_dir().join("yuukei-v2"),
    )
}

pub(crate) fn permanent_data_dir(base: PathBuf) -> PathBuf {
    base.join("Yuukei").join("v2")
}

pub(crate) fn resolve_default_data_dir(
    env_data_dir: Option<std::ffi::OsString>,
    os_data_dir: Option<PathBuf>,
    legacy_temp_dir: PathBuf,
) -> PathBuf {
    if let Some(path) = env_data_dir {
        return PathBuf::from(path);
    }
    let Some(base) = os_data_dir else {
        return legacy_temp_dir;
    };
    let data_dir = permanent_data_dir(base);
    migrate_legacy_temp_data_dir(&legacy_temp_dir, &data_dir).unwrap_or(legacy_temp_dir)
}

pub(crate) fn migrate_legacy_temp_data_dir(legacy_temp_dir: &Path, data_dir: &Path) -> io::Result<PathBuf> {
    if !directory_has_entries(legacy_temp_dir)? || !directory_is_absent_or_empty(data_dir)? {
        return Ok(data_dir.to_path_buf());
    }

    if let Some(parent) = data_dir.parent() {
        fs::create_dir_all(parent)?;
    }

    if data_dir.exists() {
        move_directory_contents(legacy_temp_dir, data_dir)?;
        let _ = fs::remove_dir(legacy_temp_dir);
    } else {
        fs::rename(legacy_temp_dir, data_dir)?;
    }

    record_data_dir_migration(data_dir, legacy_temp_dir, data_dir);
    Ok(data_dir.to_path_buf())
}

fn directory_has_entries(path: &Path) -> io::Result<bool> {
    match fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn directory_is_absent_or_empty(path: &Path) -> io::Result<bool> {
    match fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn move_directory_contents(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        fs::rename(entry.path(), to.join(entry.file_name()))?;
    }
    Ok(())
}

fn record_data_dir_migration(data_dir: &Path, from: &Path, to: &Path) {
    let logger = match AppLogger::open(data_dir.join("app-activity.jsonl")) {
        Ok(logger) => logger,
        Err(error) => {
            eprintln!(
                "failed to open app activity log after data dir migration {}: {error}",
                data_dir.display()
            );
            return;
        }
    };
    let _ = logger.record(
        "data_dir.migrated",
        "device-host",
        JsonMap::from([
            ("from".to_string(), json!(display_path(from))),
            ("to".to_string(), json!(display_path(to))),
        ]),
    );
}
pub(crate) fn display_path(path: impl AsRef<Path>) -> String {
    path.as_ref().display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_data_dir_prefers_env_and_uses_os_data_dir(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let env_data_dir = dir.path().join("env-data");
        let os_data_home = dir.path().join("os-data-home");
        let legacy_temp_dir = dir.path().join("legacy-temp");
        fs::create_dir_all(&legacy_temp_dir)?;
        fs::write(legacy_temp_dir.join("events.sqlite3"), b"legacy")?;

        let env_selected = resolve_default_data_dir(
            Some(env_data_dir.clone().into_os_string()),
            Some(os_data_home.clone()),
            legacy_temp_dir.clone(),
        );
        assert_eq!(env_selected, env_data_dir);
        assert!(legacy_temp_dir.join("events.sqlite3").exists());

        let selected =
            resolve_default_data_dir(None, Some(os_data_home.clone()), dir.path().join("missing"));
        assert_eq!(selected, os_data_home.join("Yuukei").join("v2"));
        Ok(())
    }

    #[test]
    fn data_dir_migration_moves_legacy_contents_and_records_app_log(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let legacy = dir.path().join("yuukei-v2");
        let data_dir = dir
            .path()
            .join("Application Support")
            .join("Yuukei")
            .join("v2");
        fs::create_dir_all(legacy.join("residents").join("default"))?;
        fs::create_dir_all(&data_dir)?;
        fs::write(
            legacy
                .join("residents")
                .join("default")
                .join("events.sqlite3"),
            b"events",
        )?;

        let selected = migrate_legacy_temp_data_dir(&legacy, &data_dir)?;

        assert_eq!(selected, data_dir);
        assert!(data_dir
            .join("residents")
            .join("default")
            .join("events.sqlite3")
            .exists());
        let app_log = fs::read_to_string(data_dir.join("app-activity.jsonl"))?;
        assert!(app_log.contains("\"type\":\"data_dir.migrated\""));
        assert!(app_log.contains("\"from\""));
        assert!(app_log.contains("\"to\""));
        Ok(())
    }

    #[test]
    fn data_dir_migration_uses_new_directory_when_both_have_data(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let legacy = dir.path().join("yuukei-v2");
        let data_dir = dir.path().join("data").join("Yuukei").join("v2");
        fs::create_dir_all(&legacy)?;
        fs::create_dir_all(&data_dir)?;
        fs::write(legacy.join("legacy.txt"), b"legacy")?;
        fs::write(data_dir.join("current.txt"), b"current")?;

        let selected = migrate_legacy_temp_data_dir(&legacy, &data_dir)?;

        assert_eq!(selected, data_dir);
        assert!(legacy.join("legacy.txt").exists());
        assert!(data_dir.join("current.txt").exists());
        assert!(!data_dir.join("legacy.txt").exists());
        Ok(())
    }

    #[test]
    fn data_dir_migration_failure_keeps_legacy_directory(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let legacy = dir.path().join("yuukei-v2");
        let blocked_data_home = dir.path().join("blocked");
        fs::create_dir_all(&legacy)?;
        fs::write(legacy.join("events.sqlite3"), b"legacy")?;
        fs::write(&blocked_data_home, b"not a directory")?;

        let selected = resolve_default_data_dir(None, Some(blocked_data_home), legacy.clone());

        assert_eq!(selected, legacy);
        assert!(selected.join("events.sqlite3").exists());
        Ok(())
    }

}
