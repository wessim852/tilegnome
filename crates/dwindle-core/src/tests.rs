use std::collections::HashMap;

use crate::{
    EngineError, EngineState, LayoutTree, Orientation, Rect, TreeError,
    navigation::best_candidate,
    protocol::{
        Command, Config, ContextKey, Direction, Response, Snapshot, WindowId, WindowSnapshot,
        WorkAreaSnapshot,
    },
    tree::{NodeKey, NodeKind},
};

#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Leaf(String),
    Split(Orientation, f64, Box<Shape>, Box<Shape>),
}

fn shape(tree: &LayoutTree) -> Option<Shape> {
    fn node(tree: &LayoutTree, key: NodeKey) -> Shape {
        match &tree.nodes[key].kind {
            NodeKind::Leaf { window } => Shape::Leaf(window.0.clone()),
            NodeKind::Split {
                orientation,
                ratio,
                first,
                second,
            } => Shape::Split(
                *orientation,
                (*ratio * 1_000_000.0).round() / 1_000_000.0,
                Box::new(node(tree, *first)),
                Box::new(node(tree, *second)),
            ),
        }
    }
    tree.root.map(|root| node(tree, root))
}

fn leaf(name: &str) -> Shape {
    Shape::Leaf(name.into())
}

fn split(orientation: Orientation, ratio: f64, first: Shape, second: Shape) -> Shape {
    Shape::Split(orientation, ratio, Box::new(first), Box::new(second))
}

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
        min_width: 0,
        min_height: 0,
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

fn state_shape(state: &EngineState, context: ContextKey) -> Option<Shape> {
    shape(state.tree(context).unwrap())
}

fn focus(state: &mut EngineState, name: &str) {
    state
        .apply(Command::FocusWindow {
            window_id: id(name),
        })
        .unwrap();
}

fn toggle_maximize(state: &mut EngineState, name: &str) {
    state
        .apply(Command::ToggleMaximize {
            window_id: id(name),
        })
        .unwrap();
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
fn refocusing_left_leaf_splits_left_branch_not_last_inserted_branch() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "Firefox", key, true, area());
    add(&mut state, "Terminal", key, false, area());
    focus(&mut state, "Terminal");
    add(&mut state, "Zed", key, false, area());

    focus(&mut state, "Firefox");
    add(&mut state, "New", key, false, area());

    assert_eq!(
        state_shape(&state, key),
        Some(split(
            Orientation::Horizontal,
            0.5,
            split(Orientation::Vertical, 0.5, leaf("Firefox"), leaf("New"),),
            split(Orientation::Vertical, 0.5, leaf("Terminal"), leaf("Zed"),),
        ))
    );
    state.validate_invariants().unwrap();
}

