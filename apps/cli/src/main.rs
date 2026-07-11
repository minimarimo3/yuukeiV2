mod menu;

use std::{
    env,
    io::{self, BufRead, Write},
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use menu::{
    menu_lines, transition, MenuAction, MenuActor, MenuData, MenuExtension, MenuHitZone, MenuState,
};
use tokio::task::JoinHandle;
use yuukei_device_host::{
    cli_surface_session, ActorSurfaceAssetCatalog, AvatarGestureInput, AvatarGesturePoke,
    AvatarGestureScreen, LocalYuukeiRuntime, RuntimePaths, WorldPackSelectionState, CLI_SURFACE_ID,
};
use yuukei_protocol::{ResidentSnapshot, RuntimeCommand};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CliMode {
    Repl,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputMode {
    Human,
    Jsonl,
}

impl OutputMode {
    fn from_environment() -> Self {
        match env::var("YUUKEI_CLI_OUTPUT").as_deref() {
            Ok("jsonl") => Self::Jsonl,
            _ => Self::Human,
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::Human => Self::Jsonl,
            Self::Jsonl => Self::Human,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Jsonl => "jsonl",
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        if error.to_string().starts_with("unknown argument:") {
            print_usage(&mut io::stderr());
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    match parse_args(env::args().skip(1)) {
        Ok(CliMode::Help) => {
            print_usage(&mut io::stdout());
            Ok(())
        }
        Ok(CliMode::Repl) => run_repl().await,
        Err(error) => Err(error),
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliMode> {
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(CliMode::Repl),
        [flag] if flag == "-h" || flag == "--help" => Ok(CliMode::Help),
        [flag, ..] => bail!("unknown argument: {flag}"),
    }
}

async fn run_repl() -> Result<()> {
    let presence_enabled = env::var("YUUKEI_CLI_PRESENCE").as_deref() == Ok("1");
    let mut runtime = open_attached_default().await?;
    let mut presence_loop = presence_enabled.then(|| runtime.spawn_presence_loop());
    let mut state = MenuState::Top;
    let mut output_mode = OutputMode::from_environment();
    let mut command_history = Vec::new();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();

    loop {
        let data = current_menu_data(&runtime)?;
        print_menu(&state, &data);
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end_matches(&['\r', '\n'][..]);
        let result = transition(state.clone(), line, &data);
        state = result.next_state;
        if let Some(error) = result.error {
            eprintln!("error: {error}");
            continue;
        }
        let Some(action) = result.action else {
            continue;
        };
        match execute_action(
            action,
            &mut runtime,
            &mut presence_loop,
            presence_enabled,
            &mut command_history,
            &mut output_mode,
        )
        .await
        {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => eprintln!("error: {error:#}"),
        }
    }

    if let Some(handle) = presence_loop {
        handle.abort();
    }
    Ok(())
}

async fn open_attached_default() -> Result<LocalYuukeiRuntime> {
    let runtime = LocalYuukeiRuntime::open_default()
        .await
        .context("failed to open Yuukei local runtime")?;
    attach_runtime(runtime).await
}

async fn attach_runtime(runtime: LocalYuukeiRuntime) -> Result<LocalYuukeiRuntime> {
    runtime
        .attach_surface(cli_surface_session(runtime.device_id()))
        .await
        .context("failed to attach CLI surface")?;
    runtime
        .emit_app_startup()
        .await
        .context("failed to emit app startup")?;
    Ok(runtime)
}

async fn execute_action(
    action: MenuAction,
    runtime: &mut LocalYuukeiRuntime,
    presence_loop: &mut Option<JoinHandle<()>>,
    presence_enabled: bool,
    command_history: &mut Vec<RuntimeCommand>,
    output_mode: &mut OutputMode,
) -> Result<bool> {
    match action {
        MenuAction::Quit => return Ok(true),
        MenuAction::SendConversation(text) => {
            let commands = runtime
                .send_conversation_text(CLI_SURFACE_ID, &text)
                .await
                .context("failed to send conversation text")?;
            record_commands(command_history, &commands, *output_mode)?;
        }
        MenuAction::SendPoke {
            actor_id,
            hit_zone_id,
            hit_zone_label,
        } => {
            let commands = runtime
                .send_avatar_gesture_poke(
                    CLI_SURFACE_ID,
                    AvatarGesturePoke {
                        actor_id,
                        hit_zone_id,
                        hit_zone_label,
                        hit_surface: Some("unknown".to_string()),
                        hit_bone: None,
                        input: AvatarGestureInput {
                            kind: "cli".to_string(),
                            button: "none".to_string(),
                        },
                        screen: AvatarGestureScreen { x: 0.0, y: 0.0 },
                    },
                )
                .await
                .context("failed to send avatar gesture poke")?;
            record_commands(command_history, &commands, *output_mode)?;
        }
        MenuAction::SendGrab { actor_id } => {
            let commands = runtime
                .send_avatar_gesture_grab(CLI_SURFACE_ID, &actor_id)
                .await
                .context("failed to send avatar gesture grab")?;
            record_commands(command_history, &commands, *output_mode)?;
        }
        MenuAction::SendDrop {
            actor_id,
            moved_distance,
        } => {
            let commands = runtime
                .send_avatar_gesture_drop(CLI_SURFACE_ID, &actor_id, moved_distance)
                .await
                .context("failed to send avatar gesture drop")?;
            record_commands(command_history, &commands, *output_mode)?;
        }
        MenuAction::ShowSnapshot => print_snapshot(&runtime.snapshot()?, *output_mode)?,
        MenuAction::ShowHistory => print_commands(command_history, *output_mode)?,
        MenuAction::ShowWorldPackStatus => {
            print_world_pack_status(&runtime.world_pack_status(), *output_mode)?;
        }
        MenuAction::ShowPaths => print_paths(runtime.paths(), *output_mode)?,
        MenuAction::ExportEventLog(path) => {
            let path = PathBuf::from(path);
            let exported = runtime
                .export_event_log_jsonl(&path)
                .context("failed to export event log")?;
            print_exported_event_log(exported, &path, *output_mode)?;
        }
        MenuAction::ToggleOutputMode => {
            *output_mode = output_mode.toggled();
            print_output_mode(*output_mode)?;
        }
        MenuAction::SelectWorldPack(path) => {
            let new_runtime = LocalYuukeiRuntime::select_world_pack_directory(PathBuf::from(path))
                .await
                .context("failed to select World Pack")?;
            replace_runtime(
                runtime,
                attach_runtime(new_runtime).await?,
                presence_loop,
                presence_enabled,
            );
            command_history.clear();
            print_world_pack_status(&runtime.world_pack_status(), *output_mode)?;
        }
        MenuAction::ResetWorldPack => {
            let new_runtime = LocalYuukeiRuntime::reset_world_pack_to_default()
                .await
                .context("failed to reset World Pack")?;
            replace_runtime(
                runtime,
                attach_runtime(new_runtime).await?,
                presence_loop,
                presence_enabled,
            );
            command_history.clear();
            print_world_pack_status(&runtime.world_pack_status(), *output_mode)?;
        }
        MenuAction::InstallExtension(path) => {
            LocalYuukeiRuntime::install_extension_directory(PathBuf::from(path))
                .context("failed to install Extension")?;
            replace_runtime(
                runtime,
                open_attached_default().await?,
                presence_loop,
                presence_enabled,
            );
            command_history.clear();
            print_extensions(*output_mode)?;
        }
        MenuAction::SetExtensionEnabled {
            extension_id,
            enabled,
        } => {
            LocalYuukeiRuntime::set_extension_enabled(&extension_id, enabled)
                .with_context(|| format!("failed to update Extension {extension_id}"))?;
            replace_runtime(
                runtime,
                open_attached_default().await?,
                presence_loop,
                presence_enabled,
            );
            command_history.clear();
            print_extensions(*output_mode)?;
        }
    }
    Ok(false)
}

fn replace_runtime(
    runtime: &mut LocalYuukeiRuntime,
    new_runtime: LocalYuukeiRuntime,
    presence_loop: &mut Option<JoinHandle<()>>,
    presence_enabled: bool,
) {
    if let Some(handle) = presence_loop.take() {
        handle.abort();
    }
    if presence_enabled {
        *presence_loop = Some(new_runtime.spawn_presence_loop());
    }
    *runtime = new_runtime;
}

fn current_menu_data(runtime: &LocalYuukeiRuntime) -> Result<MenuData> {
    let ActorSurfaceAssetCatalog { actors, .. } = runtime.actor_surface_assets();
    let actors = actors
        .into_iter()
        .map(|actor| MenuActor {
            id: actor.actor_id,
            display_name: actor.display_name,
            hit_zones: actor
                .renderer
                .map(|renderer| {
                    renderer
                        .hit_zones
                        .into_iter()
                        .map(|hit_zone| MenuHitZone {
                            id: hit_zone.id,
                            label: hit_zone.label,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    let extensions = LocalYuukeiRuntime::extension_settings_state()?
        .installed
        .into_iter()
        .map(|extension| MenuExtension {
            id: extension.extension_id,
            display_name: extension.display_name,
            enabled: extension.enabled,
        })
        .collect();
    Ok(MenuData { actors, extensions })
}

fn print_menu(state: &MenuState, data: &MenuData) {
    for line in menu_lines(state, data) {
        eprintln!("{line}");
    }
    eprint!("> ");
    let _ = io::stderr().flush();
}

fn record_commands(
    command_history: &mut Vec<RuntimeCommand>,
    commands: &[RuntimeCommand],
    output_mode: OutputMode,
) -> Result<()> {
    command_history.extend(commands.iter().cloned());
    print_commands(commands, output_mode)
}

fn print_commands(commands: &[RuntimeCommand], output_mode: OutputMode) -> Result<()> {
    if commands.is_empty() {
        if output_mode == OutputMode::Human {
            println!("コマンドは発行されませんでした。");
        }
        return Ok(());
    }
    match output_mode {
        OutputMode::Human => {
            for command in commands {
                println!("{}", command_label(command));
                if command.kind == "dialogue.say" {
                    if let Some(text) = command.payload.get("text").and_then(|value| value.as_str())
                    {
                        println!("  \"{text}\"");
                    }
                }
            }
        }
        OutputMode::Jsonl => {
            for command in commands {
                println!("{}", serde_json::to_string(command)?);
            }
        }
    }
    Ok(())
}

fn print_snapshot(snapshot: &ResidentSnapshot, output_mode: OutputMode) -> Result<()> {
    match output_mode {
        OutputMode::Human => print_snapshot_summary(snapshot),
        OutputMode::Jsonl => println!("{}", serde_json::to_string(snapshot)?),
    }
    Ok(())
}

fn print_world_pack_status(
    status: &WorldPackSelectionState,
    output_mode: OutputMode,
) -> Result<()> {
    match output_mode {
        OutputMode::Human => print_world_pack_status_summary(status),
        OutputMode::Jsonl => println!("{}", serde_json::to_string(status)?),
    }
    Ok(())
}

fn print_paths(paths: &RuntimePaths, output_mode: OutputMode) -> Result<()> {
    match output_mode {
        OutputMode::Human => {
            println!("Event log: {}", paths.event_log_path.display());
            println!("App log: {}", paths.app_log_path.display());
            println!("World root: {}", paths.world_root.display());
            println!("Extension root: {}", paths.extension_root.display());
        }
        OutputMode::Jsonl => println!(
            "{}",
            serde_json::json!({
                "eventLogPath": paths.event_log_path,
                "appLogPath": paths.app_log_path,
                "worldRoot": paths.world_root,
                "extensionRoot": paths.extension_root,
            })
        ),
    }
    Ok(())
}

fn print_exported_event_log(
    exported: usize,
    path: &PathBuf,
    output_mode: OutputMode,
) -> Result<()> {
    match output_mode {
        OutputMode::Human => println!("{exported} records exported to {}.", path.display()),
        OutputMode::Jsonl => println!(
            "{}",
            serde_json::json!({ "exported": exported, "path": path })
        ),
    }
    Ok(())
}

fn print_extensions(output_mode: OutputMode) -> Result<()> {
    let extensions = LocalYuukeiRuntime::extension_settings_state()?;
    match output_mode {
        OutputMode::Human => {
            for extension in extensions.installed {
                let enabled = if extension.enabled {
                    "有効"
                } else {
                    "無効"
                };
                println!("{} ({})", extension.display_name, enabled);
            }
        }
        OutputMode::Jsonl => println!("{}", serde_json::to_string(&extensions)?),
    }
    Ok(())
}

fn print_output_mode(output_mode: OutputMode) -> Result<()> {
    match output_mode {
        OutputMode::Human => println!("出力モード: {}", output_mode.name()),
        OutputMode::Jsonl => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({ "outputMode": output_mode.name() }))?
        ),
    }
    Ok(())
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

fn print_world_pack_status_summary(status: &WorldPackSelectionState) {
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
    if status.daihon_diagnostics.is_empty() {
        return;
    }
    println!("Daihon errors: {}", status.daihon_diagnostics.len());
    for diagnostic in status.daihon_diagnostics.iter().take(4) {
        println!(
            "  - {} / {} / {}",
            diagnostic.code,
            diagnostic.script_path.as_deref().unwrap_or("unknown"),
            diagnostic.message
        );
    }
    if status.daihon_diagnostics.len() > 4 {
        println!("  ... {} more", status.daihon_diagnostics.len() - 4);
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

fn print_usage(output: &mut impl Write) {
    let _ = writeln!(
        output,
        "Usage:\n  yuukei-cli-surface\n  yuukei-cli-surface --help"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_start_the_repl() -> Result<()> {
        assert_eq!(parse_args(Vec::new())?, CliMode::Repl);
        Ok(())
    }

    #[test]
    fn help_flags_are_the_only_accepted_flags() -> Result<()> {
        assert_eq!(parse_args(vec!["-h".to_string()])?, CliMode::Help);
        assert_eq!(parse_args(vec!["--help".to_string()])?, CliMode::Help);
        assert!(parse_args(vec!["--say".to_string(), "hello".to_string()]).is_err());
        Ok(())
    }

    #[test]
    fn jsonl_output_is_only_enabled_by_its_exact_environment_value() {
        assert_eq!(OutputMode::Jsonl.name(), "jsonl");
        assert_eq!(OutputMode::Human.toggled(), OutputMode::Jsonl);
    }
}
