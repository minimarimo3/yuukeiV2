use std::{env, path::PathBuf};

use anyhow::{bail, Context, Result};
use dialoguer::{theme::ColorfulTheme, Input, Select};
use yuukei_device_host::{
    cli_surface_session, LocalYuukeiRuntime, RuntimePaths, WorldPackSelectionState, CLI_SURFACE_ID,
};
use yuukei_protocol::{ResidentSnapshot, RuntimeCommand};

#[derive(Debug, Clone, Eq, PartialEq)]
enum CliMode {
    Wizard,
    Say(String),
    Snapshot,
    ExportEvents(PathBuf),
    SelectWorldPack(PathBuf),
    ResetWorldPack,
    Help,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = parse_args(env::args().skip(1))?;
    if mode == CliMode::Help {
        print_usage();
        return Ok(());
    }

    let runtime = match &mode {
        CliMode::SelectWorldPack(path) => LocalYuukeiRuntime::select_world_pack_directory(path)
            .await
            .context("failed to select World Pack")?,
        CliMode::ResetWorldPack => LocalYuukeiRuntime::reset_world_pack_to_default()
            .await
            .context("failed to reset World Pack")?,
        _ => LocalYuukeiRuntime::open_default()
            .await
            .context("failed to open Yuukei local runtime")?,
    };
    runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .await
        .context("failed to attach CLI surface")?;
    runtime
        .emit_app_startup()
        .await
        .context("failed to emit app startup")?;
    let snapshot = runtime.snapshot()?;

    match mode {
        CliMode::Wizard => run_wizard(runtime, snapshot).await,
        CliMode::Say(text) => {
            if text.trim().is_empty() {
                bail!("--say requires non-empty text");
            }
            let commands = runtime
                .send_conversation_text(CLI_SURFACE_ID, text.trim())
                .await
                .context("failed to send conversation text")?;
            println!("{}", serde_json::to_string_pretty(&commands)?);
            Ok(())
        }
        CliMode::Snapshot => {
            println!("{}", serde_json::to_string_pretty(&runtime.snapshot()?)?);
            Ok(())
        }
        CliMode::ExportEvents(path) => {
            let exported = runtime
                .export_event_log_jsonl(&path)
                .context("failed to export event log")?;
            println!(
                "{}",
                serde_json::json!({
                    "exported": exported,
                    "path": path.display().to_string()
                })
            );
            Ok(())
        }
        CliMode::SelectWorldPack(_) | CliMode::ResetWorldPack => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "snapshot": runtime.snapshot()?,
                    "worldPackStatus": runtime.world_pack_status()
                }))?
            );
            Ok(())
        }
        CliMode::Help => unreachable!("handled before runtime startup"),
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliMode> {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(CliMode::Wizard);
    }

    match args[0].as_str() {
        "--say" => {
            let Some(text) = args.get(1) else {
                bail!("missing text after --say");
            };
            Ok(CliMode::Say(text.clone()))
        }
        "--snapshot" => Ok(CliMode::Snapshot),
        "--export-events" => {
            let Some(path) = args.get(1) else {
                bail!("missing path after --export-events");
            };
            Ok(CliMode::ExportEvents(PathBuf::from(path)))
        }
        "--world-pack" => {
            let Some(path) = args.get(1) else {
                bail!("missing path after --world-pack");
            };
            Ok(CliMode::SelectWorldPack(PathBuf::from(path)))
        }
        "--reset-world-pack" => Ok(CliMode::ResetWorldPack),
        "-h" | "--help" => Ok(CliMode::Help),
        other => bail!("unknown argument: {other}"),
    }
}

