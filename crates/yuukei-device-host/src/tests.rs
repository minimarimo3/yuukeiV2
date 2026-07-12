
use std::path::Path;

use serde_json::json;
use tempfile::tempdir;
use yuukei_protocol::NewEventLogRecord;

use super::*;

#[test]
fn cli_session_declares_terminal_surface() {
    let session = cli_surface_session("device-test");
    assert_eq!(session.surface_id, CLI_SURFACE_ID);
    assert_eq!(session.device_id, "device-test");
    assert_eq!(session.kind, SurfaceKind::Cli);
    assert_eq!(
        session.presentation.renderer,
        Some(SurfaceRenderer::Terminal)
    );
    assert_eq!(session.presentation.accepts_input, Some(true));
}

#[test]
fn tauri_session_declares_transparent_vrm_surface() {
    let session = tauri_surface_session("device-test");
    assert_eq!(session.surface_id, TAURI_SURFACE_ID);
    assert_eq!(session.device_id, "device-test");
    assert_eq!(session.kind, SurfaceKind::Desktop);
    assert_eq!(session.presentation.renderer, Some(SurfaceRenderer::Vrm));
    assert_eq!(session.presentation.transparent, Some(true));
    assert_eq!(session.presentation.accepts_input, Some(true));
    assert!(session
        .capabilities
        .iter()
        .any(|capability| capability == "avatar.gesture.poke"));
}

#[tokio::test]
async fn app_startup_event_dispatches_once_with_japanese_time_input() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_lifecycle_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .await?;
    let commands = runtime.emit_app_startup().await?;
    let second = runtime.emit_app_startup().await?;

    assert_eq!(commands.len(), 1);
    assert!(commands[0]
        .payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .starts_with("起動："));
    assert_eq!(
        commands[0]
            .target
            .as_ref()
            .and_then(|target| target.surface_id.as_deref()),
        Some(CLI_SURFACE_ID)
    );
    assert!(second.is_empty());

    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records
        .into_iter()
        .map(|record| record.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        records
            .iter()
            .filter(|kind| kind.as_str() == "app.startup")
            .count(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn device_power_events_are_logged_and_dispatched() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_lifecycle_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .await?;
    let sleep = runtime.emit_device_sleep_before().await?;
    let wake = runtime.emit_device_wake().await?;

    assert_eq!(sleep[0].payload["text"], "少し眠ります。");
    assert_eq!(wake[0].payload["text"], "おかえりなさい。");
    assert_eq!(
        wake[0]
            .target
            .as_ref()
            .and_then(|target| target.surface_id.as_deref()),
        Some(CLI_SURFACE_ID)
    );

    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records
        .into_iter()
        .map(|record| record.kind)
        .collect::<Vec<_>>();
    assert!(records.iter().any(|kind| kind == "device.sleep.before"));
    assert!(records.iter().any(|kind| kind == "device.wake"));
    Ok(())
}

#[tokio::test]
async fn stage_walk_ended_is_logged_with_actor_and_reason() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_lifecycle_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;

    runtime.emit_stage_walk_ended("yuukei", "arrived").await?;

    let records = runtime.home().event_log().read(EventLogQuery {
        kind: Some("stage.walk.ended".to_string()),
        ..EventLogQuery::default()
    })?;
    let record = records.records.first().expect("stage walk ended record");
    assert_eq!(record.actor_id.as_deref(), Some("yuukei"));
    assert_eq!(record.payload["reason"], json!("arrived"));
    assert_eq!(record.payload.len(), 1);

    runtime
        .emit_stage_walk_ended_for_command("yuukei", "replaced", "walk-1")
        .await?;
    let records = runtime.home().event_log().read(EventLogQuery {
        kind: Some("stage.walk.ended".to_string()),
        ..EventLogQuery::default()
    })?;
    assert_eq!(
        records.records[1]
            .causality
            .as_ref()
            .and_then(|causality| causality.source_command_id.as_deref()),
        Some("walk-1")
    );
    Ok(())
}

#[tokio::test]
async fn desktop_window_events_are_logged_with_desktop_observation_privacy() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_lifecycle_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .emit_desktop_window_transition(DesktopWindowTransition {
            kind: DesktopWindowTransitionKind::Appeared,
            window_key: "window-1".to_string(),
            app: "Finder".to_string(),
        })
        .await?;

    let records = runtime.home().event_log().read(EventLogQuery {
        kind: Some("desktop.window.appeared".to_string()),
        ..EventLogQuery::default()
    })?;
    let record = records.records.first().expect("desktop window record");
    assert_eq!(record.payload["windowKey"], json!("window-1"));
    assert_eq!(record.payload["app"], json!("Finder"));
    let privacy = record.privacy.as_ref().expect("desktop privacy");
    assert_eq!(privacy.category, DESKTOP_OBSERVATION_PRIVACY_CATEGORY);
    assert_eq!(privacy.retention, RetentionPolicy::Short);
    assert!(privacy.extension_readable);
    Ok(())
}

#[tokio::test]
async fn desktop_folder_and_download_events_store_only_normalized_payloads() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_lifecycle_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .emit_desktop_folder_transition(DesktopFolderTransition {
            category: DesktopFolderCategory::Downloads,
            app: "finder".to_string(),
        })
        .await?;
    runtime
        .emit_desktop_download_completed(DesktopDownloadObservation {
            file_name: "photo.png".to_string(),
            file_category: DownloadFileCategory::Image,
        })
        .await?;

    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    let folder = records
        .iter()
        .find(|record| record.kind == "desktop.folder.opened")
        .expect("folder record");
    assert_eq!(folder.payload["category"], json!("downloads"));
    assert_eq!(folder.payload["app"], json!("finder"));
    assert!(!folder.payload.contains_key("path"));
    assert_eq!(
        folder.privacy.as_ref().expect("folder privacy").category,
        DESKTOP_OBSERVATION_PRIVACY_CATEGORY
    );

    let download = records
        .iter()
        .find(|record| record.kind == "desktop.download.completed")
        .expect("download record");
    assert_eq!(download.payload["fileName"], json!("photo.png"));
    assert_eq!(download.payload["fileCategory"], json!("image"));
    assert!(!download.payload.contains_key("path"));
    assert_eq!(
        download
            .privacy
            .as_ref()
            .expect("download privacy")
            .category,
        DESKTOP_OBSERVATION_PRIVACY_CATEGORY
    );
    Ok(())
}

#[tokio::test]
async fn presence_tick_uses_life_tick_signal() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_presence_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .await?;
    let commands = runtime.emit_presence_tick().await?;

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].payload["text"], "生活時計です。");

    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records
        .into_iter()
        .map(|record| record.kind)
        .collect::<Vec<_>>();
    assert!(records.iter().any(|kind| kind == "presence.life_tick"));
    assert!(!records.iter().any(|kind| kind == "presence.idle_tick"));
    Ok(())
}

#[tokio::test]
async fn talk_impulse_uses_standard_alias_and_dispatches_dialogue() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_talk_impulse_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .await?;
    let commands = runtime.emit_talk_impulse().await?;

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].payload["text"], "少ししゃべります。");

    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records
        .into_iter()
        .map(|record| record.kind)
        .collect::<Vec<_>>();
    assert!(records.iter().any(|kind| kind == "presence.talk_impulse"));
    Ok(())
}