#[test]
fn client_minimum_size_does_not_bypass_the_focused_leaf() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, true, area());

    let mut spotify = window("C", key, false);
    spotify.min_width = 800;
    spotify.min_height = 600;
    state
        .apply(Command::AddWindow {
            window: spotify,
            work_area: area(),
        })
        .unwrap();

    assert_eq!(rect_for(&state, "A").width, 945);
    assert_eq!(rect_for(&state, "A").height, 1060);
    assert_eq!(rect_for(&state, "B").width, 945);
    assert_eq!(rect_for(&state, "B").height, 525);
    assert_eq!(rect_for(&state, "C").width, 945);
    assert_eq!(rect_for(&state, "C").height, 525);
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
fn one_pixel_target_rejects_split_without_mutating_state() {
    let tiny = Rect {
        x: 4,
        y: 7,
        width: 1,
        height: 1,
    };
    assert_eq!(tiny.split_horizontal(0.5, 10), (tiny, tiny));
    assert_eq!(tiny.split_vertical(0.5, 10), (tiny, tiny));

    let mut tree = LayoutTree::new();
    tree.insert(id("A"), None, tiny, &config()).unwrap();
    assert_eq!(
        tree.insert(id("B"), Some(&id("A")), tiny, &config()),
        Err(TreeError::CannotSplit {
            width: 1,
            height: 1,
        })
    );
    assert_eq!(shape(&tree), Some(leaf("A")));
    tree.validate_invariants().unwrap();

    let key = context(0, 0);
    let mut state = synced_state(&[(key, tiny)]);
    add(&mut state, "A", key, true, tiny);
    assert!(matches!(
        state.apply(Command::AddWindow {
            window: window("B", key, false),
            work_area: tiny,
        }),
        Err(EngineError::Tree(TreeError::CannotSplit { .. }))
    ));
    assert_eq!(state.context_len(key), 1);
    assert_eq!(state.window_context(&id("B")), None);
    state.validate_invariants().unwrap();
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
    let mut check = |command| {
        state.apply(command).unwrap();
        state.validate_invariants().unwrap();
    };
    for (name, focused) in [("A", true), ("B", true), ("C", true)] {
        check(Command::AddWindow {
            window: window(name, c0, focused),
            work_area: area(),
        });
    }
    check(Command::Resize {
        window_id: id("C"),
        direction: Direction::Down,
    });
    check(Command::MoveDirection {
        window_id: id("A"),
        direction: Direction::Right,
    });
    check(Command::AddWindow {
        window: window("D", c0, false),
        work_area: area(),
    });
    check(Command::RemoveWindow { window_id: id("B") });
    check(Command::ToggleFloating { window_id: id("C") });
    check(Command::Relayout { context: Some(c1) });
    check(Command::Relayout { context: Some(c0) });
    check(Command::ToggleFloating { window_id: id("C") });
    check(Command::WindowContextChanged {
        window_id: id("D"),
        context: c1,
        work_area: area(),
    });
    check(Command::RemoveWindow { window_id: id("A") });
    check(Command::AddWindow {
        window: window("E", c0, false),
        work_area: area(),
    });
}

#[test]
fn directional_swap_exchanges_leaf_contents_without_rebuilding_tree() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, false, area());
    let before_nodes = state.tree(key).unwrap().nodes.len();
    state
        .apply(Command::MoveDirection {
            window_id: id("A"),
            direction: Direction::Right,
        })
        .unwrap();
    assert_eq!(state.tree(key).unwrap().nodes.len(), before_nodes);
    assert_eq!(
        state_shape(&state, key),
        Some(split(Orientation::Horizontal, 0.5, leaf("B"), leaf("A"),))
    );
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

#[test]
fn removal_collapses_exact_parent_and_reinsertion_splits_current_focus() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "Firefox", key, true, area());
    add(&mut state, "Terminal", key, true, area());
    add(&mut state, "Zed", key, false, area());
    let expected_three = split(
        Orientation::Horizontal,
        0.5,
        leaf("Firefox"),
        split(Orientation::Vertical, 0.5, leaf("Terminal"), leaf("Zed")),
    );
    assert_eq!(state_shape(&state, key), Some(expected_three));

    state
        .apply(Command::RemoveWindow {
            window_id: id("Zed"),
        })
        .unwrap();
    assert_eq!(
        state_shape(&state, key),
        Some(split(
            Orientation::Horizontal,
            0.5,
            leaf("Firefox"),
            leaf("Terminal"),
        ))
    );

    focus(&mut state, "Terminal");
    add(&mut state, "Discord", key, false, area());
    assert_eq!(
        state_shape(&state, key),
        Some(split(
            Orientation::Horizontal,
            0.5,
            leaf("Firefox"),
            split(
                Orientation::Vertical,
                0.5,
                leaf("Terminal"),
                leaf("Discord"),
            ),
        ))
    );
}

#[test]
fn collapse_preserves_ancestor_ratio() {
    let mut tree = LayoutTree::new();
    let mut cfg = config();
    cfg.split_ratio = 0.65;
    tree.insert(id("A"), None, area(), &cfg).unwrap();
    tree.insert(id("B"), Some(&id("A")), area(), &cfg).unwrap();
    cfg.split_ratio = 0.4;
    tree.insert(id("C"), Some(&id("B")), area(), &cfg).unwrap();

    tree.remove(&id("C")).unwrap();
    assert_eq!(
        shape(&tree),
        Some(split(Orientation::Horizontal, 0.65, leaf("A"), leaf("B"),))
    );
}

