use std::{env, path::PathBuf};

use anyhow::{bail, Context, Result};
use dialoguer::{theme::ColorfulTheme, Input, Select};
use yuukei_device_host::{cli_surface_session, LocalYuukeiRuntime, RuntimePaths, CLI_SURFACE_ID};
use yuukei_protocol::{ResidentSnapshot, RuntimeCommand};

#[derive(Debug, Clone, Eq, PartialEq)]
enum CliMode {
    Wizard,
    Say(String),
    Snapshot,
    ExportEvents(PathBuf),
    Help,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = parse_args(env::args().skip(1))?;
    if mode == CliMode::Help {
        print_usage();
        return Ok(());
    }

    let runtime = LocalYuukeiRuntime::open_default()
        .await
        .context("failed to open Yuukei local runtime")?;
    let snapshot = runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .context("failed to attach CLI surface")?;

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
        "-h" | "--help" => Ok(CliMode::Help),
        other => bail!("unknown argument: {other}"),
    }
}

async fn run_wizard(runtime: LocalYuukeiRuntime, snapshot: ResidentSnapshot) -> Result<()> {
    let theme = ColorfulTheme::default();
    let mut command_history: Vec<RuntimeCommand> = Vec::new();

    println!("Yuukei CLI Surface");
    println!("Surface: {CLI_SURFACE_ID}");
    print_paths(runtime.paths());
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
            }
            WizardAction::Quit => {
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
    Quit,
}

fn select_action(theme: &ColorfulTheme) -> Result<WizardAction> {
    let actions = [
        ("話しかける", WizardAction::Talk),
        ("状態を見る", WizardAction::Snapshot),
        ("コマンド履歴を見る", WizardAction::CommandHistory),
        ("イベントログを書き出す", WizardAction::ExportEvents),
        ("ログファイルの場所を見る", WizardAction::Paths),
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
  yuukei-cli-surface --export-events <path>"
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
}
