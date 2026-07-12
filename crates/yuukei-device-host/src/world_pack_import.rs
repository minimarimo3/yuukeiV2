use super::*;

const WORLD_PACK_IMPORT_MAX_BYTES: u64 = 500 * 1024 * 1024;
pub(crate) const WORLD_PACK_IMPORT_MAX_ENTRIES: usize = 5000;
const WORLD_PACK_LICENSE_TEXT_LIMIT: usize = 4000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPackZipInspection {
    pub pack_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_source: Option<String>,
    pub imported_root: PathBuf,
    pub replaces_existing: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct InspectedWorldPackZip {
    root_prefix: Option<PathBuf>,
    pack_id: String,
    display_name: String,
    license_text: Option<String>,
    license_source: Option<String>,
    imported_root: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LicenseCandidate {
    license_index: Option<usize>,
    license_txt_index: Option<usize>,
    readme_index: Option<usize>,
}

impl LicenseCandidate {
    pub(crate) fn set_file(&mut self, name: &str, index: usize) {
        if name == "LICENSE" {
            self.license_index.get_or_insert(index);
        } else if name == "LICENSE.txt" {
            self.license_txt_index.get_or_insert(index);
        } else if name == "README.md" {
            self.readme_index.get_or_insert(index);
        }
    }

    pub(crate) fn selected(&self) -> Option<(usize, &'static str)> {
        self.license_index
            .map(|index| (index, "LICENSE"))
            .or_else(|| self.license_txt_index.map(|index| (index, "LICENSE.txt")))
            .or_else(|| self.readme_index.map(|index| (index, "README.md")))
    }
}

pub(crate) fn inspect_world_pack_zip_at(
    data_dir: &Path,
    path: impl AsRef<Path>,
) -> Result<WorldPackZipInspection> {
    let zip_path = path.as_ref();
    let file = File::open(zip_path).map_err(|error| {
        DeviceHostError::WorldPackImport(format!("zipファイルを開けませんでした: {}", error))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        DeviceHostError::WorldPackImport(format!("zipファイルを読み込めませんでした: {}", error))
    })?;
    let inspected = inspect_world_pack_zip_archive(data_dir, &mut archive)?;
    Ok(WorldPackZipInspection {
        pack_id: inspected.pack_id,
        display_name: inspected.display_name,
        license_text: inspected.license_text,
        license_source: inspected.license_source,
        replaces_existing: inspected.imported_root.exists(),
        imported_root: inspected.imported_root,
    })
}

pub(crate) fn inspect_world_pack_zip_archive<R: Read + Seek>(
    data_dir: &Path,
    archive: &mut zip::ZipArchive<R>,
) -> Result<InspectedWorldPackZip> {
    if archive.len() > WORLD_PACK_IMPORT_MAX_ENTRIES {
        return Err(DeviceHostError::WorldPackImport(format!(
            "zip内のファイル数が多すぎます。上限は{}件です。",
            WORLD_PACK_IMPORT_MAX_ENTRIES
        )));
    }

    let mut total_size = 0_u64;
    let mut safe_paths: Vec<(usize, PathBuf)> = Vec::new();
    let mut pack_json_candidates: Vec<(usize, PathBuf)> = Vec::new();

    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(world_pack_zip_error)?;
        if file.is_symlink() {
            return Err(DeviceHostError::WorldPackImport(format!(
                "zip内にシンボリックリンクがあります: {}",
                file.name()
            )));
        }
        let Some(enclosed_name) = file.enclosed_name() else {
            return Err(DeviceHostError::WorldPackImport(format!(
                "zip内に安全でないパスがあります: {}",
                file.name()
            )));
        };
        total_size = total_size.saturating_add(file.size());
        if total_size > WORLD_PACK_IMPORT_MAX_BYTES {
            return Err(DeviceHostError::WorldPackImport(
                "World Packが大きすぎます。展開後の合計サイズは500MBまでです。".to_string(),
            ));
        }
        if file.is_dir() {
            safe_paths.push((index, enclosed_name));
            continue;
        }
        if enclosed_name.file_name().and_then(|name| name.to_str()) == Some("pack.json") {
            let depth = enclosed_name.components().count();
            if depth == 1 || depth == 2 {
                pack_json_candidates.push((index, enclosed_name.clone()));
            }
        }
        safe_paths.push((index, enclosed_name));
    }

    let (pack_json_index, pack_json_path) = match pack_json_candidates.as_slice() {
        [(index, path)] => (*index, path.clone()),
        [] => {
            return Err(DeviceHostError::WorldPackImport(
                "pack.jsonが見つかりません。zipのルート直下、または単一のトップディレクトリ内に置いてください。"
                    .to_string(),
            ))
        }
        _ => {
            return Err(DeviceHostError::WorldPackImport(
                "pack.jsonが複数見つかりました。World Packは1つだけ含めてください。".to_string(),
            ))
        }
    };
    let root_prefix = pack_root_prefix(&pack_json_path);

    let mut license_candidates = LicenseCandidate::default();
    for (index, path) in &safe_paths {
        let Some(relative_path) = strip_pack_root(path, root_prefix.as_deref()) else {
            return Err(DeviceHostError::WorldPackImport(
                "pack.jsonの外に別のファイルがあります。zipにはWorld Pack本体だけを入れてください。"
                    .to_string(),
            ));
        };
        if relative_path.components().count() == 1 {
            if let Some(name) = relative_path.file_name().and_then(|name| name.to_str()) {
                license_candidates.set_file(name, *index);
            }
        }
    }

    let pack_json_text = read_zip_text(archive, pack_json_index, None)?;
    let pack_json: Value = serde_json::from_str(&pack_json_text).map_err(|error| {
        DeviceHostError::WorldPackImport(format!("pack.jsonが壊れています: {}", error))
    })?;
    let pack_id = pack_json
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| DeviceHostError::WorldPackImport("pack.jsonのidが空です。".to_string()))?
        .to_string();
    let display_name = pack_json
        .get("displayName")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&pack_id)
        .to_string();
    let mut license_text = None;
    let mut license_source = None;
    if let Some((index, source)) = license_candidates.selected() {
        license_text = Some(read_zip_text(
            archive,
            index,
            Some(WORLD_PACK_LICENSE_TEXT_LIMIT as u64),
        )?);
        license_source = Some(source.to_string());
    } else if let Some(license) = pack_json.get("license").and_then(Value::as_str) {
        let trimmed = license.trim();
        if !trimmed.is_empty() {
            license_text = Some(truncate_text(trimmed, WORLD_PACK_LICENSE_TEXT_LIMIT));
            license_source = Some("pack.json license".to_string());
        }
    }

    Ok(InspectedWorldPackZip {
        root_prefix,
        pack_id: pack_id.clone(),
        display_name,
        license_text,
        license_source,
        imported_root: data_dir
            .join("packs-imported")
            .join(imported_pack_dir_name(&pack_id)),
    })
}

