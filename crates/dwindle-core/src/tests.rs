use std::collections::HashMap;

use crate::{
    EngineError, EngineState, LayoutTree, Rect,
    navigation::best_candidate,
    protocol::{
        Command, Config, ContextKey, CycleDirection, Direction, Response, Snapshot, WindowId,
        WindowSnapshot, WorkAreaSnapshot,
    },
};

fn id(value: &str) -> WindowId {
    WindowId::new(value)
}

fn context(workspace: u32, monitor: u32) -> ContextKey {
    ContextKey { workspace, monitor }
}

fn area() -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }
}

fn config() -> Config {
    Config {
        inner_gap: 10,
        outer_gap: 10,
        ..Config::default()
    }
}

fn window(name: &str, context: ContextKey, focused: bool) -> WindowSnapshot {
    WindowSnapshot {
        id: id(name),
        context,
        app_id: Some(format!("dev.test.{name}")),
        frame_rect: Rect::default(),
        fullscreen: false,
        window_type: "normal".into(),
        focused,
    }
}

fn synced_state(contexts: &[(ContextKey, Rect)]) -> EngineState {
    let mut state = EngineState::new(config());
    state
        .apply(Command::FullSync {
            snapshot: Snapshot {
                windows: Vec::new(),
                work_areas: contexts
                    .iter()
                    .map(|(context, rect)| WorkAreaSnapshot {
                        context: *context,
                        rect: *rect,
                    })
                    .collect(),
            },
            config: config(),
        })
        .unwrap();
    state
}

fn add(state: &mut EngineState, name: &str, target: ContextKey, focused: bool, rect: Rect) {
    state
        .apply(Command::AddWindow {
            window: window(name, target, focused),
            work_area: rect,
        })
        .unwrap();
    if focused {
        state
            .apply(Command::FocusWindow {
                window_id: id(name),
            })
            .unwrap();
    }
}

fn rect_for(state: &EngineState, name: &str) -> Rect {
    state
        .placements()
        .into_iter()
        .find(|placement| placement.window_id == id(name))
        .unwrap()
        .rect
}

#[test]
fn basic_dwindle_insertion_uses_focused_leaf() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    assert_eq!(
        rect_for(&state, "A"),
        Rect {
            x: 10,
            y: 10,
            width: 1900,
            height: 1060
        }
    );

    add(&mut state, "B", key, true, area());
    assert_eq!(
        rect_for(&state, "A"),
        Rect {
            x: 10,
            y: 10,
            width: 945,
            height: 1060
        }
    );
    assert_eq!(
        rect_for(&state, "B"),
        Rect {
            x: 965,
            y: 10,
            width: 945,
            height: 1060
        }
    );

    add(&mut state, "C", key, false, area());
    assert_eq!(
        rect_for(&state, "B"),
        Rect {
            x: 965,
            y: 10,
            width: 945,
            height: 525
        }
    );
    assert_eq!(
        rect_for(&state, "C"),
        Rect {
            x: 965,
            y: 545,
            width: 945,
            height: 525
        }
    );
    state.validate_invariants().unwrap();
}

#[test]
fn removal_collapses_parents_and_final_window_clears_root() {
    let mut tree = LayoutTree::new();
    let cfg = config();
    tree.insert(id("A"), None, area(), &cfg).unwrap();
    tree.insert(id("B"), Some(&id("A")), area(), &cfg).unwrap();
    tree.insert(id("C"), Some(&id("B")), area(), &cfg).unwrap();
    tree.remove(&id("B")).unwrap();
    assert_eq!(tree.len(), 2);
    tree.validate_invariants().unwrap();
    tree.remove(&id("A")).unwrap();
    tree.validate_invariants().unwrap();
    tree.remove(&id("C")).unwrap();
    assert!(tree.is_empty());
    tree.validate_invariants().unwrap();
}

#[test]
fn arbitrary_negative_monitor_coordinates_stay_inside_work_area() {
    let work_area = Rect {
        x: -1920,
        y: 120,
        width: 1920,
        height: 1080,
    };
    let key = context(0, 1);
    let mut state = synced_state(&[(key, work_area)]);
    for name in ["A", "B", "C", "D", "E"] {
        add(&mut state, name, key, false, work_area);
    }
    for placement in state.placements() {
        assert!(work_area.contains(placement.rect), "{placement:?}");
        assert!(placement.rect.width > 0 && placement.rect.height > 0);
    }
}

#[test]
fn one_pixel_splits_overlap_instead_of_emitting_invalid_rectangles() {
    let tiny = Rect {
        x: 4,
        y: 7,
        width: 1,
        height: 1,
    };
    assert_eq!(tiny.split_horizontal(0.5, 10), (tiny, tiny));
    assert_eq!(tiny.split_vertical(0.5, 10), (tiny, tiny));

    let key = context(0, 0);
    let mut state = synced_state(&[(key, tiny)]);
    add(&mut state, "A", key, false, tiny);
    add(&mut state, "B", key, false, tiny);
    assert!(
        state
            .placements()
            .iter()
            .all(|placement| placement.rect == tiny)
    );
}