async fn run_wizard(mut runtime: LocalYuukeiRuntime, snapshot: ResidentSnapshot) -> Result<()> {
    let theme = ColorfulTheme::default();
    let mut command_history: Vec<RuntimeCommand> = Vec::new();
    let mut presence_loop = runtime.spawn_presence_loop();

    println!("Yuukei CLI Surface");
    println!("Surface: {CLI_SURFACE_ID}");
    print_paths(runtime.paths());
    print_world_pack_status(&runtime.world_pack_status());
    print_snapshot_summary(&snapshot);

    loop {
        let action = select_action(&theme)?;
        match action {
            WizardAction::Talk => {
                let text = Input::<String>::with_theme(&theme)
                    .with_prompt("セリフを入力")
                    .allow_empty(false)
                    .interact_text()?;
                let commands = runtime
                    .send_conversation_text(CLI_SURFACE_ID, text.trim())
                    .await
                    .context("failed to send conversation text")?;
                print_commands(&commands);
                command_history.splice(0..0, commands);
            }
            WizardAction::Snapshot => {
                let snapshot = runtime.snapshot()?;
                print_snapshot_summary(&snapshot);
            }
            WizardAction::CommandHistory => {
                show_command_history_page(&theme, &command_history)?;
            }
            WizardAction::ExportEvents => {
                let default_path = runtime.paths().data_dir.join("events-export.jsonl");
                let path = Input::<String>::with_theme(&theme)
                    .with_prompt("書き出し先")
                    .default(default_path.display().to_string())
                    .interact_text()?;
                let exported = runtime.export_event_log_jsonl(PathBuf::from(path.trim()))?;
                println!("{exported} records exported.");
            }
            WizardAction::Paths => {
                print_paths(runtime.paths());
                print_world_pack_status(&runtime.world_pack_status());
            }
            WizardAction::SelectWorldPack => {
                let path = Input::<String>::with_theme(&theme)
                    .with_prompt("World Pack ディレクトリ")
                    .allow_empty(false)
                    .interact_text()?;
                runtime =
                    LocalYuukeiRuntime::select_world_pack_directory(PathBuf::from(path.trim()))
                        .await
                        .context("failed to select World Pack")?;
                runtime
                    .attach_surface(cli_surface_session(runtime.device_id()))
                    .await?;
                runtime.emit_app_startup().await?;
                let snapshot = runtime.snapshot()?;
                presence_loop.abort();
                presence_loop = runtime.spawn_presence_loop();
                command_history.clear();
                print_world_pack_status(&runtime.world_pack_status());
                print_snapshot_summary(&snapshot);
            }
            WizardAction::ResetWorldPack => {
                runtime = LocalYuukeiRuntime::reset_world_pack_to_default()
                    .await
                    .context("failed to reset World Pack")?;
                runtime
                    .attach_surface(cli_surface_session(runtime.device_id()))
                    .await?;
                runtime.emit_app_startup().await?;
                let snapshot = runtime.snapshot()?;
                presence_loop.abort();
                presence_loop = runtime.spawn_presence_loop();
                command_history.clear();
                print_world_pack_status(&runtime.world_pack_status());
                print_snapshot_summary(&snapshot);
            }
            WizardAction::Quit => {
                presence_loop.abort();
                println!("CLI Surface を終了します。");
                return Ok(());
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum WizardAction {
    Talk,
    Snapshot,
    CommandHistory,
    ExportEvents,
    Paths,
    SelectWorldPack,
    ResetWorldPack,
    Quit,
}

fn select_action(theme: &ColorfulTheme) -> Result<WizardAction> {
    let actions = [
        ("話しかける", WizardAction::Talk),
        ("状態を見る", WizardAction::Snapshot),
        ("コマンド履歴を見る", WizardAction::CommandHistory),
        ("イベントログを書き出す", WizardAction::ExportEvents),
        ("ログファイルの場所を見る", WizardAction::Paths),
        ("World Packを選ぶ", WizardAction::SelectWorldPack),
        ("Default World Packに戻す", WizardAction::ResetWorldPack),
        ("終了", WizardAction::Quit),
    ];
    let labels = actions.iter().map(|(label, _)| *label).collect::<Vec<_>>();
    let selected = Select::with_theme(theme)
        .with_prompt("実行する項目")
        .items(&labels)
        .default(0)
        .interact()?;
    Ok(actions[selected].1)
}

fn show_command_history_page(theme: &ColorfulTheme, commands: &[RuntimeCommand]) -> Result<()> {
    if commands.is_empty() {
        println!("まだコマンドはありません。");
        return Ok(());
    }

    let mut labels = commands.iter().map(command_label).collect::<Vec<_>>();
    labels.push("戻る".to_string());

    loop {
        let selected = Select::with_theme(theme)
            .with_prompt("詳細を見るコマンド")
            .items(&labels)
            .default(0)
            .interact()?;
        if selected == commands.len() {
            return Ok(());
        }
        println!("{}", serde_json::to_string_pretty(&commands[selected])?);
    }
}

fn print_paths(paths: &RuntimePaths) {
    println!("Event log: {}", paths.event_log_path.display());
    println!("App log: {}", paths.app_log_path.display());
    println!("World root: {}", paths.world_root.display());
    println!("Extension root: {}", paths.extension_root.display());
}

fn print_world_pack_status(status: &WorldPackSelectionState) {
    println!(
        "World Pack: {} / install: {} / configured: {}",
        status.active_install.world_pack_id,
        status.running_install_id,
        status.configured_install_id
    );
    if status.fallback_active {
        println!(
            "World Pack fallback: {}",
            status.last_load_error.as_deref().unwrap_or("unknown error")
        );
    }
}

fn print_snapshot_summary(snapshot: &ResidentSnapshot) {
    println!(
        "Resident: {} / world: {} / active surface: {}",
        snapshot.resident_id,
        snapshot.world_pack_id,
        snapshot.active_surface_id.as_deref().unwrap_or("none")
    );
    for (actor_id, actor) in &snapshot.actors {
        println!(
            "- {actor_id}: {} / {} / {}",
            actor.display_name, actor.expression, actor.motion
        );
        if let Some(bubble) = &actor.bubble {
            println!("  \"{bubble}\"");
        }
    }
}

fn print_commands(commands: &[RuntimeCommand]) {
    if commands.is_empty() {
        println!("コマンドは発行されませんでした。");
        return;
    }
    for command in commands {
        println!("{}", command_label(command));
        if command.kind == "dialogue.say" {
            if let Some(text) = command.payload.get("text").and_then(|value| value.as_str()) {
                println!("  \"{text}\"");
            }
        }
    }
}

fn command_label(command: &RuntimeCommand) -> String {
    let actor = command
        .target
        .as_ref()
        .and_then(|target| target.actor_id.as_deref())
        .unwrap_or("target");
    let text = command
        .payload
        .get("text")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if text.is_empty() {
        format!("{} / {} / {}", command.kind, command.id, actor)
    } else {
        format!("{} / {} / {}", command.kind, actor, text)
    }
}

fn print_usage() {
    println!(
        "Usage:
  yuukei-cli-surface
  yuukei-cli-surface --say <text>
  yuukei-cli-surface --snapshot
  yuukei-cli-surface --export-events <path>
  yuukei-cli-surface --world-pack <dir>
  yuukei-cli-surface --reset-world-pack"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_starts_wizard() -> Result<()> {
        assert_eq!(parse_args(Vec::new())?, CliMode::Wizard);
        Ok(())
    }

    #[test]
    fn say_mode_captures_text() -> Result<()> {
        assert_eq!(
            parse_args(vec!["--say".to_string(), "hello".to_string()])?,
            CliMode::Say("hello".to_string())
        );
        Ok(())
    }

    #[test]
    fn export_mode_captures_path() -> Result<()> {
        assert_eq!(
            parse_args(vec![
                "--export-events".to_string(),
                "target/events.jsonl".to_string()
            ])?,
            CliMode::ExportEvents(PathBuf::from("target/events.jsonl"))
        );
        Ok(())
    }

    #[test]
    fn world_pack_mode_captures_directory() -> Result<()> {
        assert_eq!(
            parse_args(vec!["--world-pack".to_string(), "packs/custom".to_string()])?,
            CliMode::SelectWorldPack(PathBuf::from("packs/custom"))
        );
        Ok(())
    }

    #[test]
    fn reset_world_pack_mode_is_supported() -> Result<()> {
        assert_eq!(
            parse_args(vec!["--reset-world-pack".to_string()])?,
            CliMode::ResetWorldPack
        );
        Ok(())
    }
}