#[test]
fn recursive_removal_collapses_every_single_child_parent() {
    let mut tree = LayoutTree::new();
    let cfg = config();
    tree.insert(id("A"), None, area(), &cfg).unwrap();
    tree.insert(id("B"), Some(&id("A")), area(), &cfg).unwrap();
    tree.insert(id("C"), Some(&id("B")), area(), &cfg).unwrap();
    tree.insert(id("D"), Some(&id("C")), area(), &cfg).unwrap();

    for (removed, expected) in [
        (
            "D",
            Some(split(
                Orientation::Horizontal,
                0.5,
                leaf("A"),
                split(Orientation::Vertical, 0.5, leaf("B"), leaf("C")),
            )),
        ),
        (
            "C",
            Some(split(Orientation::Horizontal, 0.5, leaf("A"), leaf("B"))),
        ),
        ("B", Some(leaf("A"))),
        ("A", None),
    ] {
        tree.remove(&id(removed)).unwrap();
        tree.validate_invariants().unwrap();
        assert_eq!(shape(&tree), expected);
    }
}

#[test]
fn workspace_relayout_and_gap_configuration_preserve_tree_ratio_and_swap() {
    let c0 = context(0, 0);
    let c1 = context(1, 0);
    let mut state = synced_state(&[(c0, area()), (c1, area())]);
    add(&mut state, "A", c0, true, area());
    add(&mut state, "B", c0, false, area());
    state
        .apply(Command::Resize {
            window_id: id("A"),
            direction: Direction::Right,
        })
        .unwrap();
    state
        .apply(Command::MoveDirection {
            window_id: id("B"),
            direction: Direction::Left,
        })
        .unwrap();
    let before = state_shape(&state, c0);

    state
        .apply(Command::Relayout { context: Some(c1) })
        .unwrap();
    state
        .apply(Command::Relayout { context: Some(c0) })
        .unwrap();
    let mut changed = config();
    changed.inner_gap = 37;
    state.apply(Command::Configure { config: changed }).unwrap();
    assert_eq!(state_shape(&state, c0), before);
}

#[test]
fn resizing_survives_nested_insert_and_collapse() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, false, area());
    for _ in 0..4 {
        state
            .apply(Command::Resize {
                window_id: id("A"),
                direction: Direction::Right,
            })
            .unwrap();
    }
    focus(&mut state, "B");
    add(&mut state, "C", key, false, area());
    state
        .apply(Command::RemoveWindow { window_id: id("C") })
        .unwrap();
    assert_eq!(
        state_shape(&state, key),
        Some(split(Orientation::Horizontal, 0.7, leaf("A"), leaf("B"),))
    );
}

#[test]
fn floating_reinserts_at_current_focus_not_deleted_slot() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, true, area());
    add(&mut state, "C", key, false, area());
    focus(&mut state, "A");
    state
        .apply(Command::ToggleFloating { window_id: id("B") })
        .unwrap();
    state
        .apply(Command::ToggleFloating { window_id: id("B") })
        .unwrap();
    assert_eq!(
        state_shape(&state, key),
        Some(split(
            Orientation::Horizontal,
            0.5,
            split(Orientation::Vertical, 0.5, leaf("A"), leaf("B")),
            leaf("C"),
        ))
    );
}

#[test]
fn maximize_toggle_is_presentation_only_and_uses_work_area() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "Firefox", key, true, area());
    add(&mut state, "Terminal", key, false, area());
    for _ in 0..4 {
        state
            .apply(Command::Resize {
                window_id: id("Firefox"),
                direction: Direction::Right,
            })
            .unwrap();
    }
    let before = state_shape(&state, key);

    toggle_maximize(&mut state, "Firefox");
    assert_eq!(state.maximized_window(key), Some(&id("Firefox")));
    assert_eq!(state_shape(&state, key), before);
    assert_eq!(rect_for(&state, "Firefox"), area());

    toggle_maximize(&mut state, "Firefox");
    assert_eq!(state.maximized_window(key), None);
    assert_eq!(state_shape(&state, key), before);
}

