use super::*;
use std::collections::VecDeque;

#[test]
fn conversation_composer_opens_for_actor_and_closes() {
    let mut state = bubble_state(&["yuukei", "partner"]);

    open_conversation_composer_in_state(&mut state, "yuukei").expect("open composer");
    assert_eq!(
        state
            .snapshot()
            .conversation_composer
            .as_ref()
            .map(|composer| composer.actor_id.as_str()),
        Some("yuukei")
    );

    open_conversation_composer_in_state(&mut state, "partner").expect("replace composer");
    assert_eq!(
        state
            .snapshot()
            .conversation_composer
            .as_ref()
            .map(|composer| composer.actor_id.as_str()),
        Some("partner")
    );

    close_conversation_composer_in_state(&mut state);
    assert!(state.snapshot().conversation_composer.is_none());
}

#[test]
fn conversation_composer_rejects_unknown_actor() {
    let mut state = bubble_state(&["yuukei"]);

    let error = open_conversation_composer_in_state(&mut state, "missing")
        .expect_err("unknown actor must fail");

    assert!(error.contains("unknown actor"));
    assert!(state.snapshot().conversation_composer.is_none());
}

#[test]
fn conversation_composer_uses_actor_fallback_anchor_and_monitor() {
    let mut state = bubble_state(&["yuukei"]);
    state.monitors = vec![test_monitor(1000.0, 700.0)];
    let actor = state.actors.get_mut("yuukei").expect("actor");
    actor.bounds.x = -900.0;
    actor.bounds.y = -900.0;
    actor.anchor = StageAnchor {
        x: -500.0,
        y: -500.0,
        visible: false,
    };

    open_conversation_composer_in_state(&mut state, "yuukei").expect("open composer");
    let composer = state
        .snapshot()
        .conversation_composer
        .expect("composer snapshot");

    assert_eq!(composer.monitor_id, "monitor-0");
    assert!(composer.anchor.visible);
    assert!(composer.anchor.x >= 0.0);
    assert!(composer.anchor.y >= 0.0);
    assert!(composer.anchor.x <= 1000.0);
    assert!(composer.anchor.y <= 700.0);
}

#[test]
fn closing_conversation_composer_reports_only_real_changes() {
    let mut state = bubble_state(&["yuukei"]);
    open_conversation_composer_in_state(&mut state, "yuukei").expect("open composer");

    assert!(close_conversation_composer_in_state(&mut state));
    assert!(!close_conversation_composer_in_state(&mut state));
}

#[test]
fn actor_window_labels_hex_encode_actor_ids() {
    assert_eq!(actor_window_label("yuukei"), "actor-7975756b6569");
    assert_eq!(actor_window_label("partner"), "actor-706172746e6572");
    assert_eq!(
        actor_window_label("actor/with/slash"),
        "actor-6163746f722f776974682f736c617368"
    );
}

#[test]
fn actor_window_specs_include_only_renderable_actors() {
    let catalog = test_catalog(vec![
        test_actor("yuukei", true),
        test_actor("headless", false),
        test_actor("partner", true),
    ]);

    let specs = actor_window_specs(&catalog);

    assert_eq!(
        specs
            .iter()
            .map(|spec| (&spec.actor_id, spec.index))
            .collect::<Vec<_>>(),
        vec![(&"yuukei".to_string(), 0), (&"partner".to_string(), 1)]
    );
}

#[test]
fn actor_window_reconcile_closes_stale_and_creates_missing_windows() {
    let catalog = test_catalog(vec![
        test_actor("yuukei", true),
        test_actor("partner", true),
    ]);

    let reconcile = reconcile_actor_windows(
        vec![
            "settings".to_string(),
            actor_window_label("yuukei"),
            actor_window_label("old"),
        ],
        &catalog,
    );

    assert_eq!(reconcile.close_labels, vec![actor_window_label("old")]);
    assert_eq!(
        reconcile
            .create_specs
            .iter()
            .map(|spec| spec.actor_id.as_str())
            .collect::<Vec<_>>(),
        vec!["partner"]
    );
}

#[test]
fn actor_placement_avoids_existing_bounds_when_space_allows() {
    let monitors = vec![StageMonitor {
        id: "monitor-0".to_string(),
        label: stage_overlay_window_label(0),
        name: None,
        bounds: StageRect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 700.0,
        },
        scale_factor: 1.0,
    }];
    let size = actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT);
    let first = place_actor_window(0, &monitors, &[], size);
    let second = place_actor_window(1, &monitors, std::slice::from_ref(&first), size);

    assert!(!rects_overlap(&first, &second, 16.0));
}

