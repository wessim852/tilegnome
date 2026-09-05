import Meta from 'gi://Meta';

export class SignalManager {
    constructor() {
        this._signals = new Map();
    }

    connect(object, signal, callback) {
        const id = object.connect(signal, callback);
        const ids = this._signals.get(object) ?? [];
        ids.push(id);
        this._signals.set(object, ids);
        return id;
    }

    disconnectObject(object) {
        for (const id of this._signals.get(object) ?? []) {
            try {
                object.disconnect(id);
            } catch {
                // The object may already be finalized after `unmanaged`.
            }
        }
        this._signals.delete(object);
    }

    clear() {
        for (const object of [...this._signals.keys()])
            this.disconnectObject(object);
    }
}

export class WindowRegistry {
    constructor() {
        this._windows = new Map();
        this._ids = new Map();
    }

    add(window) {
        const id = windowId(window);
        this._windows.set(id, window);
        this._ids.set(window, id);
    }

    remove(window) {
        const id = this._ids.get(window);
        if (id === undefined)
            return null;
        this._windows.delete(id);
        this._ids.delete(window);
        return id;
    }

    get(id) {
        return this._windows.get(String(id));
    }

    values() {
        return this._windows.values();
    }

    has(window) {
        return this._ids.has(window);
    }

    clear() {
        this._windows.clear();
        this._ids.clear();
    }
}

export function windowId(window) {
    return String(window.get_stable_sequence());
}

export function applicationId(window) {
    return window.get_gtk_application_id()
        ?? window.get_sandboxed_app_id()
        ?? window.get_wm_class()
        ?? null;
}

export function isTileCandidate(window, ignored) {
    try {
        const appId = applicationId(window)?.toLowerCase();
        return window.get_window_type() === Meta.WindowType.NORMAL
            && !window.is_attached_dialog()
            && !window.is_override_redirect()
            && !window.is_skip_taskbar()
            && !window.is_fullscreen()
            && !(appId && ignored.has(appId));
    } catch {
        return false;
    }
}

export function shouldTile(window, ignored) {
    try {
        return isTileCandidate(window, ignored)
            && windowContextReady(window)
            && window.allows_move()
            // Mutter's allows_resize() is false for maximized windows even when
            // the client is resizable. Admit them so placement can unmaximize.
            && (window.allows_resize() || (window.is_maximized() && window.resizeable));
    } catch {
        return false;
    }
}

export function windowContextReady(window) {
    try {
        const monitor = window.get_monitor();
        const workspace = workspaceFor(window);
        const workspaceIndex = workspace?.index() ?? -1;
        return monitor >= 0
            && monitor < global.display.get_n_monitors()
            && workspaceIndex >= 0
            && workspaceIndex < global.workspace_manager.n_workspaces;
    } catch {
        return false;
    }
}

export function workspaceFor(window) {
    // Mutter can briefly expose a mapped normal window before assigning its workspace.
    return window.get_workspace() ?? global.workspace_manager.get_active_workspace();
}

export function contextFor(window) {
    return {
        workspace: workspaceFor(window).index(),
        monitor: window.get_monitor(),
    };
}

export function rectFromMutter(rect) {
    return {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
}

export function workAreaFor(window) {
    return rectFromMutter(
        workspaceFor(window).get_work_area_for_monitor(window.get_monitor())
    );
}

export function minimumSize(window) {
    try {
        const [known, width, height] = window.get_min_size();
        if (known)
            return {min_width: Math.max(0, width), min_height: Math.max(0, height)};
    } catch {
        // Size hints are optional; zero means unconstrained to the Rust engine.
    }
    return {min_width: 0, min_height: 0};
}

export function snapshotWindow(window) {
    return {
        id: windowId(window),
        context: contextFor(window),
        app_id: applicationId(window),
        frame_rect: rectFromMutter(window.get_frame_rect()),
        ...minimumSize(window),
        fullscreen: window.is_fullscreen(),
        window_type: window.get_window_type() === Meta.WindowType.NORMAL ? 'normal' : 'other',
        focused: global.display.get_focus_window() === window,
    };
}
