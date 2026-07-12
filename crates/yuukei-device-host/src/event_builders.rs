use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarGesturePoke {
    pub actor_id: String,
    pub hit_zone_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_zone_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_bone: Option<String>,
    pub input: AvatarGestureInput,
    pub screen: AvatarGestureScreen,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarGestureInput {
    pub kind: String,
    pub button: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarGestureScreen {
    pub x: f64,
    pub y: f64,
}
pub fn tauri_surface_session(device_id: &str) -> SurfaceSession {
    SurfaceSession {
        surface_id: TAURI_SURFACE_ID.to_string(),
        device_id: device_id.to_string(),
        kind: SurfaceKind::Desktop,
        active: true,
        capabilities: vec![
            "dialogue.say".to_string(),
            "avatar.expression".to_string(),
            "avatar.motion".to_string(),
            "avatar.gesture.poke".to_string(),
            "actor.place".to_string(),
            "screen.effect.start".to_string(),
            "screen.effect.stop".to_string(),
            "screen.dialogBurst.start".to_string(),
            "screen.dialogBurst.clear".to_string(),
        ],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Vrm),
            transparent: Some(true),
            accepts_input: Some(true),
        },
    }
}

pub fn cli_surface_session(device_id: &str) -> SurfaceSession {
    SurfaceSession {
        surface_id: CLI_SURFACE_ID.to_string(),
        device_id: device_id.to_string(),
        kind: SurfaceKind::Cli,
        active: true,
        capabilities: vec![
            "dialogue.say".to_string(),
            "conversation.text".to_string(),
            "wizard.select".to_string(),
        ],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Terminal),
            transparent: Some(false),
            accepts_input: Some(true),
        },
    }
}

pub fn build_conversation_text_event(
    resident_id: &str,
    device_id: &str,
    surface_id: &str,
    text: &str,
) -> RuntimeEvent {
    RuntimeEvent {
        id: new_id("evt"),
        kind: "conversation.text".to_string(),
        timestamp: now_timestamp(),
        source: "surface".to_string(),
        resident_id: resident_id.to_string(),
        payload: JsonMap::from([("text".to_string(), Value::String(text.to_string()))]),
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: None,
        privacy: None,
    }
}

pub fn build_conversation_choice_event(
    resident_id: &str,
    device_id: &str,
    surface_id: &str,
    choice_id: &str,
    choice: &str,
    index: usize,
) -> RuntimeEvent {
    RuntimeEvent {
        id: new_id("evt"),
        kind: "conversation.choice".to_string(),
        timestamp: now_timestamp(),
        source: "surface".to_string(),
        resident_id: resident_id.to_string(),
        payload: JsonMap::from([
            ("choiceId".to_string(), json!(choice_id)),
            ("choice".to_string(), json!(choice)),
            ("index".to_string(), json!(index)),
        ]),
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: None,
        privacy: None,
    }
}

pub fn build_avatar_gesture_poke_event(
    resident_id: &str,
    device_id: &str,
    surface_id: &str,
    gesture: AvatarGesturePoke,
) -> RuntimeEvent {
    let AvatarGesturePoke {
        actor_id,
        hit_zone_id,
        hit_zone_label,
        hit_surface,
        hit_bone,
        input,
        screen,
    } = gesture;
    let mut payload = JsonMap::from([
        ("actorId".to_string(), Value::String(actor_id.clone())),
        ("hitZoneId".to_string(), Value::String(hit_zone_id)),
        (
            "input".to_string(),
            json!({
                "kind": input.kind,
                "button": input.button,
            }),
        ),
        (
            "screen".to_string(),
            json!({
                "x": screen.x,
                "y": screen.y,
            }),
        ),
    ]);
    if let Some(label) = hit_zone_label {
        payload.insert("hitZoneLabel".to_string(), Value::String(label));
    }
    if let Some(surface) = hit_surface {
        payload.insert("hitSurface".to_string(), Value::String(surface));
    }
    if let Some(bone) = hit_bone {
        payload.insert("hitBone".to_string(), Value::String(bone));
    }

    RuntimeEvent {
        id: new_id("evt"),
        kind: "avatar.gesture.poke".to_string(),
        timestamp: now_timestamp(),
        source: "surface".to_string(),
        resident_id: resident_id.to_string(),
        payload,
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: Some(actor_id),
        privacy: None,
    }
}