#[test]
fn actor_layout_resolves_overlapping_current_bounds() {
    let monitors = vec![test_monitor(1000.0, 700.0)];
    let specs = test_specs(&["yuukei", "partner"]);
    let mut current_bounds = BTreeMap::new();
    current_bounds.insert(
        "yuukei".to_string(),
        StageRect {
            x: 80.0,
            y: 80.0,
            width: ACTOR_WINDOW_WIDTH,
            height: ACTOR_WINDOW_HEIGHT,
        },
    );
    current_bounds.insert(
        "partner".to_string(),
        StageRect {
            x: 90.0,
            y: 90.0,
            width: ACTOR_WINDOW_WIDTH,
            height: ACTOR_WINDOW_HEIGHT,
        },
    );

    let resolved = resolve_actor_window_layout(
        &specs,
        &current_bounds,
        &BTreeMap::new(),
        &monitors,
        actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
    );
    let first = resolved.get("yuukei").expect("yuukei bounds");
    let second = resolved.get("partner").expect("partner bounds");

    assert!(!rects_overlap(first, second, ACTOR_COLLISION_PADDING));
}

#[test]
fn actor_layout_spreads_three_or_four_actors_when_space_allows() {
    let monitors = vec![test_monitor(1900.0, 700.0)];
    let specs = test_specs(&["yuukei", "partner", "third", "fourth"]);
    let mut current_bounds = BTreeMap::new();
    for spec in &specs {
        current_bounds.insert(
            spec.actor_id.clone(),
            StageRect {
                x: 64.0,
                y: 64.0,
                width: ACTOR_WINDOW_WIDTH,
                height: ACTOR_WINDOW_HEIGHT,
            },
        );
    }

    let resolved = resolve_actor_window_layout(
        &specs,
        &current_bounds,
        &BTreeMap::new(),
        &monitors,
        actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
    );
    let bounds = specs
        .iter()
        .map(|spec| resolved.get(&spec.actor_id).expect("actor bounds"))
        .collect::<Vec<_>>();

    for (index, first) in bounds.iter().enumerate() {
        for second in bounds.iter().skip(index + 1) {
            assert!(!rects_overlap(first, second, ACTOR_COLLISION_PADDING));
        }
    }
}

#[test]
fn actor_layout_clamps_current_bounds_inside_monitor() {
    let monitor = test_monitor(1000.0, 700.0);
    let specs = test_specs(&["yuukei"]);
    let mut current_bounds = BTreeMap::new();
    current_bounds.insert(
        "yuukei".to_string(),
        StageRect {
            x: 920.0,
            y: 620.0,
            width: ACTOR_WINDOW_WIDTH,
            height: ACTOR_WINDOW_HEIGHT,
        },
    );

    let resolved = resolve_actor_window_layout(
        &specs,
        &current_bounds,
        &BTreeMap::new(),
        std::slice::from_ref(&monitor),
        actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
    );
    let bounds = resolved.get("yuukei").expect("yuukei bounds");

    assert!(bounds.x >= monitor.bounds.x + ACTOR_WINDOW_MARGIN);
    assert!(bounds.y >= monitor.bounds.y + ACTOR_WINDOW_MARGIN);
    assert!(
        bounds.x + bounds.width
            <= monitor.bounds.x + monitor.bounds.width - ACTOR_WINDOW_MARGIN + 0.5
    );
    assert!(
        bounds.y + bounds.height
            <= monitor.bounds.y + monitor.bounds.height - ACTOR_WINDOW_MARGIN + 0.5
    );
}

#[test]
fn actor_layout_restores_persisted_foot_anchor_with_current_scale_and_clamps_it() {
    let monitor = test_monitor(1000.0, 700.0);
    let specs = test_specs(&["yuukei"]);
    let persisted = BTreeMap::from([(
        "yuukei".to_string(),
        StageFootAnchor {
            x: 4_000.0,
            y: 3_000.0,
        },
    )]);

    let resolved = resolve_actor_window_layout(
        &specs,
        &BTreeMap::new(),
        &persisted,
        std::slice::from_ref(&monitor),
        actor_window_size(50),
    );
    let bounds = resolved.get("yuukei").expect("restored bounds");

    assert_eq!(bounds.width, ACTOR_WINDOW_WIDTH * 0.5);
    assert_eq!(bounds.height, ACTOR_WINDOW_HEIGHT * 0.5);
    assert!(bounds.x + bounds.width <= monitor.bounds.width - ACTOR_WINDOW_MARGIN + 0.5);
    assert!(bounds.y + bounds.height <= monitor.bounds.height - ACTOR_WINDOW_MARGIN + 0.5);
}

