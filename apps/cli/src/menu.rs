#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MenuData {
    pub actors: Vec<MenuActor>,
    pub extensions: Vec<MenuExtension>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MenuActor {
    pub id: String,
    pub display_name: String,
    pub hit_zones: Vec<MenuHitZone>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MenuHitZone {
    pub id: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MenuExtension {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MenuState {
    Top,
    PokeActor,
    PokeHitZone { actor_id: String },
    GrabActor,
    DropActor,
    DropDistance { actor_id: String },
    ConversationText,
    WorldPack,
    WorldPackPath,
    Extensions,
    ExtensionPath,
    LogsAndPaths,
    EventLogPath,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MenuAction {
    Quit,
    SendPoke {
        actor_id: String,
        hit_zone_id: String,
        hit_zone_label: Option<String>,
    },
    SendGrab {
        actor_id: String,
    },
    SendDrop {
        actor_id: String,
        moved_distance: u64,
    },
    SendConversation(String),
    ShowSnapshot,
    ShowHistory,
    SelectWorldPack(String),
    ResetWorldPack,
    ShowWorldPackStatus,
    InstallExtension(String),
    SetExtensionEnabled {
        extension_id: String,
        enabled: bool,
    },
    ExportEventLog(String),
    ShowPaths,
    ToggleOutputMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    pub next_state: MenuState,
    pub action: Option<MenuAction>,
    pub error: Option<String>,
}

impl Transition {
    fn stay(state: MenuState) -> Self {
        Self {
            next_state: state,
            action: None,
            error: None,
        }
    }

    fn error(state: MenuState, message: impl Into<String>) -> Self {
        Self {
            next_state: state,
            action: None,
            error: Some(message.into()),
        }
    }

    fn action(next_state: MenuState, action: MenuAction) -> Self {
        Self {
            next_state,
            action: Some(action),
            error: None,
        }
    }
}

pub fn transition(state: MenuState, line: &str, data: &MenuData) -> Transition {
    match state {
        MenuState::Top => select_top(line),
        MenuState::PokeActor => select_actor(line, data, MenuState::PokeActor, |actor_id| {
            MenuState::PokeHitZone { actor_id }
        }),
        MenuState::PokeHitZone { actor_id } => select_hit_zone(line, data, &actor_id),
        MenuState::GrabActor => select_actor_action(line, data, MenuState::GrabActor, |actor_id| {
            MenuAction::SendGrab { actor_id }
        }),
        MenuState::DropActor => select_actor(line, data, MenuState::DropActor, |actor_id| {
            MenuState::DropDistance { actor_id }
        }),
        MenuState::DropDistance { actor_id } => select_distance(line, actor_id),
        MenuState::ConversationText => {
            select_text(line, MenuState::Top, MenuAction::SendConversation)
        }
        MenuState::WorldPack => select_world_pack(line),
        MenuState::WorldPackPath => {
            select_text(line, MenuState::WorldPack, MenuAction::SelectWorldPack)
        }
        MenuState::Extensions => select_extension(line, data),
        MenuState::ExtensionPath => {
            select_text(line, MenuState::Extensions, MenuAction::InstallExtension)
        }
        MenuState::LogsAndPaths => select_logs_and_paths(line),
        MenuState::EventLogPath => {
            select_text(line, MenuState::LogsAndPaths, MenuAction::ExportEventLog)
        }
    }
}

pub fn menu_lines(state: &MenuState, data: &MenuData) -> Vec<String> {
    match state {
        MenuState::Top => vec![
            "1 撫でる/つつく".to_string(),
            "2 つまむ".to_string(),
            "3 おろす".to_string(),
            "4 話しかける".to_string(),
            "5 状態を見る".to_string(),
            "6 コマンド履歴".to_string(),
            "7 World Pack".to_string(),
            "8 拡張機能".to_string(),
            "9 ログとパス".to_string(),
            "10 出力モード切替".to_string(),
            "0 終了".to_string(),
        ],
        MenuState::PokeActor => actor_lines("撫でる/つつく: アクターを選択", data),
        MenuState::PokeHitZone { actor_id } => hit_zone_lines(actor_id, data),
        MenuState::GrabActor => actor_lines("つまむ: アクターを選択", data),
        MenuState::DropActor => actor_lines("おろす: アクターを選択", data),
        MenuState::DropDistance { actor_id } => vec![
            format!("おろす: {actor_id} の移動距離を入力 (空行で戻る)"),
            "0 戻る".to_string(),
        ],
        MenuState::ConversationText => vec![
            "セリフを入力 (空行で戻る)".to_string(),
            "0 戻る".to_string(),
        ],
        MenuState::WorldPack => vec![
            "World Pack".to_string(),
            "1 選択 (パス入力)".to_string(),
            "2 リセット".to_string(),
            "3 状態表示".to_string(),
            "0 戻る".to_string(),
        ],
        MenuState::WorldPackPath => vec![
            "World Pack ディレクトリを入力 (空行で戻る)".to_string(),
            "0 戻る".to_string(),
        ],
        MenuState::Extensions => extension_lines(data),
        MenuState::ExtensionPath => vec![
            "Extension ディレクトリを入力 (空行で戻る)".to_string(),
            "0 戻る".to_string(),
        ],
        MenuState::LogsAndPaths => vec![
            "ログとパス".to_string(),
            "1 イベントログ書き出し (パス入力)".to_string(),
            "2 パス表示".to_string(),
            "0 戻る".to_string(),
        ],
        MenuState::EventLogPath => vec![
            "イベントログ書き出し先を入力 (空行で戻る)".to_string(),
            "0 戻る".to_string(),
        ],
    }
}

fn select_top(line: &str) -> Transition {
    if line.is_empty() {
        return Transition::stay(MenuState::Top);
    }
    match line {
        "0" => Transition::action(MenuState::Top, MenuAction::Quit),
        "1" => Transition::stay(MenuState::PokeActor),
        "2" => Transition::stay(MenuState::GrabActor),
        "3" => Transition::stay(MenuState::DropActor),
        "4" => Transition::stay(MenuState::ConversationText),
        "5" => Transition::action(MenuState::Top, MenuAction::ShowSnapshot),
        "6" => Transition::action(MenuState::Top, MenuAction::ShowHistory),
        "7" => Transition::stay(MenuState::WorldPack),
        "8" => Transition::stay(MenuState::Extensions),
        "9" => Transition::stay(MenuState::LogsAndPaths),
        "10" => Transition::action(MenuState::Top, MenuAction::ToggleOutputMode),
        _ => Transition::error(MenuState::Top, "不正な番号です。"),
    }
}

fn select_actor(
    line: &str,
    data: &MenuData,
    state: MenuState,
    next: impl FnOnce(String) -> MenuState,
) -> Transition {
    if line.is_empty() {
        return Transition::stay(state);
    }
    match numbered(line, &sorted_actors(data)) {
        Ok(None) => Transition::stay(MenuState::Top),
        Ok(Some(actor)) => Transition::stay(next(actor.id.clone())),
        Err(_) => Transition::error(state, "不正な番号です。"),
    }
}

fn select_actor_action(
    line: &str,
    data: &MenuData,
    state: MenuState,
    action: impl FnOnce(String) -> MenuAction,
) -> Transition {
    if line.is_empty() {
        return Transition::stay(state);
    }
    match numbered(line, &sorted_actors(data)) {
        Ok(None) => Transition::stay(MenuState::Top),
        Ok(Some(actor)) => Transition::action(MenuState::Top, action(actor.id.clone())),
        Err(_) => Transition::error(state, "不正な番号です。"),
    }
}

fn select_hit_zone(line: &str, data: &MenuData, actor_id: &str) -> Transition {
    let state = MenuState::PokeHitZone {
        actor_id: actor_id.to_string(),
    };
    if line.is_empty() {
        return Transition::stay(state);
    }
    let Some(actor) = sorted_actors(data)
        .into_iter()
        .find(|actor| actor.id == actor_id)
    else {
        return Transition::error(
            MenuState::PokeActor,
            "アクターが見つかりません。もう一度選んでください。",
        );
    };
    match numbered(line, &sorted_hit_zones(actor)) {
        Ok(None) => Transition::stay(MenuState::PokeActor),
        Ok(Some(hit_zone)) => Transition::action(
            MenuState::Top,
            MenuAction::SendPoke {
                actor_id: actor_id.to_string(),
                hit_zone_id: hit_zone.id.clone(),
                hit_zone_label: hit_zone.label.clone(),
            },
        ),
        Err(_) => Transition::error(state, "不正な番号です。"),
    }
}

fn select_distance(line: &str, actor_id: String) -> Transition {
    let state = MenuState::DropDistance {
        actor_id: actor_id.clone(),
    };
    if line.is_empty() {
        return Transition::stay(MenuState::DropActor);
    }
    if line == "0" {
        return Transition::stay(MenuState::DropActor);
    }
    match line.parse::<u64>() {
        Ok(moved_distance) => Transition::action(
            MenuState::Top,
            MenuAction::SendDrop {
                actor_id,
                moved_distance,
            },
        ),
        Err(_) => Transition::error(state, "移動距離は0以上の整数で入力してください。"),
    }
}

fn select_text(
    line: &str,
    parent: MenuState,
    action: impl FnOnce(String) -> MenuAction,
) -> Transition {
    if line.is_empty() || line == "0" {
        return Transition::stay(parent);
    }
    Transition::action(MenuState::Top, action(line.to_string()))
}

fn select_world_pack(line: &str) -> Transition {
    if line.is_empty() {
        return Transition::stay(MenuState::WorldPack);
    }
    match line {
        "0" => Transition::stay(MenuState::Top),
        "1" => Transition::stay(MenuState::WorldPackPath),
        "2" => Transition::action(MenuState::WorldPack, MenuAction::ResetWorldPack),
        "3" => Transition::action(MenuState::WorldPack, MenuAction::ShowWorldPackStatus),
        _ => Transition::error(MenuState::WorldPack, "不正な番号です。"),
    }
}

fn select_extension(line: &str, data: &MenuData) -> Transition {
    if line.is_empty() {
        return Transition::stay(MenuState::Extensions);
    }
    if line == "0" {
        return Transition::stay(MenuState::Top);
    }
    if line == "1" {
        return Transition::stay(MenuState::ExtensionPath);
    }
    match line
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(2))
    {
        Some(index) => match sorted_extensions(data).get(index) {
            Some(extension) => Transition::action(
                MenuState::Extensions,
                MenuAction::SetExtensionEnabled {
                    extension_id: extension.id.clone(),
                    enabled: !extension.enabled,
                },
            ),
            None => Transition::error(MenuState::Extensions, "不正な番号です。"),
        },
        None => Transition::error(MenuState::Extensions, "不正な番号です。"),
    }
}

fn select_logs_and_paths(line: &str) -> Transition {
    if line.is_empty() {
        return Transition::stay(MenuState::LogsAndPaths);
    }
    match line {
        "0" => Transition::stay(MenuState::Top),
        "1" => Transition::stay(MenuState::EventLogPath),
        "2" => Transition::action(MenuState::LogsAndPaths, MenuAction::ShowPaths),
        _ => Transition::error(MenuState::LogsAndPaths, "不正な番号です。"),
    }
}

fn sorted_actors(data: &MenuData) -> Vec<&MenuActor> {
    let mut actors = data.actors.iter().collect::<Vec<_>>();
    actors.sort_by(|a, b| a.id.cmp(&b.id));
    actors
}

fn sorted_hit_zones(actor: &MenuActor) -> Vec<&MenuHitZone> {
    let mut hit_zones = actor.hit_zones.iter().collect::<Vec<_>>();
    hit_zones.sort_by(|a, b| a.id.cmp(&b.id));
    hit_zones
}

fn sorted_extensions(data: &MenuData) -> Vec<&MenuExtension> {
    let mut extensions = data.extensions.iter().collect::<Vec<_>>();
    extensions.sort_by(|a, b| a.id.cmp(&b.id));
    extensions
}

fn numbered<'a, T>(line: &str, values: &[&'a T]) -> Result<Option<&'a T>, ()> {
    if line == "0" {
        return Ok(None);
    }
    let index = line
        .parse::<usize>()
        .map_err(|_| ())?
        .checked_sub(1)
        .ok_or(())?;
    values.get(index).copied().ok_or(()).map(Some)
}