#[tokio::test]
async fn scene_history_persists_across_runtime_reopen() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_once_talk_impulse_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let first_runtime = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
    first_runtime
        .attach_surface(cli_surface_session(first_runtime.device_id()))
        .await?;
    let first_commands = first_runtime.emit_talk_impulse().await?;
    assert_eq!(first_commands.len(), 1);
    assert!(first_runtime.paths().scene_history_path.exists());
    let history = first_runtime.scene_history_state().await?;
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].event_name, "雑談");
    drop(first_runtime);

    let second_runtime = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
    second_runtime
        .attach_surface(cli_surface_session(second_runtime.device_id()))
        .await?;
    let second_commands = second_runtime.emit_talk_impulse().await?;
    assert!(second_commands.is_empty());
    let cleared = second_runtime.reset_scene_history().await?;
    assert!(cleared.entries.is_empty());
    drop(second_runtime);

    let third_runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    third_runtime
        .attach_surface(cli_surface_session(third_runtime.device_id()))
        .await?;
    let third_commands = third_runtime.emit_talk_impulse().await?;
    assert_eq!(third_commands.len(), 1);
    Ok(())
}

#[tokio::test]
async fn selected_default_uses_per_pack_event_log() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let snapshot = runtime.snapshot()?;

    assert_eq!(snapshot.world_pack_id, "default-yuukei");
    assert_eq!(runtime.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
    assert!(runtime
        .paths()
        .event_log_path
        .ends_with("residents/default-yuukei/events.sqlite3"));
    assert!(runtime
        .paths()
        .scene_history_path
        .ends_with("residents/default-yuukei/scene-history.json"));
    assert!(runtime
        .paths()
        .variables_path
        .ends_with("residents/default-yuukei/variables.json"));
    assert!(runtime.paths().event_log_path.exists());
    assert!(runtime
        .world_pack_status()
        .settings_path
        .ends_with("settings/world-packs.json"));
    Ok(())
}

#[tokio::test]
async fn actor_surface_assets_are_read_from_world_pack() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    let root = workspace.path().join("packs").join("default-yuukei");
    write_pack_with_renderer_assets(&root)?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let catalog = runtime.actor_surface_assets();

    assert_eq!(catalog.world_pack_id, "default-yuukei");
    assert_eq!(catalog.actors.len(), 2);
    assert_eq!(catalog.actors[0].actor_id, "yuukei");
    assert_eq!(
        catalog.actors[0]
            .renderer
            .as_ref()
            .map(|renderer| renderer.model.as_str()),
        Some("character/character_1.vrm")
    );
    assert_eq!(
        catalog.actors[0]
            .renderer
            .as_ref()
            .and_then(|renderer| renderer.hit_zones.first())
            .map(|hit_zone| hit_zone.id.as_str()),
        Some("head")
    );
    assert_eq!(
        catalog.actors[1]
            .renderer
            .as_ref()
            .and_then(|renderer| renderer.motions.get("walk"))
            .map(String::as_str),
        Some("motion/walk.vrma")
    );
    Ok(())
}

#[tokio::test]
async fn avatar_gesture_poke_is_logged_and_dispatches_daihon() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_avatar_gesture_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .attach_surface(tauri_surface_session(runtime.device_id()))
        .await?;
    let commands = runtime
        .send_avatar_gesture_poke(
            TAURI_SURFACE_ID,
            AvatarGesturePoke {
                actor_id: "yuukei".to_string(),
                hit_zone_id: "head".to_string(),
                hit_zone_label: Some("頭".to_string()),
                hit_surface: None,
                hit_bone: None,
                input: AvatarGestureInput {
                    kind: "pointer".to_string(),
                    button: "primary".to_string(),
                },
                screen: AvatarGestureScreen { x: 12.0, y: 34.0 },
            },
        )
        .await?;

    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("poke dialogue command");
    assert_eq!(
        dialogue.payload["text"],
        "わ、頭は急に触らないでください……！"
    );

    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    let poke = records
        .iter()
        .find(|record| record.kind == "avatar.gesture.poke")
        .expect("poke event log record");
    assert_eq!(poke.actor_id.as_deref(), Some("yuukei"));
    assert_eq!(poke.surface_id.as_deref(), Some(TAURI_SURFACE_ID));
    assert_eq!(poke.payload["hitZoneId"], json!("head"));
    assert_eq!(poke.payload["hitZoneLabel"], json!("頭"));
    assert!(records
        .iter()
        .any(|record| record.kind == "daihon.dispatch.result"));
    Ok(())
}

#[tokio::test]
async fn external_world_pack_persists_and_reuses_install() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let external_root = workspace.path().join("external-pack");
    write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
    let env = test_env(workspace.path(), data.path());

    let runtime =
        LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await?;
    let install_id = runtime.install_id().to_string();
    assert_eq!(runtime.snapshot()?.world_pack_id, "external-yuukei");
    assert!(runtime
        .paths()
        .event_log_path
        .ends_with(format!("residents/{install_id}/events.sqlite3")));
    assert_ne!(install_id, DEFAULT_WORLD_PACK_INSTALL_ID);

    let reopened = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
    assert_eq!(reopened.install_id(), install_id);
    assert_eq!(reopened.resident_id(), runtime.resident_id());
    assert_eq!(reopened.snapshot()?.world_pack_id, "external-yuukei");

    let selected_again =
        LocalYuukeiRuntime::select_world_pack_directory_in(env, &external_root).await?;
    assert_eq!(selected_again.install_id(), install_id);
    Ok(())
}

#[tokio::test]
async fn world_pack_zip_import_extracts_and_selects_pack() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let zip_path = data.path().join("zip-yuukei.zip");
    write_world_pack_zip(&zip_path, None, "zip-yuukei", "Zip Yuukei", &[])?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::import_world_pack_zip_in(env, &zip_path).await?;

    assert_eq!(runtime.snapshot()?.world_pack_id, "zip-yuukei");
    assert_eq!(
        runtime.world_pack_status().active_install.canonical_root,
        fs::canonicalize(data.path().join("packs-imported").join("zip-yuukei"))?
    );
    assert!(data
        .path()
        .join("packs-imported")
        .join("zip-yuukei")
        .join("pack.json")
        .exists());
    Ok(())
}

#[tokio::test]
async fn world_pack_zip_import_accepts_single_top_directory() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let zip_path = data.path().join("top-dir-yuukei.zip");
    write_world_pack_zip(
        &zip_path,
        Some("top-dir"),
        "top-dir-yuukei",
        "Top Dir Yuukei",
        &[],
    )?;
    let env = test_env(workspace.path(), data.path());

    let runtime = LocalYuukeiRuntime::import_world_pack_zip_in(env, &zip_path).await?;

    assert_eq!(runtime.snapshot()?.world_pack_id, "top-dir-yuukei");
    assert!(data
        .path()
        .join("packs-imported")
        .join("top-dir-yuukei")
        .join("scripts")
        .join("desktop_reactions.daihon")
        .exists());
    Ok(())
}

#[test]
fn world_pack_zip_inspection_rejects_zip_slip() -> Result<()> {
    let data = tempdir()?;
    let zip_path = data.path().join("evil.zip");
    write_zip_entries(
        &zip_path,
        vec![
            ("../evil", "nope".to_string()),
            ("pack.json", "{}".to_string()),
        ],
    )?;

    let error = LocalYuukeiRuntime::inspect_world_pack_zip_in(
        test_env(data.path(), data.path()),
        &zip_path,
    )
    .unwrap_err();

    assert!(error.to_string().contains("安全でないパス"));
    Ok(())
}

#[test]
fn world_pack_zip_inspection_rejects_missing_pack_json() -> Result<()> {
    let data = tempdir()?;
    let zip_path = data.path().join("missing.zip");
    write_zip_entries(&zip_path, vec![("README.md", "hello".to_string())])?;

    let error = LocalYuukeiRuntime::inspect_world_pack_zip_in(
        test_env(data.path(), data.path()),
        &zip_path,
    )
    .unwrap_err();

    assert!(error.to_string().contains("pack.jsonが見つかりません"));
    Ok(())
}