#[test]
fn actor_layout_avoids_collisions_between_persisted_anchors() {
    let specs = test_specs(&["yuukei", "partner"]);
    let persisted = BTreeMap::from([
        ("yuukei".to_string(), StageFootAnchor { x: 500.0, y: 680.0 }),
        (
            "partner".to_string(),
            StageFootAnchor { x: 500.0, y: 680.0 },
        ),
    ]);
    let resolved = resolve_actor_window_layout(
        &specs,
        &BTreeMap::new(),
        &persisted,
        &[test_monitor(1400.0, 800.0)],
        actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
    );

    assert!(!rects_overlap(
        resolved.get("yuukei").expect("yuukei bounds"),
        resolved.get("partner").expect("partner bounds"),
        ACTOR_COLLISION_PADDING,
    ));
}

#[test]
fn actor_layout_returns_best_effort_when_space_is_tight() {
    let monitors = vec![test_monitor(460.0, 600.0)];
    let specs = test_specs(&["yuukei", "partner", "third"]);

    let resolved = resolve_actor_window_layout(
        &specs,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &monitors,
        actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
    );

    assert_eq!(resolved.len(), 3);
    for bounds in resolved.values() {
        assert!(bounds.x.is_finite());
        assert!(bounds.y.is_finite());
        assert_eq!(bounds.width, ACTOR_WINDOW_WIDTH);
        assert_eq!(bounds.height, ACTOR_WINDOW_HEIGHT);
    }
}

#[test]
fn perch_actor_bounds_centers_on_top_edge_and_clamps_to_monitor() {
    let monitors = vec![test_monitor(1000.0, 800.0)];
    let actor = StageRect {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 100.0,
    };
    let target = yuukei_device_host::DesktopWindowFrame {
        x: 300.0,
        y: 300.0,
        width: 400.0,
        height: 300.0,
    };

    let perched = perch_actor_bounds(&actor, &target, &monitors);

    assert_eq!(
        perched,
        StageRect {
            x: 400.0,
            y: 200.0,
            width: 200.0,
            height: 100.0,
        }
    );

    let near_edge = yuukei_device_host::DesktopWindowFrame {
        x: 20.0,
        y: 40.0,
        width: 80.0,
        height: 200.0,
    };
    let clamped = perch_actor_bounds(&actor, &near_edge, &monitors);
    assert_eq!(clamped.x, 0.0);
    assert_eq!(clamped.y, 0.0);
}

#[test]
fn window_terrain_loss_restores_actor_and_reports_perch_ended() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let mut state = DesktopStageState {
        monitors: vec![test_monitor(1000.0, 800.0)],
        actors: BTreeMap::from([(
            "yuukei".to_string(),
            actor_from_spec(
                &spec,
                StageRect {
                    x: 400.0,
                    y: 200.0,
                    width: 200.0,
                    height: 100.0,
                },
                true,
            ),
        )]),
        bubbles: BTreeMap::new(),
        bubble_queues: BTreeMap::new(),
        bubble_scene_keys: BTreeMap::new(),
        perches: BTreeMap::from([(
            "yuukei".to_string(),
            StagePerch {
                window_key: "window-1".to_string(),
            },
        )]),
        terrain_windows: BTreeMap::new(),
        persisted_anchors: BTreeMap::new(),
        active_drags: BTreeMap::new(),
        active_walks: BTreeMap::new(),
        conversation_composer: None,
        actor_scale_percent: DEFAULT_ACTOR_SCALE_PERCENT,
        window_observation_enabled: true,
    };

    let (apply_bounds, ended) = apply_window_terrain_to_state(&mut state, &[]);

    assert_eq!(
        ended,
        vec![StagePerchEnded {
            actor_id: "yuukei".to_string(),
            window_key: "window-1".to_string(),
            reason: "window-closed",
        }]
    );
    assert!(state.perches.is_empty());
    assert_eq!(apply_bounds.len(), 1);
    assert_eq!(
        state.actors.get("yuukei").expect("actor").bounds.width,
        ACTOR_WINDOW_WIDTH
    );
}

#[test]
fn beginning_user_drag_releases_perch_with_user_drag_reason() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let bounds = StageRect {
        x: 120.0,
        y: 80.0,
        width: ACTOR_WINDOW_WIDTH,
        height: ACTOR_WINDOW_HEIGHT,
    };
    let mut state = DesktopStageState {
        actors: BTreeMap::from([(
            "yuukei".to_string(),
            actor_from_spec(&spec, bounds.clone(), true),
        )]),
        perches: BTreeMap::from([(
            "yuukei".to_string(),
            StagePerch {
                window_key: "window-1".to_string(),
            },
        )]),
        ..DesktopStageState::default()
    };

    let (ended, cancelled_walk_id) =
        begin_actor_drag_in_state(&mut state, "yuukei", "session-1", bounds)
            .expect("begin actor drag");
    let ended = ended.expect("perch ended");

    assert_eq!(ended.reason, "user-drag");
    assert_eq!(ended.window_key, "window-1");
    assert!(state.perches.is_empty());
    assert!(cancelled_walk_id.is_none());
    assert_eq!(
        state.active_drags.get("yuukei"),
        Some(&ActiveActorDrag {
            session_id: "session-1".to_string(),
            start_bounds: StageRect {
                x: 120.0,
                y: 80.0,
                width: ACTOR_WINDOW_WIDTH,
                height: ACTOR_WINDOW_HEIGHT,
            },
        })
    );
}