#[test]
fn focused_add_window_becomes_the_next_insertion_target() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    state
        .apply(Command::AddWindow {
            window: window("B", key, true),
            work_area: area(),
        })
        .unwrap();
    state
        .apply(Command::AddWindow {
            window: window("C", key, false),
            work_area: area(),
        })
        .unwrap();

    assert_eq!(rect_for(&state, "A").height, 1060);
    assert_eq!(rect_for(&state, "B").height, 525);
}

#[test]
fn monitors_and_workspaces_are_independent() {
    let c00 = context(0, 0);
    let c01 = context(0, 1);
    let c10 = context(1, 0);
    let mut state = synced_state(&[(c00, area()), (c01, area()), (c10, area())]);
    add(&mut state, "A", c00, false, area());
    add(&mut state, "B", c01, false, area());
    add(&mut state, "C", c10, false, area());
    add(&mut state, "D", c00, false, area());
    assert_eq!(state.context_len(c00), 2);
    assert_eq!(state.context_len(c01), 1);
    assert_eq!(state.context_len(c10), 1);
    assert_eq!(rect_for(&state, "B"), area().inset(10));
    assert_eq!(rect_for(&state, "C"), area().inset(10));
}

#[test]
fn monitor_and_workspace_migration_moves_exactly_once() {
    let c00 = context(0, 0);
    let c01 = context(0, 1);
    let c11 = context(1, 1);
    let mut state = synced_state(&[(c00, area()), (c01, area()), (c11, area())]);
    add(&mut state, "A", c00, false, area());
    add(&mut state, "B", c00, false, area());

    state
        .apply(Command::WindowContextChanged {
            window_id: id("B"),
            context: c01,
            work_area: area(),
        })
        .unwrap();
    assert_eq!(state.context_len(c00), 1);
    assert_eq!(state.context_len(c01), 1);
    assert_eq!(state.window_context(&id("B")), Some(c01));

    state
        .apply(Command::WindowContextChanged {
            window_id: id("B"),
            context: c11,
            work_area: area(),
        })
        .unwrap();
    assert_eq!(state.context_len(c01), 0);
    assert_eq!(state.context_len(c11), 1);
    state.validate_invariants().unwrap();
}

#[test]
fn floating_removes_and_reinserts_window() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, false, area());
    state
        .apply(Command::ToggleFloating { window_id: id("B") })
        .unwrap();
    assert_eq!(state.is_floating(&id("B")), Some(true));
    assert_eq!(state.context_len(key), 1);
    assert_eq!(state.placements().len(), 1);

    state
        .apply(Command::ToggleFloating { window_id: id("B") })
        .unwrap();
    assert_eq!(state.is_floating(&id("B")), Some(false));
    assert_eq!(state.context_len(key), 2);
    state.validate_invariants().unwrap();
}

#[test]
fn grouping_shares_one_leaf_cycles_focus_and_ungroups() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, true, area());

    state
        .apply(Command::ToggleGroup { window_id: id("B") })
        .unwrap();
    assert_eq!(rect_for(&state, "A"), rect_for(&state, "B"));
    assert_eq!(state.context_len(key), 2);
    assert_eq!(
        state
            .apply(Command::CycleGroup {
                window_id: id("B"),
                cycle: CycleDirection::Next,
            })
            .unwrap(),
        Response::Focus { window_id: id("A") }
    );

    state
        .apply(Command::ToggleGroup { window_id: id("A") })
        .unwrap();
    assert_ne!(rect_for(&state, "A"), rect_for(&state, "B"));
    state.validate_invariants().unwrap();
}

#[test]
fn removing_a_group_member_keeps_the_leaf_valid() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, true, area());
    state
        .apply(Command::ToggleGroup { window_id: id("B") })
        .unwrap();
    state
        .apply(Command::RemoveWindow { window_id: id("A") })
        .unwrap();
    assert_eq!(state.context_len(key), 1);
    assert_eq!(state.placements().len(), 1);
    assert_eq!(state.placements()[0].window_id, id("B"));
    state.validate_invariants().unwrap();
}

#[test]
fn directional_focus_prefers_axis_overlap() {
    let rects = HashMap::from([
        (
            id("A"),
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
        ),
        (
            id("B"),
            Rect {
                x: 100,
                y: 0,
                width: 100,
                height: 100,
            },
        ),
        (
            id("C"),
            Rect {
                x: 0,
                y: 100,
                width: 100,
                height: 100,
            },
        ),
        (
            id("D"),
            Rect {
                x: 100,
                y: 100,
                width: 100,
                height: 100,
            },
        ),
    ]);
    assert_eq!(
        best_candidate(&id("A"), Direction::Right, &rects),
        Some(id("B"))
    );
    assert_eq!(
        best_candidate(&id("A"), Direction::Down, &rects),
        Some(id("C"))
    );
    assert_eq!(
        best_candidate(&id("D"), Direction::Left, &rects),
        Some(id("C"))
    );
    assert_eq!(
        best_candidate(&id("D"), Direction::Up, &rects),
        Some(id("B"))
    );
}