#[test]
fn nested_tree_is_identical_after_maximize_cycle() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "Firefox", key, true, area());
    add(&mut state, "Terminal", key, true, area());
    let mut changed = config();
    changed.split_ratio = 0.35;
    state.apply(Command::Configure { config: changed }).unwrap();
    add(&mut state, "Zed", key, false, area());
    let before = state_shape(&state, key);
    toggle_maximize(&mut state, "Terminal");
    toggle_maximize(&mut state, "Terminal");
    assert_eq!(state_shape(&state, key), before);
}

#[test]
fn removing_windows_updates_only_relevant_maximize_state() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "Firefox", key, true, area());
    add(&mut state, "Terminal", key, true, area());
    add(&mut state, "Zed", key, false, area());
    toggle_maximize(&mut state, "Firefox");

    state
        .apply(Command::RemoveWindow {
            window_id: id("Zed"),
        })
        .unwrap();
    assert_eq!(state.maximized_window(key), Some(&id("Firefox")));
    assert_eq!(
        state_shape(&state, key),
        Some(split(
            Orientation::Horizontal,
            0.5,
            leaf("Firefox"),
            leaf("Terminal"),
        ))
    );

    state
        .apply(Command::RemoveWindow {
            window_id: id("Firefox"),
        })
        .unwrap();
    assert_eq!(state.maximized_window(key), None);
    assert_eq!(state_shape(&state, key), Some(leaf("Terminal")));
}

#[test]
fn new_tiled_window_cancels_maximize_only_in_its_context() {
    let c0 = context(0, 0);
    let c1 = context(0, 1);
    let mut state = synced_state(&[(c0, area()), (c1, area())]);
    add(&mut state, "Firefox", c0, true, area());
    add(&mut state, "Terminal", c0, false, area());
    toggle_maximize(&mut state, "Firefox");

    let response = state
        .apply(Command::AddWindow {
            window: window("Zed", c1, false),
            work_area: area(),
        })
        .unwrap();
    assert_eq!(state.maximized_window(c0), Some(&id("Firefox")));
    assert!(
        matches!(response, Response::Placements { placements } if placements.len() == 1 && placements[0].window_id == id("Zed"))
    );

    add(&mut state, "Discord", c0, false, area());
    assert_eq!(state.maximized_window(c0), None);
}

#[test]
fn maximize_is_independent_per_monitor_and_survives_workspace_relayout_and_config() {
    let c00 = context(0, 0);
    let c01 = context(0, 1);
    let c10 = context(1, 0);
    let mut state = synced_state(&[(c00, area()), (c01, area()), (c10, area())]);
    add(&mut state, "Firefox", c00, true, area());
    add(&mut state, "Terminal", c00, false, area());
    add(&mut state, "Zed", c01, true, area());
    add(&mut state, "Kitty", c01, false, area());
    add(&mut state, "Browser", c10, true, area());
    toggle_maximize(&mut state, "Firefox");
    toggle_maximize(&mut state, "Zed");
    toggle_maximize(&mut state, "Browser");
    let trees = (
        state_shape(&state, c00),
        state_shape(&state, c01),
        state_shape(&state, c10),
    );

    state
        .apply(Command::Relayout { context: Some(c10) })
        .unwrap();
    let mut changed = config();
    changed.outer_gap = 44;
    state.apply(Command::Configure { config: changed }).unwrap();
    assert_eq!(state.maximized_window(c00), Some(&id("Firefox")));
    assert_eq!(state.maximized_window(c01), Some(&id("Zed")));
    assert_eq!(state.maximized_window(c10), Some(&id("Browser")));
    assert_eq!(
        (
            state_shape(&state, c00),
            state_shape(&state, c01),
            state_shape(&state, c10),
        ),
        trees
    );
}