#[test]
fn stage_walk_targets_current_monitor_edge_without_changing_vertical_position() {
    let monitors = vec![StageMonitor {
        id: "secondary".to_string(),
        label: "stage-overlay-secondary".to_string(),
        name: None,
        bounds: StageRect {
            x: 1_000.0,
            y: 80.0,
            width: 1_200.0,
            height: 800.0,
        },
        scale_factor: 1.0,
    }];
    let current = StageRect {
        x: 1_250.0,
        y: 180.0,
        width: 420.0,
        height: 560.0,
    };

    let right = walk_target_bounds(&current, &monitors, WalkDestination::RightEdge);
    let left = walk_target_bounds(&current, &monitors, WalkDestination::LeftEdge);

    assert_eq!(right.x, 2_200.0 - 420.0 - ACTOR_WINDOW_MARGIN);
    assert_eq!(left.x, 1_000.0 + ACTOR_WINDOW_MARGIN);
    assert_eq!(right.y, current.y);
    assert_eq!(left.y, current.y);
}

#[test]
fn stage_walk_step_moves_at_clamped_speed_and_stops_exactly_at_target() {
    let current = StageRect {
        x: 100.0,
        y: 200.0,
        width: 420.0,
        height: 560.0,
    };
    let target = StageRect {
        x: 300.0,
        ..current.clone()
    };

    let first = advance_walk_bounds(&current, &target, 10.0, 0.5);
    assert_eq!(first.bounds.x, 130.0);
    assert!(!first.arrived);

    let arrived = advance_walk_bounds(&first.bounds, &target, 9_999.0, 1.0);
    assert_eq!(arrived.bounds, target);
    assert!(arrived.arrived);
}

#[test]
fn stage_walk_state_releases_perch_and_tracks_replacement() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let bounds = StageRect {
        x: 120.0,
        y: 80.0,
        width: ACTOR_WINDOW_WIDTH,
        height: ACTOR_WINDOW_HEIGHT,
    };
    let mut state = DesktopStageState {
        monitors: vec![test_monitor(1_000.0, 700.0)],
        actors: BTreeMap::from([("yuukei".to_string(), actor_from_spec(&spec, bounds, true))]),
        perches: BTreeMap::from([(
            "yuukei".to_string(),
            StagePerch {
                window_key: "window-1".to_string(),
            },
        )]),
        ..DesktopStageState::default()
    };

    let started = start_actor_walk_in_state(
        &mut state,
        "yuukei",
        "walk-1",
        WalkDestination::RightEdge,
        120.0,
    )
    .expect("start walk");

    assert_eq!(started.perch_ended.expect("perch ended").reason, "walk");
    assert!(state.perches.is_empty());
    assert_eq!(state.active_walks["yuukei"].walk_id, "walk-1");
    assert_eq!(
        cancel_actor_walk_in_state(&mut state, "yuukei").as_deref(),
        Some("walk-1")
    );
    assert!(cancel_actor_walk_in_state(&mut state, "yuukei").is_none());
}

#[test]
fn beginning_user_drag_cancels_active_walk() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let bounds = StageRect {
        x: 120.0,
        y: 80.0,
        width: ACTOR_WINDOW_WIDTH,
        height: ACTOR_WINDOW_HEIGHT,
    };
    let mut state = DesktopStageState {
        monitors: vec![test_monitor(1_000.0, 700.0)],
        actors: BTreeMap::from([(
            "yuukei".to_string(),
            actor_from_spec(&spec, bounds.clone(), true),
        )]),
        ..DesktopStageState::default()
    };
    start_actor_walk_in_state(
        &mut state,
        "yuukei",
        "walk-1",
        WalkDestination::RightEdge,
        240.0,
    )
    .expect("start walk");

    let (_, cancelled_walk_id) =
        begin_actor_drag_in_state(&mut state, "yuukei", "drag-1", bounds).expect("begin drag");

    assert_eq!(cancelled_walk_id.as_deref(), Some("walk-1"));
    assert!(state.active_walks.is_empty());
}