#[test]
fn world_pack_zip_inspection_rejects_entry_limit() -> Result<()> {
    let data = tempdir()?;
    let zip_path = data.path().join("too-many.zip");
    let mut entries = Vec::new();
    for index in 0..=WORLD_PACK_IMPORT_MAX_ENTRIES {
        entries.push((format!("files/{index}.txt"), String::new()));
    }
    write_zip_owned_entries(&zip_path, entries)?;

    let error = LocalYuukeiRuntime::inspect_world_pack_zip_in(
        test_env(data.path(), data.path()),
        &zip_path,
    )
    .unwrap_err();

    assert!(error.to_string().contains("ファイル数が多すぎます"));
    Ok(())
}

#[test]
fn world_pack_zip_license_text_prefers_license_then_readme_then_none() -> Result<()> {
    let data = tempdir()?;
    let with_license = data.path().join("with-license.zip");
    write_world_pack_zip(
        &with_license,
        None,
        "license-yuukei",
        "License Yuukei",
        &[("README.md", "readme text"), ("LICENSE", "license text")],
    )?;
    let inspection = LocalYuukeiRuntime::inspect_world_pack_zip_in(
        test_env(data.path(), data.path()),
        &with_license,
    )?;
    assert_eq!(inspection.license_source.as_deref(), Some("LICENSE"));
    assert_eq!(inspection.license_text.as_deref(), Some("license text"));

    let with_readme = data.path().join("with-readme.zip");
    write_world_pack_zip(
        &with_readme,
        None,
        "readme-yuukei",
        "Readme Yuukei",
        &[("README.md", "readme only")],
    )?;
    let inspection = LocalYuukeiRuntime::inspect_world_pack_zip_in(
        test_env(data.path(), data.path()),
        &with_readme,
    )?;
    assert_eq!(inspection.license_source.as_deref(), Some("README.md"));
    assert_eq!(inspection.license_text.as_deref(), Some("readme only"));

    let without_license = data.path().join("without-license.zip");
    write_world_pack_zip(&without_license, None, "plain-yuukei", "Plain Yuukei", &[])?;
    let inspection = LocalYuukeiRuntime::inspect_world_pack_zip_in(
        test_env(data.path(), data.path()),
        &without_license,
    )?;
    assert!(inspection.license_text.is_none());
    assert!(inspection.license_source.is_none());
    Ok(())
}

#[tokio::test]
async fn invalid_saved_external_pack_falls_back_to_default() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let external_root = workspace.path().join("external-pack");
    write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
    let env = test_env(workspace.path(), data.path());

    let selected =
        LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await?;
    let external_install_id = selected.install_id().to_string();
    fs::remove_file(external_root.join("pack.json"))?;

    let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
    let status = reopened.world_pack_status();
    assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
    assert_eq!(reopened.snapshot()?.world_pack_id, "default-yuukei");
    assert!(status.fallback_active);
    assert_eq!(status.configured_install_id, external_install_id);
    assert!(status.last_load_error.is_some());
    Ok(())
}

#[tokio::test]
async fn invalid_saved_external_daihon_is_reported_in_session_status_and_app_log() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let external_root = workspace.path().join("external-pack");
    write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
    let env = test_env(workspace.path(), data.path());

    let selected =
        LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await?;
    let external_install_id = selected.install_id().to_string();
    fs::write(
        external_root
            .join("scripts")
            .join("desktop_reactions.daihon"),
        r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: だれ
「届きません。」
"#,
    )?;

    let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
    let status = reopened.world_pack_status();

    assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
    assert!(status.fallback_active);
    assert_eq!(status.configured_install_id, external_install_id);
    assert_eq!(status.daihon_diagnostics.len(), 1);
    assert_eq!(
        status.daihon_diagnostics[0].install_id.as_deref(),
        Some(external_install_id.as_str())
    );
    let external_root = display_path(fs::canonicalize(&external_root)?);
    assert_eq!(
        status.daihon_diagnostics[0].pack_root.as_deref(),
        Some(external_root.as_str())
    );
    assert!(status.daihon_diagnostics[0]
        .message
        .contains("unknown Daihon speaker"));

    let raw_log = fs::read_to_string(data.path().join("app-activity.jsonl"))?;
    assert!(raw_log.contains("\"type\":\"daihon.diagnostics\""));
    assert!(raw_log.contains("\"context\":\"world-pack.fallback-load\""));
    Ok(())
}

#[tokio::test]
async fn manual_selection_rejects_missing_required_capability_without_saving() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let external_root = workspace.path().join("external-pack");
    write_pack(
        &external_root,
        "external-yuukei",
        "External Yuukei",
        &["dialogue.generate"],
    )?;
    let env = test_env(workspace.path(), data.path());

    let error =
        match LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await
        {
            Ok(_) => panic!("missing required capability should reject the world pack"),
            Err(error) => error,
        };
    assert!(error.to_string().contains("dialogue.generate"));

    let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
    assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
    assert!(!reopened.world_pack_status().fallback_active);
    Ok(())
}

#[tokio::test]
async fn manual_selection_rejects_invalid_speaker_alias_without_saving() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let external_root = workspace.path().join("external-pack");
    write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
    let pack_path = external_root.join("pack.json");
    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&pack_path)?)?;
    manifest["actors"][0]["speakerAliases"] = json!(["ゆ", "ゆ"]);
    fs::write(&pack_path, serde_json::to_string_pretty(&manifest)?)?;
    let env = test_env(workspace.path(), data.path());

    let error =
        match LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await
        {
            Ok(_) => panic!("invalid speaker alias should reject the world pack"),
            Err(error) => error,
        };
    assert!(error.to_string().contains("duplicate speaker alias"));

    let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
    assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
    assert!(!reopened.world_pack_status().fallback_active);
    Ok(())
}

#[tokio::test]
async fn extension_install_copies_folder_and_persists_settings() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let source = workspace.path().join("downloads").join("nya-process");
    write_extension_source(&source, "nya-process", "Nya Process", "にゃ")?;
    let env = test_env(workspace.path(), data.path());

    let state = LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let snapshot = runtime.snapshot()?;

    assert!(runtime.paths().extension_root.ends_with("extensions"));
    assert!(data
        .path()
        .join("extensions")
        .join("nya-process")
        .join("manifest.json")
        .exists());
    assert!(data
        .path()
        .join("settings")
        .join("extensions.json")
        .exists());
    assert_eq!(state.installed[0].extension_id, "nya-process");
    assert_eq!(
        state
            .hook_order
            .get(&ExtensionHookPoint::BeforeCommandEmit)
            .cloned()
            .unwrap_or_default(),
        vec!["nya-process".to_string()]
    );
    assert!(snapshot.extensions.contains_key("nya-process"));
    Ok(())
}

#[tokio::test]
async fn disabled_extension_is_preserved_but_not_executed() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let source = workspace.path().join("downloads").join("disabled-process");
    write_extension_source(&source, "disabled-process", "Disabled Process", "にゃ")?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let state =
        LocalYuukeiRuntime::set_extension_enabled_in(env.clone(), "disabled-process", false)?;
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let commands = runtime
        .send_conversation_text(CLI_SURFACE_ID, "こんにちは")
        .await?;

    assert!(!state.installed[0].enabled);
    assert_eq!(
        state
            .hook_order
            .get(&ExtensionHookPoint::BeforeCommandEmit)
            .cloned()
            .unwrap_or_default(),
        vec!["disabled-process".to_string()]
    );
    assert!(!runtime.snapshot()?.extensions["disabled-process"].enabled);
    assert!(!commands[0]
        .payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .ends_with("にゃ"));
    Ok(())
}

