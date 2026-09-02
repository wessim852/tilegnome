# Architecture

`dwindle-core` is the source of truth. It owns typed protocol data, an arena-backed binary split tree, geometry, per-workspace/per-monitor floating and layout-maximize state, navigation, swapping, resizing, reconciliation, and invariant checks. Every leaf contains one tiled window and every internal node contains exactly two children; rectangles are derived from that tree.

`dwindle-daemon` owns `dev.dwindlers.Engine` on the user session bus. Its single `Request(string) -> string` method deserializes a typed command, mutates one `EngineState`, and serializes a typed response.

The GNOME 50 extension is an adapter only. It filters and tracks `Meta.Window` objects, converts Mutter state to protocol snapshots/events, applies validated placements with `Meta.Window.move_resize_frame()`, focuses daemon-selected windows, and owns Shell keybindings and signal cleanup. Super+F reports a typed command; Rust changes only that context's presentation state and returns the work-area placement. A full sync rebuilds Rust state after either side restarts or monitor topology changes, never for routine focus, workspace, tree, maximize, or configuration changes.

```text
GNOME Shell / Mutter
        | Meta.Window events and placements
        v
GNOME 50 GJS adapter
        | D-Bus session bus, one JSON request method
        v
Rust zbus daemon
        |
        v
Rust dwindle engine
```

Monitor indexes are context identifiers only for the lifetime of one topology. A topology change triggers a full rebuild, prioritizing correctness over preserving the exact old tree.