#[test]
fn arriving_stage_walk_persists_final_foot_anchor() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let bounds = StageRect {
        x: 120.0,
        y: 80.0,
        width: ACTOR_WINDOW_WIDTH,
        height: ACTOR_WINDOW_HEIGHT,
    };
    let mut state = DesktopStageState {
        monitors: vec![test_monitor(1_000.0, 700.0)],
        actors: BTreeMap::from([("yuukei".to_string(), actor_from_spec(&spec, bounds, true))]),
        ..DesktopStageState::default()
    };
    start_actor_walk_in_state(
        &mut state,
        "yuukei",
        "walk-1",
        WalkDestination::RightEdge,
        960.0,
    )
    .expect("start walk");

    let progress =
        advance_actor_walk_in_state(&mut state, "yuukei", "walk-1", 10.0).expect("active walk");

    assert!(progress.arrived);
    assert!(!state.active_walks.contains_key("yuukei"));
    assert_eq!(
        state.persisted_anchors.get("yuukei"),
        Some(&foot_anchor(&progress.bounds))
    );
}

#[test]
fn moving_actor_drag_uses_start_bounds_without_clamping_or_persisting() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let start_bounds = StageRect {
        x: 120.0,
        y: 80.0,
        width: ACTOR_WINDOW_WIDTH,
        height: ACTOR_WINDOW_HEIGHT,
    };
    let mut state = DesktopStageState {
        monitors: vec![test_monitor(1000.0, 700.0)],
        actors: BTreeMap::from([(
            "yuukei".to_string(),
            actor_from_spec(&spec, start_bounds.clone(), true),
        )]),
        persisted_anchors: BTreeMap::from([(
            "yuukei".to_string(),
            StageFootAnchor { x: 330.0, y: 640.0 },
        )]),
        ..DesktopStageState::default()
    };
    begin_actor_drag_in_state(&mut state, "yuukei", "session-1", start_bounds)
        .expect("begin actor drag");

    let moved = move_actor_drag_in_state(&mut state, "yuukei", "session-1", 2_000.0, -300.0)
        .expect("move actor drag");

    assert_eq!(moved.x, 2_120.0);
    assert_eq!(moved.y, -220.0);
    assert_eq!(
        state.persisted_anchors.get("yuukei"),
        Some(&StageFootAnchor { x: 330.0, y: 640.0 })
    );
}

#[test]
fn stale_drag_operations_do_not_move_or_end_the_current_session() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let start = StageRect {
        x: 10.0,
        y: 20.0,
        width: ACTOR_WINDOW_WIDTH,
        height: ACTOR_WINDOW_HEIGHT,
    };
    let mut state = DesktopStageState {
        actors: BTreeMap::from([(
            "yuukei".to_string(),
            actor_from_spec(&spec, start.clone(), true),
        )]),
        ..DesktopStageState::default()
    };
    begin_actor_drag_in_state(&mut state, "yuukei", "current", start.clone()).expect("begin");

    assert!(move_actor_drag_in_state(&mut state, "yuukei", "stale", 50.0, 50.0).is_err());
    assert_eq!(state.actors.get("yuukei").expect("actor").bounds, start);
    assert!(take_actor_drag_in_state(&mut state, "yuukei", "stale").is_err());
    assert_eq!(
        state.active_drags.get("yuukei").expect("active").session_id,
        "current"
    );

    let ended = take_actor_drag_in_state(&mut state, "yuukei", "current").expect("finish current");
    assert_eq!(ended.session_id, "current");
    assert!(!state.active_drags.contains_key("yuukei"));
}

#[test]
fn consecutive_actor_drag_sessions_do_not_interfere() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let start = StageRect {
        x: 10.0,
        y: 20.0,
        width: ACTOR_WINDOW_WIDTH,
        height: ACTOR_WINDOW_HEIGHT,
    };
    let mut state = DesktopStageState {
        actors: BTreeMap::from([(
            "yuukei".to_string(),
            actor_from_spec(&spec, start.clone(), true),
        )]),
        ..DesktopStageState::default()
    };
    begin_actor_drag_in_state(&mut state, "yuukei", "first", start.clone()).expect("first begin");
    take_actor_drag_in_state(&mut state, "yuukei", "first").expect("first finish");
    begin_actor_drag_in_state(&mut state, "yuukei", "second", start).expect("second begin");
    assert!(take_actor_drag_in_state(&mut state, "yuukei", "first").is_err());
    assert_eq!(
        state
            .active_drags
            .get("yuukei")
            .expect("second active")
            .session_id,
        "second"
    );
}