#[test]
fn directional_swap_exchanges_leaf_positions() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, false, area());
    let a_before = rect_for(&state, "A");
    let b_before = rect_for(&state, "B");
    state
        .apply(Command::MoveDirection {
            window_id: id("B"),
            direction: Direction::Left,
        })
        .unwrap();
    assert_eq!(rect_for(&state, "A"), b_before);
    assert_eq!(rect_for(&state, "B"), a_before);
}

#[test]
fn resize_grows_and_shrinks_from_both_sides_and_clamps() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, false, area());
    let before = rect_for(&state, "A").width;
    state
        .apply(Command::Resize {
            window_id: id("A"),
            direction: Direction::Right,
        })
        .unwrap();
    assert!(rect_for(&state, "A").width > before);

    let before = rect_for(&state, "B").width;
    state
        .apply(Command::Resize {
            window_id: id("B"),
            direction: Direction::Right,
        })
        .unwrap();
    assert!(rect_for(&state, "B").width > before);

    let before = rect_for(&state, "B").width;
    state
        .apply(Command::Resize {
            window_id: id("B"),
            direction: Direction::Left,
        })
        .unwrap();
    assert!(rect_for(&state, "B").width < before);

    for _ in 0..100 {
        state
            .apply(Command::Resize {
                window_id: id("A"),
                direction: Direction::Right,
            })
            .unwrap();
    }
    let usable = area().inset(10).width - 10;
    assert_eq!(
        rect_for(&state, "A").width,
        (f64::from(usable) * 0.9).round() as i32
    );
    state.validate_invariants().unwrap();
}

#[test]
fn full_sync_rebuilds_all_contexts_and_focus() {
    let c0 = context(0, 0);
    let c1 = context(0, 1);
    let mut state = EngineState::default();
    state
        .apply(Command::FullSync {
            snapshot: Snapshot {
                work_areas: vec![
                    WorkAreaSnapshot {
                        context: c0,
                        rect: area(),
                    },
                    WorkAreaSnapshot {
                        context: c1,
                        rect: area(),
                    },
                ],
                windows: vec![
                    window("A", c0, false),
                    window("B", c0, true),
                    window("C", c1, false),
                ],
            },
            config: config(),
        })
        .unwrap();
    assert_eq!(state.context_len(c0), 2);
    assert_eq!(state.context_len(c1), 1);
    assert_eq!(state.placements().len(), 3);
    state.validate_invariants().unwrap();
}

#[test]
fn failed_full_sync_keeps_the_previous_state() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    let before = state.placements();

    let result = state.apply(Command::FullSync {
        snapshot: Snapshot {
            work_areas: vec![WorkAreaSnapshot {
                context: key,
                rect: area(),
            }],
            windows: vec![window("B", key, false), window("C", context(9, 9), true)],
        },
        config: config(),
    });

    assert!(matches!(result, Err(EngineError::MissingContext(9, 9))));
    assert_eq!(state.placements(), before);
    state.validate_invariants().unwrap();
}

#[test]
fn requested_mutation_sequence_preserves_invariants() {
    let c0 = context(0, 0);
    let c1 = context(0, 1);
    let mut state = synced_state(&[(c0, area()), (c1, area())]);
    add(&mut state, "A", c0, true, area());
    add(&mut state, "B", c0, true, area());
    add(&mut state, "C", c0, true, area());
    state
        .apply(Command::RemoveWindow { window_id: id("B") })
        .unwrap();
    add(&mut state, "D", c0, false, area());
    state
        .apply(Command::MoveDirection {
            window_id: id("A"),
            direction: Direction::Right,
        })
        .unwrap();
    state
        .apply(Command::ToggleFloating { window_id: id("C") })
        .unwrap();
    state
        .apply(Command::ToggleFloating { window_id: id("C") })
        .unwrap();
    state
        .apply(Command::WindowContextChanged {
            window_id: id("D"),
            context: c1,
            work_area: area(),
        })
        .unwrap();
    state
        .apply(Command::RemoveWindow { window_id: id("A") })
        .unwrap();
    state.validate_invariants().unwrap();
}

#[test]
fn protocol_is_typed_at_the_json_boundary() {
    let command = Command::Resize {
        window_id: id("42"),
        direction: Direction::Left,
    };
    let json = serde_json::to_string(&command).unwrap();
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);
    assert!(
        serde_json::from_str::<Command>(
            r#"{"command":"resize","window_id":42,"direction":"left"}"#
        )
        .is_err()
    );
}