#[tokio::test]
async fn extension_hook_order_is_user_owned_and_process_cwd_is_installed_dir() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let nya_source = workspace.path().join("downloads").join("z-nya");
    let english_source = workspace.path().join("downloads").join("a-english");
    write_extension_source(&nya_source, "nya-suffix", "Nya Suffix", "にゃ")?;
    write_extension_source(&english_source, "english-marker", "English Marker", " EN")?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &nya_source)?;
    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &english_source)?;
    LocalYuukeiRuntime::set_extension_hook_order_in(
        env.clone(),
        ExtensionHookPoint::BeforeCommandEmit,
        vec!["english-marker".to_string(), "nya-suffix".to_string()],
    )?;
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let commands = runtime
        .send_conversation_text(CLI_SURFACE_ID, "こんにちは")
        .await?;

    assert!(commands[0]
        .payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .ends_with(" ENにゃ"));
    Ok(())
}

#[tokio::test]
async fn extension_capability_defaults_persist_and_apply_before_home_start() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &["dialogue.generate"],
    )?;
    let source = workspace.path().join("downloads").join("user-tts");
    write_capability_extension_source(
        &source,
        "user-tts",
        "User TTS",
        &["speech.synthesis", "dialogue.generate"],
    )?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let runtime = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
    let mut receiver = runtime.home().subscribe_commands();
    runtime
        .send_conversation_text(CLI_SURFACE_ID, "こんにちは")
        .await?;
    let audio = next_runtime_command_of_kind(&mut receiver, "audio.play").await;
    assert_eq!(audio.payload["audioPath"], json!("/tmp/user-tts/cmd_1.wav"));
    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    assert!(records.iter().any(|record| {
        record.kind == "capability.invocation.result"
            && record.payload["extensionId"] == json!("user-tts")
    }));

    let state = LocalYuukeiRuntime::set_capability_default_in(
        env.clone(),
        "speech.synthesis",
        "yuukei.default-tts",
    )?;
    assert_eq!(
        state.capability_defaults.get("speech.synthesis"),
        Some(&"yuukei.default-tts".to_string())
    );
    let reopened = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
    let mut receiver = reopened.home().subscribe_commands();
    reopened
        .send_conversation_text(CLI_SURFACE_ID, "もう一度")
        .await?;
    let audio = next_runtime_command_of_kind(&mut receiver, "audio.play").await;
    assert_eq!(audio.payload["audioPath"], json!("/tmp/user-tts/cmd_1.wav"));

    let state =
        LocalYuukeiRuntime::set_capability_default_in(env.clone(), "speech.synthesis", "user-tts")?;
    assert_eq!(
        state.capability_defaults.get("speech.synthesis"),
        Some(&"user-tts".to_string())
    );
    let raw_settings = fs::read_to_string(data.path().join("settings").join("extensions.json"))?;
    assert!(raw_settings.contains("\"capabilityDefaults\""));
    assert!(raw_settings.contains("\"user-tts\""));

    let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
    let mut receiver = reopened.home().subscribe_commands();
    reopened
        .send_conversation_text(CLI_SURFACE_ID, "さらに")
        .await?;
    let audio = next_runtime_command_of_kind(&mut receiver, "audio.play").await;
    assert_eq!(audio.payload["audioPath"], json!("/tmp/user-tts/cmd_1.wav"));
    Ok(())
}

#[tokio::test]
async fn process_extension_receives_persistent_extension_data_dir() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let source = workspace.path().join("downloads").join("memory-env");
    write_extension_data_dir_probe_source(&source)?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let commands = runtime
        .send_conversation_text(CLI_SURFACE_ID, "環境変数は？")
        .await?;
    assert!(commands
        .iter()
        .any(|command| command.payload["text"] == json!("data dir ok")));

    let expected_dir = data.path().join("extension-data").join("memory-env");
    assert!(expected_dir.is_dir());
    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    let result = records
        .iter()
        .find(|record| {
            record.kind == "capability.invocation.result"
                && record.payload["extensionId"] == json!("memory-env")
        })
        .expect("capability result");
    assert_eq!(result.payload["metadata"]["exists"], json!(true));
    assert_eq!(
        result.payload["metadata"]["dataDir"],
        json!(expected_dir.to_string_lossy().to_string())
    );
    Ok(())
}

#[tokio::test]
async fn extension_setting_values_and_secrets_persist_without_exposing_secret() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let source = workspace.path().join("downloads").join("settings-probe");
    write_settings_probe_extension_source(&source)?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let state = LocalYuukeiRuntime::set_extension_setting_values_in(
        env.clone(),
        "settings-probe",
        Map::from_iter([
            ("provider".to_string(), json!("gemini")),
            ("timeoutMs".to_string(), json!(45000)),
        ]),
    )?;
    assert_eq!(
        state.installed[0].setting_values["provider"],
        json!("gemini")
    );
    assert!(state.installed[0].secrets_set.is_empty());

    let state = LocalYuukeiRuntime::set_extension_secret_in(
        env.clone(),
        "settings-probe",
        "gemini.apiKey",
        Some("super-secret".to_string()),
    )?;
    assert_eq!(state.installed[0].secrets_set, vec!["gemini.apiKey"]);
    assert!(!serde_json::to_string(&state)?.contains("super-secret"));

    let raw_settings = fs::read_to_string(data.path().join("settings").join("extensions.json"))?;
    assert!(raw_settings.contains("\"extensionValues\""));
    assert!(raw_settings.contains("\"provider\""));
    assert!(!raw_settings.contains("super-secret"));

    let secrets_path = data.path().join("settings").join("extension-secrets.json");
    let raw_secrets = fs::read_to_string(&secrets_path)?;
    assert!(raw_secrets.contains("super-secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&secrets_path)?.permissions().mode() & 0o777,
            0o600
        );
    }

    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let commands = runtime
        .send_conversation_text(CLI_SURFACE_ID, "設定は？")
        .await?;
    assert!(commands
        .iter()
        .any(|command| command.payload["text"] == json!("settings ok")));
    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    assert!(!serde_json::to_string(&records)?.contains("super-secret"));
    Ok(())
}

#[test]
fn extension_setting_values_reject_invalid_values() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    let source = workspace.path().join("downloads").join("settings-probe");
    write_settings_probe_extension_source(&source)?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let unknown = LocalYuukeiRuntime::set_extension_setting_values_in(
        env.clone(),
        "settings-probe",
        Map::from_iter([("missing".to_string(), json!("value"))]),
    )
    .unwrap_err();
    assert!(unknown.to_string().contains("unknown setting key"));

    let type_error = LocalYuukeiRuntime::set_extension_setting_values_in(
        env.clone(),
        "settings-probe",
        Map::from_iter([("timeoutMs".to_string(), json!("fast"))]),
    )
    .unwrap_err();
    assert!(type_error.to_string().contains("must be number"));

    let select_error = LocalYuukeiRuntime::set_extension_setting_values_in(
        env.clone(),
        "settings-probe",
        Map::from_iter([("provider".to_string(), json!("unknown-provider"))]),
    )
    .unwrap_err();
    assert!(select_error.to_string().contains("declared options"));

    let min_error = LocalYuukeiRuntime::set_extension_setting_values_in(
        env,
        "settings-probe",
        Map::from_iter([("timeoutMs".to_string(), json!(500))]),
    )
    .unwrap_err();
    assert!(min_error.to_string().contains("below minimum"));
    Ok(())
}