#[test]
fn moving_maximized_and_background_windows_obeys_context_rules() {
    let c0 = context(0, 0);
    let c1 = context(0, 1);
    let mut state = synced_state(&[(c0, area()), (c1, area())]);
    add(&mut state, "Firefox", c0, true, area());
    add(&mut state, "Terminal", c0, false, area());
    add(&mut state, "Zed", c0, false, area());
    toggle_maximize(&mut state, "Firefox");

    state
        .apply(Command::WindowContextChanged {
            window_id: id("Zed"),
            context: c1,
            work_area: area(),
        })
        .unwrap();
    assert_eq!(state.maximized_window(c0), Some(&id("Firefox")));

    state
        .apply(Command::WindowContextChanged {
            window_id: id("Firefox"),
            context: c1,
            work_area: area(),
        })
        .unwrap();
    assert_eq!(state.maximized_window(c0), None);
    assert_eq!(state.maximized_window(c1), None);
    assert_eq!(state.window_context(&id("Firefox")), Some(c1));
    state.validate_invariants().unwrap();
}

#[test]
fn focus_swap_resize_and_float_cancel_same_context_maximize() {
    let key = context(0, 0);
    let mut state = synced_state(&[(key, area())]);
    add(&mut state, "A", key, true, area());
    add(&mut state, "B", key, false, area());

    toggle_maximize(&mut state, "A");
    let response = state
        .apply(Command::FocusDirection {
            window_id: id("A"),
            direction: Direction::Right,
        })
        .unwrap();
    assert!(
        matches!(response, Response::PlacementsAndFocus { window_id, .. } if window_id == id("B"))
    );
    assert_eq!(state.maximized_window(key), None);

    toggle_maximize(&mut state, "B");
    state
        .apply(Command::MoveDirection {
            window_id: id("B"),
            direction: Direction::Left,
        })
        .unwrap();
    assert_eq!(state.maximized_window(key), None);

    toggle_maximize(&mut state, "B");
    state
        .apply(Command::Resize {
            window_id: id("B"),
            direction: Direction::Right,
        })
        .unwrap();
    assert_eq!(state.maximized_window(key), None);

    toggle_maximize(&mut state, "B");
    state
        .apply(Command::ToggleFloating { window_id: id("B") })
        .unwrap();
    assert_eq!(state.maximized_window(key), None);
    assert_eq!(state.is_floating(&id("B")), Some(true));
    assert_eq!(
        state
            .apply(Command::ToggleMaximize { window_id: id("B") })
            .unwrap(),
        Response::Ack
    );
    state.validate_invariants().unwrap();
}

#[test]
fn complex_multi_context_sequence_validates_after_every_mutation() {
    let c0 = context(0, 0);
    let c1 = context(0, 1);
    let w1 = context(1, 0);
    let mut state = synced_state(&[(c0, area()), (c1, area()), (w1, area())]);
    let mut check = |command| {
        state.apply(command).unwrap();
        state.validate_invariants().unwrap();
    };

    check(Command::AddWindow {
        window: window("A", c0, true),
        work_area: area(),
    });
    check(Command::AddWindow {
        window: window("B", c0, true),
        work_area: area(),
    });
    check(Command::AddWindow {
        window: window("C", c1, true),
        work_area: area(),
    });
    check(Command::AddWindow {
        window: window("D", c1, false),
        work_area: area(),
    });
    check(Command::ToggleMaximize { window_id: id("B") });
    check(Command::ToggleMaximize { window_id: id("C") });
    check(Command::RemoveWindow { window_id: id("A") });
    check(Command::AddWindow {
        window: window("E", c1, false),
        work_area: area(),
    });
    check(Command::WindowContextChanged {
        window_id: id("B"),
        context: c1,
        work_area: area(),
    });
    check(Command::Relayout { context: Some(w1) });
    check(Command::Relayout { context: Some(c1) });
    check(Command::ToggleMaximize { window_id: id("D") });
    check(Command::ToggleFloating { window_id: id("D") });
}