#[test]
fn actor_scale_recomputes_perched_actor_with_scaled_size() {
    let spec = test_specs(&["yuukei"]).remove(0);
    let target = DesktopWindowFrame {
        x: 300.0,
        y: 900.0,
        width: 400.0,
        height: 300.0,
    };
    let mut state = DesktopStageState {
        monitors: vec![test_monitor(1400.0, 1200.0)],
        actors: BTreeMap::from([(
            "yuukei".to_string(),
            actor_from_spec(
                &spec,
                StageRect {
                    x: 390.0,
                    y: 160.0,
                    width: ACTOR_WINDOW_WIDTH,
                    height: ACTOR_WINDOW_HEIGHT,
                },
                true,
            ),
        )]),
        bubbles: BTreeMap::new(),
        bubble_queues: BTreeMap::new(),
        bubble_scene_keys: BTreeMap::new(),
        perches: BTreeMap::from([(
            "yuukei".to_string(),
            StagePerch {
                window_key: "window-1".to_string(),
            },
        )]),
        terrain_windows: BTreeMap::from([("window-1".to_string(), target)]),
        persisted_anchors: BTreeMap::new(),
        active_drags: BTreeMap::new(),
        active_walks: BTreeMap::new(),
        conversation_composer: None,
        actor_scale_percent: DEFAULT_ACTOR_SCALE_PERCENT,
        window_observation_enabled: true,
    };

    let apply_bounds = apply_actor_scale_to_state(&mut state, 150);
    let bounds = state.actors.get("yuukei").expect("actor").bounds.clone();

    assert_eq!(apply_bounds.len(), 1);
    assert_eq!(bounds.width, ACTOR_WINDOW_WIDTH * 1.5);
    assert_eq!(bounds.height, ACTOR_WINDOW_HEIGHT * 1.5);
    assert_eq!(bounds.x, 300.0 + 200.0 - bounds.width / 2.0);
    assert_eq!(bounds.y, 900.0 - bounds.height);
}

#[test]
fn actor_visibility_updates_matching_stage_actor() {
    let manager = DesktopStageManager::new();
    let spec = test_specs(&["yuukei"]).remove(0);
    {
        let mut state = manager.state.write().expect("stage lock");
        state.actors.insert(
            spec.actor_id.clone(),
            actor_from_spec(
                &spec,
                StageRect {
                    x: 64.0,
                    y: 64.0,
                    width: ACTOR_WINDOW_WIDTH,
                    height: ACTOR_WINDOW_HEIGHT,
                },
                true,
            ),
        );
    }

    assert!(manager
        .set_actor_visibility_for_window(&spec.label, false)
        .expect("hide visibility"));
    assert!(
        !manager
            .snapshot()
            .expect("snapshot")
            .actors
            .first()
            .expect("actor")
            .visible
    );
    assert!(manager
        .set_actor_visibility_for_window(&spec.label, true)
        .expect("show visibility"));
    assert!(
        manager
            .snapshot()
            .expect("snapshot")
            .actors
            .first()
            .expect("actor")
            .visible
    );
}

#[test]
fn command_actor_id_prefers_explicit_target() {
    let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
    command.target = Some(yuukei_protocol::CommandTarget {
        device_id: None,
        surface_id: None,
        actor_id: Some("targeted".to_string()),
    });
    command.payload.insert(
        "speakerId".to_string(),
        Value::String("payload".to_string()),
    );

    assert_eq!(command_actor_id(&command).as_deref(), Some("targeted"));
}

#[test]
fn dialogue_say_keeps_one_visible_bubble_and_advances_same_scene_queue() {
    let mut state = bubble_state(&["yuukei"]);
    apply_dialogue_say_to_state(
        &mut state,
        &say("first", "yuukei", "一つ目", Some("scene-a"), None),
        10,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("second", "yuukei", "二つ目", Some("scene-a"), None),
        20,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("third", "yuukei", "三つ目", Some("scene-a"), None),
        30,
    );

    assert_eq!(state.bubbles.len(), 1);
    assert_eq!(only_bubble(&state).text, "一つ目");
    assert_eq!(state.bubble_queues["yuukei"].len(), 2);

    dismiss_bubble_in_state(&mut state, "first", 100);
    assert_eq!(only_bubble(&state).text, "二つ目");
    assert_eq!(only_bubble(&state).created_at_ms, 100);
    dismiss_bubble_in_state(&mut state, "second", 200);
    assert_eq!(only_bubble(&state).text, "三つ目");
    assert_eq!(only_bubble(&state).created_at_ms, 200);
    dismiss_bubble_in_state(&mut state, "third", 300);
    assert!(state.bubbles.is_empty());
}