#[test]
fn extension_manifest_rejects_invalid_settings_schema() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    let source = workspace.path().join("downloads").join("settings-probe");
    write_settings_probe_extension_source(&source)?;
    let manifest_path = source.join("manifest.json");
    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    manifest["settings"]["fields"][1]["key"] = json!("provider");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    let env = test_env(workspace.path(), data.path());
    let duplicate =
        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source).unwrap_err();
    assert!(duplicate.to_string().contains("duplicate setting key"));

    write_settings_probe_extension_source(&source)?;
    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    manifest["settings"]["fields"][2]["default"] = json!("should-not-load");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    let secret_default =
        LocalYuukeiRuntime::install_extension_directory_in(env, &source).unwrap_err();
    assert!(secret_default
        .to_string()
        .contains("secret setting cannot declare default"));
    Ok(())
}

#[test]
fn capability_usage_aggregates_usage_metadata_with_7_day_boundary() {
    let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    let records = vec![
        usage_record(
            1,
            "2026-07-06T12:00:00.000Z",
            "yuukei-intelligence",
            "dialogue.generate",
            "openai-compatible",
            "local-model",
            10,
            4,
        ),
        usage_record(
            2,
            "2026-06-29T12:00:00.000Z",
            "yuukei-intelligence",
            "dialogue.generate",
            "openai-compatible",
            "local-model",
            20,
            6,
        ),
        usage_record(
            3,
            "2026-06-29T11:59:59.000Z",
            "yuukei-intelligence",
            "dialogue.generate",
            "openai-compatible",
            "local-model",
            30,
            8,
        ),
        usage_record(
            4,
            "2026-07-06T12:00:00.000Z",
            "yuukei-intelligence",
            "memory.index",
            "gemini",
            "gemini-2.5-flash",
            40,
            12,
        ),
    ];

    let usage = capability_usage_from_records(&records, now);
    let extension = usage
        .extensions
        .iter()
        .find(|extension| extension.extension_id == "yuukei-intelligence")
        .expect("extension usage");
    let dialogue = extension
        .capabilities
        .iter()
        .find(|capability| capability.capability == "dialogue.generate")
        .expect("dialogue usage");
    let local_model = &dialogue.models[0];

    assert_eq!(
        local_model.all_time,
        TokenUsageTotals {
            requests: 3,
            input_tokens: 60,
            output_tokens: 18
        }
    );
    assert_eq!(
        local_model.last_7_days,
        TokenUsageTotals {
            requests: 2,
            input_tokens: 30,
            output_tokens: 10
        }
    );
    assert!(extension
        .capabilities
        .iter()
        .any(|capability| capability.capability == "memory.index"));
}

#[tokio::test]
async fn capability_usage_reads_capability_result_events() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_pack(
        &workspace.path().join("packs").join("default-yuukei"),
        "default-yuukei",
        "Default Yuukei",
        &[],
    )?;
    let env = test_env(workspace.path(), data.path());
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime.home().event_log().append(NewEventLogRecord {
        id: "evt_usage_runtime".to_string(),
        kind: "capability.invocation.result".to_string(),
        timestamp: Utc::now().to_rfc3339(),
        resident_id: DEFAULT_RESIDENT_ID.to_string(),
        source: "capability".to_string(),
        device_id: None,
        surface_id: None,
        actor_id: None,
        payload: JsonMap::from([
            ("invocationId".to_string(), json!("cap_1")),
            ("extensionId".to_string(), json!("usage-extension")),
            ("capability".to_string(), json!("dialogue.generate")),
            ("output".to_string(), json!({ "speak": true })),
            (
                "metadata".to_string(),
                json!({
                    "usage": {
                        "inputTokens": 12,
                        "outputTokens": 5,
                        "model": "usage-model",
                        "provider": "openai-compatible"
                    }
                }),
            ),
        ]),
        causality: None,
        privacy: None,
    })?;

    let usage = runtime.capability_usage()?;
    let extension = usage
        .extensions
        .iter()
        .find(|extension| extension.extension_id == "usage-extension")
        .expect("extension usage");
    let model = &extension.capabilities[0].models[0];

    assert_eq!(extension.capabilities[0].capability, "dialogue.generate");
    assert_eq!(model.provider, "openai-compatible");
    assert_eq!(model.model, "usage-model");
    assert_eq!(
        model.all_time,
        TokenUsageTotals {
            requests: 1,
            input_tokens: 12,
            output_tokens: 5
        }
    );
    assert_eq!(model.last_7_days, model.all_time);
    Ok(())
}

#[tokio::test]
async fn yuukei_intelligence_process_generates_dialogue_through_device_host() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let source = workspace
        .path()
        .join("downloads")
        .join("yuukei-intelligence");
    write_yuukei_intelligence_extension_source(&source)?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let commands = runtime
        .send_conversation_text(CLI_SURFACE_ID, "何か言う？")
        .await?;

    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("generated dialogue command");
    assert_eq!(dialogue.payload["text"], json!("スタブから返事します。"));
    assert_eq!(dialogue.source, "capability.dialogue.generate");
    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    let result = records
        .iter()
        .find(|record| {
            record.kind == "capability.invocation.result"
                && record.payload["extensionId"] == json!("yuukei-intelligence")
                && record.payload["capability"] == json!("dialogue.generate")
        })
        .expect("dialogue.generate result");
    assert_eq!(
        result.payload["metadata"]["usage"],
        json!({
            "inputTokens": 13,
            "outputTokens": 5,
            "model": "stub-model",
            "provider": "openai-compatible"
        })
    );
    Ok(())
}

#[tokio::test]
async fn yuukei_intelligence_process_interprets_dialogue_through_device_host() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_interpret_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let source = workspace
        .path()
        .join("downloads")
        .join("yuukei-intelligence");
    write_yuukei_intelligence_extension_source(&source)?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let startup = runtime.emit_app_startup().await?;
    assert!(startup.iter().any(|command| {
        command.kind == "dialogue.say"
            && command.payload["text"] == json!("今日はお出かけの日だよね！")
    }));

    let commands = runtime
        .send_conversation_text(CLI_SURFACE_ID, "あ〜うん。いやちょっと忙しくて...")
        .await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("interpreted dialogue command");
    assert_eq!(
        dialogue.payload["text"],
        json!("そっか、残念だけど無理しないでね。")
    );
    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    assert!(records.iter().any(|record| {
        record.kind == "capability.invocation.result"
            && record.payload["extensionId"] == json!("yuukei-intelligence")
            && record.payload["capability"] == json!("dialogue.interpret")
            && record.payload["output"]["choice"] == json!("いいえ")
    }));
    Ok(())
}

#[tokio::test]
async fn yuukei_intelligence_process_indexes_and_retrieves_memory_through_device_host() -> Result<()>
{
    let workspace = tempdir()?;
    let data = tempdir()?;
    write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
    let source = workspace
        .path()
        .join("downloads")
        .join("yuukei-intelligence");
    write_yuukei_intelligence_extension_source(&source)?;
    let env = test_env(workspace.path(), data.path());

    LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    let mut yesterday = RuntimeEvent::new("conversation.text", "surface", DEFAULT_RESIDENT_ID);
    yesterday.timestamp = (Local::now() - chrono::Duration::days(1)).to_rfc3339();
    yesterday
        .payload
        .insert("text".to_string(), json!("唐揚げを食べた"));
    runtime
        .home()
        .event_log()
        .append(NewEventLogRecord::from(yesterday))?;

    runtime.emit_app_startup().await?;
    let commands = runtime
        .send_conversation_text(CLI_SURFACE_ID, "唐揚げのこと覚えてる？")
        .await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("generated dialogue command");
    assert_eq!(
        dialogue.payload["text"],
        json!("覚えています。唐揚げが好きなんですよね。")
    );

    let records = runtime
        .home()
        .event_log()
        .read(EventLogQuery::default())?
        .records;
    assert!(records.iter().any(|record| {
        record.kind == "capability.invocation.result"
            && record.payload["capability"] == json!("memory.index")
            && record.payload["output"]["indexed"] == json!(true)
    }));
    assert!(records.iter().any(|record| {
        record.kind == "capability.invocation.result"
            && record.payload["capability"] == json!("memory.retrieve")
            && record.payload["output"]["memories"][0]["text"] == json!("唐揚げが好き。")
    }));
    Ok(())
}

