use super::*;

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
            perches: BTreeMap::from([(
                "yuukei".to_string(),
                StagePerch {
                    window_key: "window-1".to_string(),
                },
            )]),
            terrain_windows: BTreeMap::new(),
            persisted_anchors: BTreeMap::new(),
            active_drags: BTreeMap::new(),
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

        let ended = begin_actor_drag_in_state(&mut state, "yuukei", bounds)
            .expect("begin actor drag")
            .expect("perch ended");

        assert_eq!(ended.reason, "user-drag");
        assert_eq!(ended.window_key, "window-1");
        assert!(state.perches.is_empty());
        assert_eq!(
            state.active_drags.get("yuukei"),
            Some(&ActiveActorDrag {
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
        begin_actor_drag_in_state(&mut state, "yuukei", start_bounds).expect("begin actor drag");

        let moved = move_actor_drag_in_state(&mut state, "yuukei", 2_000.0, -300.0)
            .expect("move actor drag");

        assert_eq!(moved.x, 2_120.0);
        assert_eq!(moved.y, -220.0);
        assert_eq!(
            state.persisted_anchors.get("yuukei"),
            Some(&StageFootAnchor { x: 330.0, y: 640.0 })
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
            perches: BTreeMap::from([(
                "yuukei".to_string(),
                StagePerch {
                    window_key: "window-1".to_string(),
                },
            )]),
            terrain_windows: BTreeMap::from([("window-1".to_string(), target)]),
            persisted_anchors: BTreeMap::new(),
            active_drags: BTreeMap::new(),
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

    fn test_catalog(
        actors: Vec<crate::DesktopActorSurfaceAsset>,
    ) -> DesktopActorSurfaceAssetCatalog {
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