#[test]
fn dialogue_say_replaces_visible_bubble_and_discards_queue_for_another_scene_or_no_causality() {
    let mut state = bubble_state(&["yuukei"]);
    apply_dialogue_say_to_state(
        &mut state,
        &say("first", "yuukei", "一つ目", Some("scene-a"), None),
        10,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("queued", "yuukei", "待機", Some("scene-a"), None),
        20,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("replacement", "yuukei", "別シーン", Some("scene-b"), None),
        30,
    );

    assert_eq!(state.bubbles.len(), 1);
    assert_eq!(only_bubble(&state).bubble_id, "replacement");
    assert!(state
        .bubble_queues
        .get("yuukei")
        .is_none_or(VecDeque::is_empty));

    apply_dialogue_say_to_state(
        &mut state,
        &say("no-causality", "yuukei", "因果なし", None, None),
        40,
    );
    assert_eq!(only_bubble(&state).bubble_id, "no-causality");
    assert!(state
        .bubble_queues
        .get("yuukei")
        .is_none_or(VecDeque::is_empty));
}

#[test]
fn dialogue_say_preserves_an_unresolved_choice_and_keeps_only_the_latest_other_scene_queue() {
    let mut state = bubble_state(&["yuukei"]);
    apply_dialogue_say_to_state(
        &mut state,
        &say("choice-host", "yuukei", "選んで", Some("scene-a"), None),
        10,
    );
    apply_dialogue_choices_to_state(&mut state, &choices("choice", "yuukei", 120), 20);
    apply_dialogue_say_to_state(
        &mut state,
        &say("scene-b", "yuukei", "次の場面", Some("scene-b"), None),
        30,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("scene-c", "yuukei", "最後の場面", Some("scene-c"), None),
        40,
    );

    assert_eq!(only_bubble(&state).bubble_id, "choice-host");
    assert!(only_bubble(&state).choice.is_some());
    assert_eq!(state.bubble_queues["yuukei"].len(), 1);
    assert_eq!(
        state.bubble_queues["yuukei"]
            .front()
            .expect("queued")
            .bubble_id,
        "scene-c"
    );

    clear_dialogue_choice_in_state(&mut state, "yuukei", "choice");
    dismiss_bubble_in_state(&mut state, "choice-host", 100);
    assert_eq!(only_bubble(&state).bubble_id, "scene-c");
    assert_eq!(only_bubble(&state).created_at_ms, 100);
}

#[test]
fn dialogue_say_uses_reading_time_without_duration_and_clamps_explicit_duration() {
    let mut state = bubble_state(&["yuukei"]);
    apply_dialogue_say_to_state(
        &mut state,
        &say("short", "yuukei", "短い", Some("scene-a"), None),
        10,
    );
    assert_eq!(only_bubble(&state).duration_ms, MIN_BUBBLE_DURATION_MS);

    apply_dialogue_say_to_state(
        &mut state,
        &say("long", "yuukei", &"あ".repeat(200), Some("scene-b"), None),
        20,
    );
    assert_eq!(only_bubble(&state).duration_ms, 9_000);

    apply_dialogue_say_to_state(
        &mut state,
        &say("explicit-min", "yuukei", "指定", Some("scene-c"), Some(1)),
        30,
    );
    assert_eq!(only_bubble(&state).duration_ms, MIN_BUBBLE_DURATION_MS);
    apply_dialogue_say_to_state(
        &mut state,
        &say("explicit", "yuukei", "指定", Some("scene-d"), Some(5_000)),
        40,
    );
    assert_eq!(only_bubble(&state).duration_ms, 5_000);
    apply_dialogue_say_to_state(
        &mut state,
        &say(
            "explicit-max",
            "yuukei",
            "指定",
            Some("scene-e"),
            Some(50_000),
        ),
        50,
    );
    assert_eq!(only_bubble(&state).duration_ms, MAX_BUBBLE_DURATION_MS);
}

#[test]
fn standalone_choices_respect_their_timeout_up_to_six_hundred_seconds() {
    let mut state = bubble_state(&["yuukei"]);
    apply_dialogue_choices_to_state(&mut state, &choices("two-minutes", "yuukei", 120), 10);
    assert_eq!(only_bubble(&state).duration_ms, 120_000);

    let mut state = bubble_state(&["yuukei"]);
    apply_dialogue_choices_to_state(&mut state, &choices("clamped", "yuukei", 700), 10);
    assert_eq!(only_bubble(&state).duration_ms, 600_000);
}

#[test]
fn dialogue_bubbles_are_independent_per_actor_and_actor_removal_discards_queue() {
    let mut state = bubble_state(&["yuukei", "partner"]);
    apply_dialogue_say_to_state(
        &mut state,
        &say("yuukei-visible", "yuukei", "ゆ", Some("scene-a"), None),
        10,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("yuukei-queued", "yuukei", "ゆ待機", Some("scene-a"), None),
        20,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("partner-visible", "partner", "相手", Some("scene-b"), None),
        30,
    );

    assert_eq!(state.bubbles.len(), 2);
    assert_eq!(state.bubble_queues["yuukei"].len(), 1);
    retain_stage_state_for_actors(&mut state, &BTreeSet::from(["partner".to_string()]));
    assert_eq!(state.bubbles.len(), 1);
    assert_eq!(only_bubble(&state).actor_id, "partner");
    assert!(!state.bubble_queues.contains_key("yuukei"));
}