#[test]
fn process_extension_manifest_rejects_non_process_runtime() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    let source = workspace.path().join("downloads").join("bad-runtime");
    write_extension_source(&source, "bad-runtime", "Bad Runtime", "にゃ")?;
    let manifest_path = source.join("manifest.json");
    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    manifest["runtime"] = json!("wasm");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

    let env = test_env(workspace.path(), data.path());
    let error = LocalYuukeiRuntime::install_extension_directory_in(env, &source).unwrap_err();
    assert!(error.to_string().contains("runtime"));
    assert!(error.to_string().contains("process"));
    Ok(())
}

#[test]
fn process_extension_manifest_rejects_cross_namespace_signal_alias() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    let source = workspace.path().join("downloads").join("bad-alias");
    write_extension_source(&source, "bad-alias", "Bad Alias", "にゃ")?;
    let manifest_path = source.join("manifest.json");
    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    manifest["emittedEvents"] = json!(["ext.bad-alias.allowed"]);
    manifest["signalAliases"] = json!([
        { "alias": "会話_偽装", "signal": "conversation.text" }
    ]);
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

    let env = test_env(workspace.path(), data.path());
    let error = LocalYuukeiRuntime::install_extension_directory_in(env, &source).unwrap_err();
    assert!(error.to_string().contains("signal alias"));
    Ok(())
}

fn test_env(workspace_root: &Path, data_dir: &Path) -> LocalRuntimeEnvironment {
    LocalRuntimeEnvironment {
        workspace_root: workspace_root.to_path_buf(),
        default_world_root: workspace_root.join("packs").join("default-yuukei"),
        data_dir: data_dir.to_path_buf(),
        device_id: "device-test".to_string(),
    }
}