fn actor_lines(title: &str, data: &MenuData) -> Vec<String> {
    let mut lines = vec![title.to_string()];
    lines.extend(
        sorted_actors(data)
            .into_iter()
            .enumerate()
            .map(|(index, actor)| format!("{} {} ({})", index + 1, actor.display_name, actor.id)),
    );
    lines.push("0 戻る".to_string());
    lines
}

fn hit_zone_lines(actor_id: &str, data: &MenuData) -> Vec<String> {
    let mut lines = vec![format!("撫でる/つつく: {actor_id} のヒットゾーンを選択")];
    if let Some(actor) = sorted_actors(data)
        .into_iter()
        .find(|actor| actor.id == actor_id)
    {
        lines.extend(
            sorted_hit_zones(actor)
                .into_iter()
                .enumerate()
                .map(|(index, hit_zone)| {
                    let label = hit_zone.label.as_deref().unwrap_or(&hit_zone.id);
                    format!("{} {} ({})", index + 1, label, hit_zone.id)
                }),
        );
    }
    lines.push("0 戻る".to_string());
    lines
}

fn extension_lines(data: &MenuData) -> Vec<String> {
    let mut lines = vec![
        "拡張機能".to_string(),
        "1 インストール (パス入力)".to_string(),
    ];
    lines.extend(
        sorted_extensions(data)
            .into_iter()
            .enumerate()
            .map(|(index, extension)| {
                let enabled = if extension.enabled {
                    "有効"
                } else {
                    "無効"
                };
                format!(
                    "{} {} ({}, {})",
                    index + 2,
                    extension.display_name,
                    extension.id,
                    enabled
                )
            }),
    );
    lines.push("0 戻る".to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu_data() -> MenuData {
        MenuData {
            actors: vec![
                MenuActor {
                    id: "yuukei".to_string(),
                    display_name: "夕景".to_string(),
                    hit_zones: vec![
                        MenuHitZone {
                            id: "hand".to_string(),
                            label: Some("手".to_string()),
                        },
                        MenuHitZone {
                            id: "head".to_string(),
                            label: Some("頭".to_string()),
                        },
                    ],
                },
                MenuActor {
                    id: "partner".to_string(),
                    display_name: "相手".to_string(),
                    hit_zones: vec![],
                },
            ],
            extensions: vec![
                MenuExtension {
                    id: "zeta".to_string(),
                    display_name: "Zeta".to_string(),
                    enabled: false,
                },
                MenuExtension {
                    id: "alpha".to_string(),
                    display_name: "Alpha".to_string(),
                    enabled: true,
                },
            ],
        }
    }

    #[test]
    fn poke_selection_uses_id_sorted_actor_and_hit_zone_numbers() {
        let data = menu_data();
        let actor = transition(MenuState::PokeActor, "2", &data);
        assert_eq!(
            actor.next_state,
            MenuState::PokeHitZone {
                actor_id: "yuukei".to_string()
            }
        );
        assert_eq!(actor.action, None);

        let hit_zone = transition(actor.next_state, "1", &data);
        assert_eq!(hit_zone.next_state, MenuState::Top);
        assert_eq!(
            hit_zone.action,
            Some(MenuAction::SendPoke {
                actor_id: "yuukei".to_string(),
                hit_zone_id: "hand".to_string(),
                hit_zone_label: Some("手".to_string()),
            })
        );
    }

    #[test]
    fn grab_and_drop_require_actor_then_nonnegative_distance() {
        let data = menu_data();
        let grab = transition(MenuState::GrabActor, "1", &data);
        assert_eq!(grab.next_state, MenuState::Top);
        assert_eq!(
            grab.action,
            Some(MenuAction::SendGrab {
                actor_id: "partner".to_string()
            })
        );

        let actor = transition(MenuState::DropActor, "2", &data);
        assert_eq!(
            actor.next_state,
            MenuState::DropDistance {
                actor_id: "yuukei".to_string()
            }
        );
        let invalid = transition(actor.next_state.clone(), "-1", &data);
        assert_eq!(invalid.next_state, actor.next_state);
        assert!(invalid.error.is_some());
        let drop = transition(actor.next_state, "42", &data);
        assert_eq!(drop.next_state, MenuState::Top);
        assert_eq!(
            drop.action,
            Some(MenuAction::SendDrop {
                actor_id: "yuukei".to_string(),
                moved_distance: 42,
            })
        );
    }

    #[test]
    fn invalid_menu_number_stays_and_value_empty_line_returns_to_parent() {
        let data = menu_data();
        let invalid = transition(MenuState::Top, "99", &data);
        assert_eq!(invalid.next_state, MenuState::Top);
        assert!(invalid.error.is_some());

        let empty_menu = transition(MenuState::Top, "", &data);
        assert_eq!(empty_menu.next_state, MenuState::Top);
        assert_eq!(empty_menu.action, None);
        assert_eq!(empty_menu.error, None);

        let cancelled = transition(MenuState::ConversationText, "", &data);
        assert_eq!(cancelled.next_state, MenuState::Top);
        assert_eq!(cancelled.action, None);
        assert_eq!(cancelled.error, None);
    }

    #[test]
    fn path_value_empty_line_returns_to_its_own_parent_menu() {
        let data = menu_data();
        assert_eq!(
            transition(MenuState::WorldPackPath, "", &data).next_state,
            MenuState::WorldPack
        );
        assert_eq!(
            transition(MenuState::ExtensionPath, "", &data).next_state,
            MenuState::Extensions
        );
        assert_eq!(
            transition(MenuState::EventLogPath, "", &data).next_state,
            MenuState::LogsAndPaths
        );
    }

    #[test]
    fn extension_numbers_follow_id_order_after_install_item() {
        let data = menu_data();
        let toggle = transition(MenuState::Extensions, "2", &data);
        assert_eq!(toggle.next_state, MenuState::Extensions);
        assert_eq!(
            toggle.action,
            Some(MenuAction::SetExtensionEnabled {
                extension_id: "alpha".to_string(),
                enabled: false,
            })
        );
    }
}