#[test]
fn stage_snapshot_does_not_expose_bubble_queue_or_scene_state() {
    let mut state = bubble_state(&["yuukei"]);
    apply_dialogue_say_to_state(
        &mut state,
        &say("visible", "yuukei", "表示", Some("scene-a"), None),
        10,
    );
    apply_dialogue_say_to_state(
        &mut state,
        &say("queued", "yuukei", "待機", Some("scene-a"), None),
        20,
    );

    let snapshot = serde_json::to_value(state.snapshot()).expect("snapshot JSON");
    assert!(snapshot.get("bubbles").is_some());
    assert!(snapshot.get("bubbleQueues").is_none());
    assert!(snapshot.get("bubbleSceneKeys").is_none());
}

fn bubble_state(actor_ids: &[&str]) -> DesktopStageState {
    let specs = test_specs(actor_ids);
    DesktopStageState {
        actors: specs
            .iter()
            .map(|spec| {
                (
                    spec.actor_id.clone(),
                    actor_from_spec(
                        spec,
                        StageRect {
                            x: 0.0,
                            y: 0.0,
                            width: ACTOR_WINDOW_WIDTH,
                            height: ACTOR_WINDOW_HEIGHT,
                        },
                        true,
                    ),
                )
            })
            .collect(),
        ..DesktopStageState::default()
    }
}

fn say(
    id: &str,
    actor_id: &str,
    text: &str,
    source_event_id: Option<&str>,
    duration_ms: Option<u64>,
) -> RuntimeCommand {
    let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
    command.id = id.to_string();
    command.target = Some(yuukei_protocol::CommandTarget {
        device_id: None,
        surface_id: None,
        actor_id: Some(actor_id.to_string()),
    });
    command
        .payload
        .insert("text".to_string(), Value::String(text.to_string()));
    if let Some(duration_ms) = duration_ms {
        command
            .payload
            .insert("durationMs".to_string(), Value::Number(duration_ms.into()));
    }
    command.causality = source_event_id.map(|source_event_id| yuukei_protocol::Causality {
        source_event_id: Some(source_event_id.to_string()),
        source_command_id: None,
        trace_id: None,
    });
    command
}

fn choices(id: &str, actor_id: &str, timeout_seconds: u64) -> RuntimeCommand {
    let mut command = RuntimeCommand::new("dialogue.choices", "daihon", "resident-default");
    command.id = format!("command-{id}");
    command.target = Some(yuukei_protocol::CommandTarget {
        device_id: None,
        surface_id: None,
        actor_id: Some(actor_id.to_string()),
    });
    command
        .payload
        .insert("choiceId".to_string(), Value::String(id.to_string()));
    command.payload.insert(
        "choices".to_string(),
        Value::Array(vec![
            Value::String("はい".to_string()),
            Value::String("いいえ".to_string()),
        ]),
    );
    command.payload.insert(
        "timeoutSeconds".to_string(),
        Value::Number(timeout_seconds.into()),
    );
    command
}

fn only_bubble(state: &DesktopStageState) -> &StageBubble {
    assert_eq!(state.bubbles.len(), 1);
    state.bubbles.values().next().expect("one bubble")
}

fn test_catalog(actors: Vec<crate::DesktopActorSurfaceAsset>) -> DesktopActorSurfaceAssetCatalog {
    DesktopActorSurfaceAssetCatalog {
        world_pack_id: "world-test".to_string(),
        actors,
    }
}

fn test_actor(actor_id: &str, renderable: bool) -> crate::DesktopActorSurfaceAsset {
    crate::DesktopActorSurfaceAsset {
        actor_id: actor_id.to_string(),
        display_name: actor_id.to_string(),
        renderer: renderable.then(|| crate::DesktopActorSurfaceRendererAsset {
            kind: "vrm",
            model_url: format!("yuukei-pack://localhost/actors/{actor_id}/model"),
            motions: Default::default(),
            hit_zones: Vec::new(),
        }),
    }
}

fn test_monitor(width: f64, height: f64) -> StageMonitor {
    StageMonitor {
        id: "monitor-0".to_string(),
        label: stage_overlay_window_label(0),
        name: None,
        bounds: StageRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        },
        scale_factor: 1.0,
    }
}

fn test_specs(actor_ids: &[&str]) -> Vec<ActorWindowSpec> {
    actor_ids
        .iter()
        .enumerate()
        .map(|(index, actor_id)| ActorWindowSpec {
            actor_id: (*actor_id).to_string(),
            display_name: (*actor_id).to_string(),
            label: actor_window_label(actor_id),
            index,
        })
        .collect()
}