pub(crate) fn import_world_pack_zip_to_dir(
    data_dir: &Path,
    zip_path: impl AsRef<Path>,
) -> Result<PathBuf> {
    let zip_path = zip_path.as_ref();
    let file = File::open(zip_path).map_err(|error| {
        DeviceHostError::WorldPackImport(format!("zipファイルを開けませんでした: {}", error))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        DeviceHostError::WorldPackImport(format!("zipファイルを読み込めませんでした: {}", error))
    })?;
    let inspected = inspect_world_pack_zip_archive(data_dir, &mut archive)?;
    let imported_parent = data_dir.join("packs-imported");
    fs::create_dir_all(&imported_parent)?;
    let staging_root = imported_parent.join(format!(
        ".importing-{}-{}",
        imported_pack_dir_name(&inspected.pack_id),
        now_timestamp()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
    ));
    if staging_root.exists() {
        fs::remove_dir_all(&staging_root)?;
    }
    fs::create_dir_all(&staging_root)?;

    let extract_result = extract_world_pack_zip_archive(&mut archive, &inspected, &staging_root)
        .and_then(|_| {
            WorldPack::load_from_dir(&staging_root).map_err(|error| {
                DeviceHostError::WorldPackImport(format!(
                    "World Packの検証に失敗しました: {}",
                    error
                ))
            })
        });
    if let Err(error) = extract_result {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(error);
    }

    if inspected.imported_root.exists() {
        fs::remove_dir_all(&inspected.imported_root)?;
    }
    fs::rename(&staging_root, &inspected.imported_root)?;
    Ok(inspected.imported_root)
}

pub(crate) fn extract_world_pack_zip_archive<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    inspected: &InspectedWorldPackZip,
    destination: &Path,
) -> Result<()> {
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(world_pack_zip_error)?;
        if file.is_symlink() {
            return Err(DeviceHostError::WorldPackImport(format!(
                "zip内にシンボリックリンクがあります: {}",
                file.name()
            )));
        }
        let path = file.enclosed_name().ok_or_else(|| {
            DeviceHostError::WorldPackImport(format!(
                "zip内に安全でないパスがあります: {}",
                file.name()
            ))
        })?;
        let Some(relative_path) = strip_pack_root(&path, inspected.root_prefix.as_deref()) else {
            return Err(DeviceHostError::WorldPackImport(
                "pack.jsonの外に別のファイルがあります。zipにはWorld Pack本体だけを入れてください。"
                    .to_string(),
            ));
        };
        let output_path = destination.join(relative_path);
        if file.is_dir() {
            fs::create_dir_all(&output_path)?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&output_path)?;
        io::copy(&mut file, &mut output)?;
    }
    Ok(())
}

pub(crate) fn pack_root_prefix(pack_json_path: &Path) -> Option<PathBuf> {
    let mut components = pack_json_path.components();
    let first = components.next()?.as_os_str().to_owned();
    if components.next().is_some() {
        Some(PathBuf::from(first))
    } else {
        None
    }
}

pub(crate) fn strip_pack_root(path: &Path, root_prefix: Option<&Path>) -> Option<PathBuf> {
    match root_prefix {
        Some(prefix) => path.strip_prefix(prefix).ok().map(Path::to_path_buf),
        None => Some(path.to_path_buf()),
    }
}

pub(crate) fn read_zip_text<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
    limit: Option<u64>,
) -> Result<String> {
    let mut file = archive.by_index(index).map_err(world_pack_zip_error)?;
    let mut bytes = Vec::new();
    match limit {
        Some(limit) => {
            let mut limited = (&mut file).take(limit);
            limited.read_to_end(&mut bytes)?;
        }
        None => {
            file.read_to_end(&mut bytes)?;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn truncate_text(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub(crate) fn imported_pack_dir_name(pack_id: &str) -> String {
    let sanitized: String = pack_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "imported-pack".to_string()
    } else {
        sanitized
    }
}

pub(crate) fn world_pack_zip_error(error: zip::result::ZipError) -> DeviceHostError {
    DeviceHostError::WorldPackImport(format!("zipファイルを読み込めませんでした: {}", error))
}