pub fn build_avatar_drag_event(
    resident_id: &str,
    device_id: &str,
    surface_id: &str,
    actor_id: &str,
    kind: &str,
    moved_distance: Option<u64>,
) -> RuntimeEvent {
    debug_assert!(matches!(
        kind,
        "avatar.gesture.grab" | "avatar.gesture.drop"
    ));
    let mut payload = JsonMap::new();
    if let Some(distance) = moved_distance {
        payload.insert("movedDistance".to_string(), json!(distance));
    }
    RuntimeEvent {
        id: new_id("evt"),
        kind: kind.to_string(),
        timestamp: now_timestamp(),
        source: "surface".to_string(),
        resident_id: resident_id.to_string(),
        payload,
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: Some(actor_id.to_string()),
        privacy: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_event_uses_surface_boundary_fields() {
        let event =
            build_conversation_text_event("resident-test", "device-test", "surface-test", "hello");
        assert_eq!(event.kind, "conversation.text");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, "resident-test");
        assert_eq!(event.device_id.as_deref(), Some("device-test"));
        assert_eq!(event.surface_id.as_deref(), Some("surface-test"));
        assert_eq!(event.payload["text"], json!("hello"));
    }

    #[test]
    fn conversation_choice_event_uses_surface_boundary_fields() {
        let event = build_conversation_choice_event(
            "resident-test",
            "device-test",
            "surface-test",
            "choice-1",
            "見る",
            0,
        );
        assert_eq!(event.kind, "conversation.choice");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, "resident-test");
        assert_eq!(event.device_id.as_deref(), Some("device-test"));
        assert_eq!(event.surface_id.as_deref(), Some("surface-test"));
        assert_eq!(event.payload["choiceId"], json!("choice-1"));
        assert_eq!(event.payload["choice"], json!("見る"));
        assert_eq!(event.payload["index"], json!(0));
    }

    #[test]
    fn avatar_gesture_poke_event_uses_surface_boundary_fields() {
        let event = build_avatar_gesture_poke_event(
            "resident-test",
            "device-test",
            "surface-test",
            AvatarGesturePoke {
                actor_id: "yuukei".to_string(),
                hit_zone_id: "head".to_string(),
                hit_zone_label: Some("頭".to_string()),
                hit_surface: Some("face".to_string()),
                hit_bone: Some("head".to_string()),
                input: AvatarGestureInput {
                    kind: "pointer".to_string(),
                    button: "primary".to_string(),
                },
                screen: AvatarGestureScreen { x: 123.0, y: 456.0 },
            },
        );

        assert_eq!(event.kind, "avatar.gesture.poke");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, "resident-test");
        assert_eq!(event.device_id.as_deref(), Some("device-test"));
        assert_eq!(event.surface_id.as_deref(), Some("surface-test"));
        assert_eq!(event.actor_id.as_deref(), Some("yuukei"));
        assert_eq!(event.payload["actorId"], json!("yuukei"));
        assert_eq!(event.payload["hitZoneId"], json!("head"));
        assert_eq!(event.payload["hitZoneLabel"], json!("頭"));
        assert_eq!(event.payload["hitSurface"], json!("face"));
        assert_eq!(event.payload["hitBone"], json!("head"));
        assert_eq!(event.payload["input"]["kind"], json!("pointer"));
        assert_eq!(event.payload["input"]["button"], json!("primary"));
        assert_eq!(event.payload["screen"]["x"], json!(123.0));
        assert_eq!(event.payload["screen"]["y"], json!(456.0));
    }

    #[test]
    fn avatar_drag_events_keep_coordinates_out_of_the_payload() {
        let grab = build_avatar_drag_event(
            "resident-test",
            "device-test",
            "surface-test",
            "yuukei",
            "avatar.gesture.grab",
            None,
        );
        let drop = build_avatar_drag_event(
            "resident-test",
            "device-test",
            "surface-test",
            "yuukei",
            "avatar.gesture.drop",
            Some(184),
        );

        assert_eq!(grab.actor_id.as_deref(), Some("yuukei"));
        assert!(grab.payload.is_empty());
        assert_eq!(drop.actor_id.as_deref(), Some("yuukei"));
        assert_eq!(
            drop.payload,
            JsonMap::from([("movedDistance".to_string(), json!(184))])
        );
        assert!(!drop.payload.contains_key("x"));
        assert!(!drop.payload.contains_key("y"));
    }
}