fn write_pack(
    root: &Path,
    id: &str,
    display_name: &str,
    required_capabilities: &[&str],
) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::write(
        root.join("scripts").join("desktop_reactions.daihon"),
        r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
ユーザー発言=入力#ユーザー発言
「聞こえています。＜ユーザー発言＞」
"#,
    )?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": id,
            "displayName": display_name,
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": display_name,
                    "profile": {}
                }
            ],
            "signals": {
                "allow": ["conversation.text", "surface.attach"]
            },
            "capabilities": {
                "required": required_capabilities,
                "optional": ["speech.synthesis"]
            },
            "daihon": {
                "scripts": ["scripts/desktop_reactions.daihon"]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn usage_record(
    sequence: i64,
    timestamp: &str,
    extension_id: &str,
    capability: &str,
    provider: &str,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> EventLogRecord {
    EventLogRecord {
        sequence,
        id: format!("evt_usage_{sequence}"),
        kind: "capability.invocation.result".to_string(),
        timestamp: timestamp.to_string(),
        resident_id: DEFAULT_RESIDENT_ID.to_string(),
        source: "capability".to_string(),
        device_id: None,
        surface_id: None,
        actor_id: None,
        payload: JsonMap::from([
            ("extensionId".to_string(), json!(extension_id)),
            ("capability".to_string(), json!(capability)),
            (
                "metadata".to_string(),
                json!({
                    "usage": {
                        "inputTokens": input_tokens,
                        "outputTokens": output_tokens,
                        "model": model,
                        "provider": provider
                    }
                }),
            ),
        ]),
        causality: None,
        privacy: None,
    }
}

fn write_llm_fallback_pack(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "default-yuukei",
            "displayName": "Default Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Yuukei",
                    "profile": {
                        "role": "UI resident",
                        "speechStyle": "short and present"
                    }
                }
            ],
            "signals": {
                "allow": ["conversation.text", "surface.attach"]
            },
            "capabilities": {
                "required": [],
                "optional": ["speech.synthesis", "dialogue.generate"]
            },
            "llmDelegation": {
                "signals": [{ "signal": "conversation.text" }]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

fn write_interpret_pack(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::write(
        root.join("scripts").join("interpret_demo.daihon"),
        r#"
## interpret demo
### startup
合図: ＠アプリ_起動
話者: yuukei
話題=「お出かけ確認」
「今日はお出かけの日だよね！」
### reply
合図: ＠会話_入力
条件:（話題 = 「お出かけ確認」）
話者: yuukei
判定=＜解釈 (入力#ユーザー発言) 「ユーザーは今日のお出かけに行けますか？」 「はい/いいえ」＞
※（判定 = 「はい」）なら:
「やった、楽しみにしてるね。」
話題=「」
※あるいは（判定 = 「いいえ」）なら:
「そっか、残念だけど無理しないでね。」
話題=「」
※あるいは（判定 = 「不明」）なら:
「ん、今日は行けそう？それとも難しそう？」
※それ以外:
「ん、今日は行けそう？それとも難しそう？」
おわり
"#,
    )?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "default-yuukei",
            "displayName": "Default Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Yuukei",
                    "profile": {
                        "role": "UI resident",
                        "speechStyle": "short and present"
                    }
                }
            ],
            "signals": {
                "allow": ["conversation.text", "app.startup"]
            },
            "capabilities": {
                "required": [],
                "optional": ["dialogue.interpret", "dialogue.generate"]
            },
            "daihon": {
                "scripts": ["scripts/interpret_demo.daihon"]
            },
            "initialVariables": {
                "話題": ""
            },
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

fn world_pack_zip_entries(
    top_dir: Option<&str>,
    id: &str,
    display_name: &str,
    extra_root_files: &[(&str, &str)],
) -> Vec<(String, String)> {
    let prefix = top_dir
        .filter(|dir| !dir.is_empty())
        .map(|dir| format!("{dir}/"))
        .unwrap_or_default();
    let mut entries = vec![
        (
            format!("{prefix}pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": id,
                "displayName": display_name,
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": display_name,
                        "profile": {}
                    }
                ],
                "signals": {
                    "allow": ["conversation.text", "surface.attach"]
                },
                "capabilities": {
                    "required": [],
                    "optional": []
                },
                "daihon": {
                    "scripts": ["scripts/desktop_reactions.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))
            .expect("pack json"),
        ),
        (
            format!("{prefix}scripts/desktop_reactions.daihon"),
            r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
「zipから来ました。」
"#
            .to_string(),
        ),
    ];
    for (path, text) in extra_root_files {
        entries.push((format!("{prefix}{path}"), (*text).to_string()));
    }
    entries
}

fn write_world_pack_zip(
    path: &Path,
    top_dir: Option<&str>,
    id: &str,
    display_name: &str,
    extra_root_files: &[(&str, &str)],
) -> Result<()> {
    write_zip_owned_entries(
        path,
        world_pack_zip_entries(top_dir, id, display_name, extra_root_files),
    )
}

fn write_zip_entries(path: &Path, entries: Vec<(&str, String)>) -> Result<()> {
    write_zip_owned_entries(
        path,
        entries
            .into_iter()
            .map(|(name, contents)| (name.to_string(), contents))
            .collect(),
    )
}

fn write_zip_owned_entries(path: &Path, entries: Vec<(String, String)>) -> Result<()> {
    let file = File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, contents) in entries {
        zip.start_file(name, options)
            .map_err(world_pack_zip_error)?;
        zip.write_all(contents.as_bytes())?;
    }
    zip.finish().map_err(world_pack_zip_error)?;
    Ok(())
}

fn write_pack_with_renderer_assets(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::create_dir_all(root.join("character"))?;
    fs::create_dir_all(root.join("motion"))?;
    fs::write(
        root.join("scripts").join("desktop_reactions.daihon"),
        r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
「聞こえています。」
"#,
    )?;
    fs::write(root.join("character").join("character_1.vrm"), [])?;
    fs::write(root.join("character").join("character_2.vrm"), [])?;
    fs::write(root.join("motion").join("walk.vrma"), [])?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "default-yuukei",
            "displayName": "Default Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Yuukei",
                    "profile": {},
                    "renderer": {
                        "kind": "vrm",
                        "model": "character/character_1.vrm",
                        "motions": {
                            "walk": "motion/walk.vrma"
                        },
                        "hitZones": [
                            {
                                "id": "head",
                                "label": "頭",
                                "source": "humanoidBone",
                                "bones": ["head"],
                                "shape": "auto",
                                "events": ["avatar.gesture.poke", "avatar.gesture.pat"],
                                "priority": 40
                            }
                        ]
                    }
                },
                {
                    "id": "partner",
                    "displayName": "Partner",
                    "profile": {},
                    "renderer": {
                        "kind": "vrm",
                        "model": "character/character_2.vrm",
                        "motions": {
                            "walk": "motion/walk.vrma"
                        }
                    }
                }
            ],
            "signals": {
                "allow": ["conversation.text", "surface.attach"]
            },
            "capabilities": {
                "required": [],
                "optional": []
            },
            "daihon": {
                "scripts": ["scripts/desktop_reactions.daihon"]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

fn write_avatar_gesture_pack(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::write(
        root.join("scripts").join("desktop_reactions.daihon"),
        r#"
## desktop reactions
### poke head
合図: ＠住人_つつく
条件:（入力#hitZoneId = 「head」）
話者: yuukei
＜表情 照れ＞
「わ、頭は急に触らないでください……！」
"#,
    )?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "default-yuukei",
            "displayName": "Default Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Yuukei",
                    "profile": {},
                    "renderer": {
                        "kind": "vrm",
                        "model": "character/character_1.vrm",
                        "hitZones": [
                            {
                                "id": "head",
                                "label": "頭",
                                "source": "humanoidBone",
                                "bones": ["head"],
                                "shape": "auto",
                                "events": ["avatar.gesture.poke", "avatar.gesture.pat"]
                            }
                        ]
                    }
                }
            ],
            "signals": {
                "allow": ["avatar.gesture.poke", "surface.attach"]
            },
            "capabilities": {
                "required": [],
                "optional": ["speech.synthesis"]
            },
            "daihon": {
                "scripts": ["scripts/desktop_reactions.daihon"]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    fs::create_dir_all(root.join("character"))?;
    fs::write(root.join("character").join("character_1.vrm"), [])?;
    Ok(())
}

fn write_lifecycle_pack(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::write(
        root.join("scripts").join("desktop_reactions.daihon"),
        r#"
## desktop reactions
### startup
合図: ＠アプリ_起動
話者: yuukei
「起動：＜入力#時間帯＞」

### before sleep
合図: ＠端末_スリープ前
話者: yuukei
「少し眠ります。」

### wake
合図: ＠端末_復帰
話者: yuukei
「おかえりなさい。」
"#,
    )?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "default-yuukei",
            "displayName": "Default Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Default Yuukei",
                    "profile": {}
                }
            ],
            "signals": {
                "allow": ["app.startup", "surface.attach", "device.sleep.before", "device.wake"]
            },
            "capabilities": {
                "required": [],
                "optional": ["speech.synthesis"]
            },
            "daihon": {
                "scripts": ["scripts/desktop_reactions.daihon"]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

fn write_presence_pack(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::write(
        root.join("scripts").join("desktop_reactions.daihon"),
        r#"
## desktop reactions
### life tick
合図: ＠生活_定期
話者: yuukei
「生活時計です。」
"#,
    )?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "default-yuukei",
            "displayName": "Default Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Default Yuukei",
                    "profile": {}
                }
            ],
            "signals": {
                "allow": ["presence.life_tick", "surface.attach"]
            },
            "capabilities": {
                "required": [],
                "optional": ["speech.synthesis"]
            },
            "daihon": {
                "scripts": ["scripts/desktop_reactions.daihon"]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

fn write_talk_impulse_pack(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::write(
        root.join("scripts").join("random_talk.daihon"),
        r#"
## 雑談
### talk
合図: ＠雑談_定期
話者: yuukei
「少ししゃべります。」
"#,
    )?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "talk-yuukei",
            "displayName": "Talk Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Talk Yuukei",
                    "profile": {}
                }
            ],
            "signals": {
                "allow": ["presence.talk_impulse", "surface.attach"]
            },
            "capabilities": {
                "required": [],
                "optional": []
            },
            "daihon": {
                "scripts": ["scripts/random_talk.daihon"]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

fn write_once_talk_impulse_pack(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("scripts"))?;
    fs::write(
        root.join("scripts").join("random_talk.daihon"),
        r#"
## 雑談
### talk
合図: ＠雑談_定期
頻度: 一度きり
話者: yuukei
「一度だけしゃべります。」
"#,
    )?;
    fs::write(
        root.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "talk-yuukei",
            "displayName": "Talk Yuukei",
            "defaultActorId": "yuukei",
            "actors": [
                {
                    "id": "yuukei",
                    "displayName": "Talk Yuukei",
                    "profile": {}
                }
            ],
            "signals": {
                "allow": ["presence.talk_impulse", "surface.attach"]
            },
            "capabilities": {
                "required": [],
                "optional": []
            },
            "daihon": {
                "scripts": ["scripts/random_talk.daihon"]
            },
            "initialVariables": {},
            "uiSpace": {}
        }))?,
    )?;
    Ok(())
}

fn write_extension_source(root: &Path, id: &str, display_name: &str, suffix: &str) -> Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": id,
            "displayName": display_name,
            "hooks": [
                {
                    "hookPoint": "beforeCommandEmit",
                    "commandTypes": ["dialogue.say"]
                }
            ],
            "process": {
                "command": "node",
                "args": ["append.js", suffix]
            }
        }))?,
    )?;
    fs::write(
        root.join("append.js"),
        r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const suffix = process.argv[2] ?? "";
const command = input.command;
command.payload.text = String(command.payload.text ?? "") + suffix;
process.stdout.write(JSON.stringify({ action: "replaceCommand", command }));
"#,
    )?;
    Ok(())
}

async fn next_runtime_command_of_kind(
    receiver: &mut tokio::sync::broadcast::Receiver<RuntimeCommand>,
    kind: &str,
) -> RuntimeCommand {
    loop {
        let command = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
            .await
            .expect("runtime command timed out")
            .expect("runtime command channel closed");
        if command.kind == kind {
            return command;
        }
    }
}

fn write_capability_extension_source(
    root: &Path,
    id: &str,
    display_name: &str,
    capabilities: &[&str],
) -> Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": id,
            "displayName": display_name,
            "capabilities": capabilities.iter().map(|capability| json!({
                "capability": capability,
                "methods": ["invoke"]
            })).collect::<Vec<_>>(),
            "process": {
                "command": "node",
                "args": ["capability.js"]
            }
        }))?,
    )?;
    fs::write(
        root.join("capability.js"),
        format!(
            r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const output = input.capability === "speech.synthesis"
  ? {{ audioPath: "/tmp/user-tts/cmd_1.wav", durationMs: 1200, format: "wav" }}
  : {{}};
process.stdout.write(JSON.stringify({{
  invocationId: input.id,
  extensionId: "{id}",
  capability: input.capability,
  output,
  metadata: {{}}
}}));
"#
        ),
    )?;
    Ok(())
}

