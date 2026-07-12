use super::*;

#[derive(Debug, Error)]
pub enum AppLogError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("app log lock is poisoned")]
    PoisonedLock,
}

#[derive(Clone)]
pub struct AppLogger {
    path: PathBuf,
    file: Arc<Mutex<File>>,
    max_bytes: u64,
    max_generations: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLogRecord {
    pub id: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    pub payload: JsonMap,
}

impl AppLogger {
    const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024;
    const DEFAULT_MAX_GENERATIONS: usize = 3;

    pub fn open(path: impl AsRef<Path>) -> std::result::Result<Self, AppLogError> {
        Self::open_with_rotation(path, Self::DEFAULT_MAX_BYTES, Self::DEFAULT_MAX_GENERATIONS)
    }

    fn open_with_rotation(
        path: impl AsRef<Path>,
        max_bytes: u64,
        max_generations: usize,
    ) -> std::result::Result<Self, AppLogError> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            file: Arc::new(Mutex::new(file)),
            max_bytes,
            max_generations,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(
        &self,
        kind: impl Into<String>,
        source: impl Into<String>,
        payload: JsonMap,
    ) -> std::result::Result<AppLogRecord, AppLogError> {
        let record = AppLogRecord {
            id: new_id("app"),
            timestamp: now_timestamp(),
            kind: kind.into(),
            source: source.into(),
            payload,
        };
        let mut file = self.file.lock().map_err(|_| AppLogError::PoisonedLock)?;
        if let Err(error) = self.rotate_if_needed(&mut file) {
            eprintln!(
                "failed to rotate app activity log {}: {error}",
                self.path.display()
            );
        }
        serde_json::to_writer(&mut *file, &record)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(record)
    }

    fn rotate_if_needed(&self, file: &mut File) -> std::result::Result<(), AppLogError> {
        if self.max_generations == 0 {
            return Ok(());
        }
        let size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        if size <= self.max_bytes {
            return Ok(());
        }
        file.flush()?;
        let oldest = rotated_app_log_path(&self.path, self.max_generations);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for generation in (1..self.max_generations).rev() {
            let from = rotated_app_log_path(&self.path, generation);
            if from.exists() {
                fs::rename(&from, rotated_app_log_path(&self.path, generation + 1))?;
            }
        }
        if self.path.exists() {
            fs::rename(&self.path, rotated_app_log_path(&self.path, 1))?;
        }
        *file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        Ok(())
    }
}

fn rotated_app_log_path(path: &Path, generation: usize) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("app-activity.jsonl");
    path.with_file_name(format!(
        "{stem}.{generation}.jsonl",
        stem = file_name.trim_end_matches(".jsonl")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn app_logger_writes_jsonl_records() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let path = dir.path().join("app-activity.jsonl");
        let logger = AppLogger::open(&path)?;

        logger.record(
            "test.event",
            "test",
            JsonMap::from([("ok".to_string(), json!(true))]),
        )?;

        let raw = fs::read_to_string(&path)?;
        assert!(raw.contains("\"type\":\"test.event\""));
        assert!(raw.contains("\"ok\":true"));
        assert_eq!(logger.path(), path.as_path());
        Ok(())
    }

    #[test]
    fn app_logger_rotates_after_size_limit_and_keeps_three_generations(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let path = dir.path().join("app-activity.jsonl");
        let logger = AppLogger::open_with_rotation(&path, 180, 3)?;

        for index in 0..12 {
            logger.record(
                "test.event",
                "test",
                JsonMap::from([
                    ("index".to_string(), json!(index)),
                    ("padding".to_string(), json!("xxxxxxxxxxxxxxxxxxxxxxxx")),
                ]),
            )?;
        }

        assert!(path.exists());
        assert!(dir.path().join("app-activity.1.jsonl").exists());
        assert!(dir.path().join("app-activity.2.jsonl").exists());
        assert!(dir.path().join("app-activity.3.jsonl").exists());
        assert!(!dir.path().join("app-activity.4.jsonl").exists());
        Ok(())
    }
}