fn write_extension_data_dir_probe_source(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "memory-env",
            "displayName": "Memory Env Probe",
            "capabilities": [
                {
                    "capability": "dialogue.generate",
                    "methods": ["generate"]
                }
            ],
            "process": {
                "command": "node",
                "args": ["probe.js"]
            }
        }))?,
    )?;
    fs::write(
        root.join("probe.js"),
        r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const dataDir = process.env.YUUKEI_EXTENSION_DATA_DIR ?? "";
process.stdout.write(JSON.stringify({
  invocationId: input.id,
  extensionId: "memory-env",
  capability: input.capability,
  output: { speak: true, text: "data dir ok" },
  metadata: {
    dataDir,
    exists: dataDir ? fs.existsSync(dataDir) : false
  }
}));
"#,
    )?;
    Ok(())
}

fn write_settings_probe_extension_source(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "settings-probe",
            "displayName": "Settings Probe",
            "capabilities": [
                {
                    "capability": "dialogue.generate",
                    "methods": ["generate"]
                }
            ],
            "settings": {
                "fields": [
                    {
                        "key": "provider",
                        "type": "select",
                        "label": "Provider",
                        "options": [
                            { "value": "gemini", "label": "Gemini" },
                            { "value": "openai-compatible", "label": "OpenAI compatible" }
                        ],
                        "default": "openai-compatible"
                    },
                    {
                        "key": "timeoutMs",
                        "type": "number",
                        "label": "Timeout",
                        "default": 30000,
                        "min": 1000,
                        "max": 120000
                    },
                    {
                        "key": "gemini.apiKey",
                        "type": "secret",
                        "label": "Gemini API key",
                        "visibleWhen": { "key": "provider", "equals": "gemini" }
                    }
                ]
            },
            "process": {
                "command": "node",
                "args": ["settings.js"]
            }
        }))?,
    )?;
    fs::write(
        root.join("settings.js"),
        r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const settings = JSON.parse(process.env.YUUKEI_EXTENSION_SETTINGS_JSON ?? "{}");
const ok = settings.provider === "gemini"
  && settings.timeoutMs === 45000
  && settings["gemini.apiKey"] === "super-secret";
process.stdout.write(JSON.stringify({
  invocationId: input.id,
  extensionId: "settings-probe",
  capability: input.capability,
  output: { speak: true, text: ok ? "settings ok" : "settings missing" },
  metadata: { provider: settings.provider, hasGeminiApiKey: Boolean(settings["gemini.apiKey"]) }
}));
"#,
    )?;
    Ok(())
}

fn write_yuukei_intelligence_extension_source(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    copy_dir_for_test(
        &repo_root
            .join("packages")
            .join("yuukei-intelligence")
            .join("src"),
        &root.join("src"),
    )?;
    fs::write(
        root.join("fetch-stub.mjs"),
        r#"
globalThis.fetch = async (url, init) => {
  const body = JSON.parse(init.body);
  if (String(url) !== "http://stub.local/v1/chat/completions") {
    throw new Error(`unexpected URL: ${url}`);
  }
  if (body.model !== "stub-model") {
    throw new Error(`unexpected model: ${body.model}`);
  }
  const bodyText = JSON.stringify(body);
  if (bodyText.includes("ユーザーは今日のお出かけに行けますか？")) {
    return new Response(JSON.stringify({
      usage: { prompt_tokens: 21, completion_tokens: 7 },
      choices: [
        {
          message: {
            content: "{\"choice\":\"いいえ\",\"confidence\":0.9}"
          }
        }
      ]
    }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  }
  if (bodyText.includes("memory.index provider") || bodyText.includes("newFacts")) {
    return new Response(JSON.stringify({
      usage: { prompt_tokens: 34, completion_tokens: 10 },
      choices: [
        {
          message: {
            content: "{\"diary\":\"ユーザーは唐揚げの話をした。\",\"newFacts\":[\"唐揚げが好き。\"]}"
          }
        }
      ]
    }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  }
  if (bodyText.includes("唐揚げのこと覚えてる？")) {
    if (!bodyText.includes("唐揚げが好き。")) {
      throw new Error("prompt did not include retrieved memory");
    }
    return new Response(JSON.stringify({
      usage: { prompt_tokens: 55, completion_tokens: 12 },
      choices: [
        {
          message: {
            content: "{\"speak\":true,\"text\":\"覚えています。唐揚げが好きなんですよね。\"}"
          }
        }
      ]
    }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  }
  if (!bodyText.includes("何か言う？")) {
    throw new Error("prompt did not include source text");
  }
  return new Response(JSON.stringify({
    usage: { prompt_tokens: 13, completion_tokens: 5 },
    choices: [
      {
        message: {
          content: "{\"speak\":true,\"text\":\"スタブから返事します。\",\"expression\":\"smile\"}"
        }
      }
    ]
  }), {
    status: 200,
    headers: { "content-type": "application/json" }
  });
};
"#,
    )?;
    fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "id": "yuukei-intelligence",
            "displayName": "Yuukei Intelligence",
            "runtime": "process",
            "permissions": {
                "broadEventSubscription": false
            },
            "capabilities": [
                {
                    "capability": "dialogue.generate",
                    "methods": ["generate"]
                },
                {
                    "capability": "dialogue.interpret",
                    "methods": ["interpret"]
                },
                {
                    "capability": "memory.index",
                    "methods": ["index"]
                },
                {
                    "capability": "memory.retrieve",
                    "methods": ["retrieve"]
                }
            ],
            "process": {
                "command": "node",
                "args": ["--import", "./fetch-stub.mjs", "src/main.mjs"],
                "timeoutMs": 5000
            },
            "config": {
                "provider": "openai-compatible",
                "timeoutMs": 1000,
                "openaiCompatible": {
                    "baseUrl": "http://stub.local/v1",
                    "model": "stub-model"
                }
            }
        }))?,
    )?;
    Ok(())
}

fn copy_dir_for_test(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::metadata(&source_path)?;
        if metadata.is_dir() {
            copy_dir_for_test(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

// リリース判定基準(ROADMAP)の「あんぱんシナリオがdefault packで動く」を、
// 同梱の実packで恒久的に保証する。
#[tokio::test]
async fn bundled_default_pack_runs_anpan_scenario() -> Result<()> {
    let workspace = tempdir()?;
    let data = tempdir()?;
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("packs")
        .join("default-yuukei");
    copy_dir_for_test(
        &bundled,
        &workspace.path().join("packs").join("default-yuukei"),
    )?;
    let env = test_env(workspace.path(), data.path());
    let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
    runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .await?;

    let download_commands = runtime
        .emit_desktop_download_completed(DesktopDownloadObservation {
            file_name: "あんぱん.png".to_string(),
            file_category: DownloadFileCategory::Image,
        })
        .await?;
    assert!(
        download_commands
            .iter()
            .filter_map(|command| command.payload.get("text").and_then(Value::as_str))
            .any(|text| text.contains("あんぱん.png")),
        "download scene should mention the file"
    );

    let commands = runtime
        .emit_desktop_folder_transition(DesktopFolderTransition {
            category: DesktopFolderCategory::Downloads,
            app: "finder".to_string(),
        })
        .await?;

    let texts = commands
        .iter()
        .filter(|command| command.kind == "dialogue.say")
        .filter_map(|command| command.payload.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        texts.iter().any(|text| text.contains("あんぱん.png")),
        "expected the anpan scene to reference the downloaded file, got: {texts:?}"
    );
    Ok(())
}
