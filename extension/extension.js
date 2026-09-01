import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {WindowBorders} from './borders.js';
import {EngineClient} from './dbus.js';
import {Keybindings} from './keybindings.js';
import {CONFIG_KEYS, ignoredApps, readConfig} from './settings.js';
import {
    SignalManager,
    WindowRegistry,
    contextFor,
    rectFromMutter,
    shouldTile,
    snapshotWindow,
    windowContextReady,
    windowId,
    workAreaFor,
} from './windows.js';

const WINDOW_READY_RETRIES = 50;
const WINDOW_READY_RETRY_MS = 100;

export default class DwindleExtension extends Extension {
    enable() {
        this._destroyed = false;
        this._daemonWarned = false;
        this._queue = Promise.resolve();
        this._signals = new SignalManager();
        this._registry = new WindowRegistry();
        this._borders = new WindowBorders();
        this._trackedWindows = new Set();
        this._windowSources = new Map();
        this._placementSources = new Map();
        this._fullSyncSource = 0;
        this._settings = this.getSettings();

        this._client = new EngineClient(() => this._scheduleFullSync());
        this._keybindings = new Keybindings(
            this._settings,
            (action, direction) => this._onKeybinding(action, direction)
        );
        this._connectSignals();
        this._scheduleFullSync();
        console.log('[DwindleRS] enabled');
    }

    disable() {
        this._destroyed = true;
        if (this._fullSyncSource)
            GLib.source_remove(this._fullSyncSource);
        this._fullSyncSource = 0;
        for (const source of this._windowSources.values())
            GLib.source_remove(source);
        this._windowSources.clear();
        for (const source of this._placementSources.values())
            GLib.source_remove(source);
        this._placementSources.clear();
        this._signals.clear();
        this._keybindings.destroy();
        this._client.destroy();
        this._borders.clear();
        this._registry.clear();
        this._trackedWindows.clear();

        this._keybindings = null;
        this._client = null;
        this._registry = null;
        this._borders = null;
        this._signals = null;
        this._settings = null;
        console.log('[DwindleRS] disabled');
    }

    _connectSignals() {
        const display = global.display;
        this._signals.connect(display, 'window-created', (_display, window) => {
            this._scheduleWindow(window, () => {
                this._trackWindow(window);
                this._syncEligibility(window);
            });
        });
        this._signals.connect(display, 'notify::focus-window', () => this._onFocusChanged());
        this._signals.connect(display, 'window-entered-monitor', (_display, _monitor, window) => {
            this._scheduleContext(window);
        });
        this._signals.connect(display, 'window-left-monitor', (_display, _monitor, window) => {
            this._scheduleContext(window);
        });
        this._signals.connect(display, 'grab-op-end', (_display, window) => {
            this._scheduleContext(window);
        });
        this._signals.connect(display, 'workareas-changed', () => this._scheduleFullSync());

        const monitorManager = global.backend.get_monitor_manager();
        this._signals.connect(monitorManager, 'monitors-changed', () => this._scheduleFullSync());
        this._signals.connect(global.workspace_manager, 'notify::n-workspaces', () => {
            this._scheduleFullSync();
        });
        this._signals.connect(global.workspace_manager, 'active-workspace-changed', () => {
            this._onFocusChanged();
            this._scheduleFullSync();
        });

        for (const key of [...CONFIG_KEYS, 'enabled', 'ignored-apps']) {
            this._signals.connect(this._settings, `changed::${key}`, () => {
                this._scheduleFullSync();
            });
        }
    }

    _trackWindow(window) {
        if (this._trackedWindows.has(window))
            return;
        this._trackedWindows.add(window);
        this._signals.connect(window, 'unmanaged', () => this._onUnmanaged(window));
        this._signals.connect(window, 'workspace-changed', () => this._scheduleContext(window));
        this._signals.connect(window, 'notify::fullscreen', () => this._scheduleEligibility(window));
        this._signals.connect(window, 'notify::skip-taskbar', () => this._scheduleEligibility(window));
        this._signals.connect(window, 'notify::window-type', () => this._scheduleEligibility(window));
        this._signals.connect(window, 'notify::mapped', () => this._scheduleContext(window));
        this._signals.connect(window, 'notify::maximized-horizontally', () => this._scheduleContext(window));
        this._signals.connect(window, 'notify::maximized-vertically', () => this._scheduleContext(window));
    }

    _onUnmanaged(window) {
        this._cancelWindowSource(window);
        this._cancelPlacement(window);
        this._borders.remove(window);
        this._trackedWindows.delete(window);
        this._signals.disconnectObject(window);
        if (this._registry.has(window)) {
            const id = this._registry.remove(window);
            this._send({command: 'remove_window', window_id: id});
        }
    }

    _syncEligibility(window, attempt = 0) {
        if (this._destroyed)
            return;
        const eligible = this._settings.get_boolean('enabled')
            && shouldTile(window, ignoredApps(this._settings));
        if (eligible && !windowContextReady(window)) {
            // ponytail: five seconds covers slow Wayland clients; resync handles later events.
            if (attempt < WINDOW_READY_RETRIES) {
                this._scheduleWindow(
                    window,
                    () => this._syncEligibility(window, attempt + 1),
                    WINDOW_READY_RETRY_MS
                );
            } else {
                console.warn('[DwindleRS] window context did not become ready; resyncing');
                this._scheduleFullSync();
            }
            return;
        }
        const registered = this._registry.has(window);
        if (eligible && !registered) {
            this._registry.add(window);
            this._send({
                command: 'add_window',
                window: snapshotWindow(window),
                work_area: workAreaFor(window),
            });
        } else if (!eligible && registered) {
            const id = this._registry.remove(window);
            this._send({command: 'remove_window', window_id: id});
        } else if (eligible) {
            this._sendContext(window);
        }
        this._syncBorders();
    }

    _scheduleEligibility(window) {
        this._scheduleWindow(window, () => this._syncEligibility(window));
    }

    _scheduleContext(window) {
        this._scheduleWindow(window, () => {
            if (!windowContextReady(window))
                this._syncEligibility(window);
            else if (this._registry.has(window))
                this._sendContext(window);
            else
                this._syncEligibility(window);
        });
    }

    _sendContext(window) {
        try {
            this._send({
                command: 'window_context_changed',
                window_id: windowId(window),
                context: contextFor(window),
                work_area: workAreaFor(window),
            });
        } catch (error) {
            console.warn(`[DwindleRS] window context unavailable: ${error.message}`);
            this._scheduleFullSync();
        }
    }

    _scheduleWindow(window, callback, delay = 0) {
        if (this._destroyed || this._windowSources.has(window))
            return;
        const run = () => {
            this._windowSources.delete(window);
            try {
                callback();
            } catch (error) {
                console.warn(`[DwindleRS] window event skipped: ${error.message}`);
                this._scheduleFullSync();
            }
            return GLib.SOURCE_REMOVE;
        };
        const source = delay
            ? GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, run)
            : GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, run);
        this._windowSources.set(window, source);
    }

    _cancelWindowSource(window) {
        const source = this._windowSources.get(window);
        if (source)
            GLib.source_remove(source);
        this._windowSources.delete(window);
    }

    _scheduleFullSync() {
        if (this._destroyed || this._fullSyncSource)
            return;
        this._fullSyncSource = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._fullSyncSource = 0;
            try {
                this._fullSync();
            } catch (error) {
                console.warn(`[DwindleRS] FullSync skipped: ${error.message}`);
            }
            return GLib.SOURCE_REMOVE;
        });
    }

    _fullSync() {
        const current = new Set(global.display.list_all_windows());
        for (const window of current)
            this._trackWindow(window);
        for (const window of [...this._trackedWindows]) {
            if (!current.has(window)) {
                this._signals.disconnectObject(window);
                this._trackedWindows.delete(window);
            }
        }

        this._registry.clear();
        const ignored = ignoredApps(this._settings);
        const enabled = this._settings.get_boolean('enabled');
        const windows = [];
        if (enabled) {
            for (const window of current) {
                if (!windowContextReady(window)) {
                    if (shouldTile(window, ignored))
                        this._scheduleEligibility(window);
                } else if (shouldTile(window, ignored)) {
                    this._registry.add(window);
                    windows.push(snapshotWindow(window));
                }
            }
        }
        this._syncBorders();
        this._send({
            command: 'full_sync',
            snapshot: {
                windows,
                work_areas: this._workAreas(),
            },
            config: readConfig(this._settings),
        });
    }

    _workAreas() {
        const areas = [];
        const monitorCount = global.display.get_n_monitors();
        for (let workspaceIndex = 0;
            workspaceIndex < global.workspace_manager.n_workspaces;
            workspaceIndex++) {
            const workspace = global.workspace_manager.get_workspace_by_index(workspaceIndex);
            for (let monitor = 0; monitor < monitorCount; monitor++) {
                areas.push({
                    context: {workspace: workspaceIndex, monitor},
                    rect: rectFromMutter(workspace.get_work_area_for_monitor(monitor)),
                });
            }
        }
        return areas;
    }

    _onFocusChanged() {
        const window = global.display.get_focus_window();
        this._syncBorders();
        if (window && this._registry.has(window)) {
            this._send({
                command: 'focus_window',
                window_id: windowId(window),
            });
        }
    }

    _onKeybinding(action, direction) {
        if (!this._settings.get_boolean('enabled'))
            return;
        const window = global.display.get_focus_window();
        if (!window)
            return;
        if (action === 'toggle_fullscreen') {
            if (window.is_fullscreen())
                window.unmake_fullscreen();
            else
                window.make_fullscreen();
            return;
        }
        if (!this._registry.has(window))
            return;
        const command = {
            command: action,
            window_id: windowId(window),
        };
        if (action === 'cycle_group')
            command.cycle = direction;
        else if (direction)
            command.direction = direction;
        this._send(command);
    }

    _send(command) {
        this._queue = this._queue.then(async () => {
            if (this._destroyed)
                return;
            try {
                const json = await this._client.request(command);
                if (this._destroyed)
                    return;
                this._daemonWarned = false;
                this._handleResponse(JSON.parse(json));
            } catch (error) {
                if (!this._daemonWarned) {
                    console.warn(`[DwindleRS] request failed; tiling paused: ${error.message}`);
                    this._daemonWarned = true;
                }
            }
        });
    }

    _handleResponse(response) {
        if (!response || typeof response !== 'object' || typeof response.type !== 'string')
            throw new Error('malformed daemon response');
        switch (response.type) {
        case 'ack':
            break;
        case 'placements':
            this._applyPlacements(response.placements);
            break;
        case 'focus':
            this._focus(response.window_id);
            break;
        case 'placements_and_focus':
            this._applyPlacements(response.placements);
            this._focus(response.window_id);
            break;
        case 'error':
            console.warn(`[DwindleRS] engine error: ${String(response.message)}`);
            break;
        default:
            throw new Error(`unknown daemon response: ${response.type}`);
        }
    }

    _applyPlacements(placements) {
        if (!Array.isArray(placements))
            throw new Error('placements are not an array');
        let missing = false;
        for (const placement of placements) {
            if (!placement || typeof placement.window_id !== 'string')
                throw new Error('unsafe placement from daemon');
            const window = this._registry.get(placement.window_id);
            if (!window) {
                missing = true;
                continue;
            }
            try {
                if (!this._safeRect(placement.rect, [workAreaFor(window)]))
                    throw new Error('rectangle is outside the window context work area');
                this._queuePlacement(window, placement.rect);
            } catch (error) {
                console.warn(`[DwindleRS] placement skipped: ${error.message}`);
                missing = true;
            }
        }
        if (missing)
            this._scheduleFullSync();
    }

    _queuePlacement(window, rect) {
        this._cancelPlacement(window);
        this._placeWindow(window, rect, 0);
    }

    _placeWindow(window, rect, attempt) {
        if (this._destroyed || !this._registry.has(window))
            return;
        try {
            if (window.is_fullscreen())
                return;
            if (window.is_maximized())
                window.unmaximize();
            const {x, y, width, height} = rect;
            window.move_resize_frame(false, x, y, width, height);
            this._updateBorder(window);
        } catch (error) {
            console.warn(`[DwindleRS] placement skipped: ${error.message}`);
            this._scheduleFullSync();
            return;
        }
        // ponytail: six retries cover Wayland map/maximize handshakes; use
        // configure acknowledgements if a client can still override after 450 ms.
        if (attempt >= 5)
            return;
        const source = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 75, () => {
            this._placementSources.delete(window);
            this._placeWindow(window, rect, attempt + 1);
            return GLib.SOURCE_REMOVE;
        });
        this._placementSources.set(window, source);
    }

    _cancelPlacement(window) {
        const source = this._placementSources.get(window);
        if (source)
            GLib.source_remove(source);
        this._placementSources.delete(window);
    }

    _updateBorder(window) {
        if (this._registry.has(window))
            this._borders.update(window, global.display.get_focus_window() === window);
    }

    _syncBorders() {
        this._borders.sync(this._registry.values(), global.display.get_focus_window());
    }

    _safeRect(rect, workAreas) {
        if (!rect || !['x', 'y', 'width', 'height'].every(key => Number.isInteger(rect[key])))
            return false;
        if (rect.width <= 0 || rect.height <= 0)
            return false;
        return workAreas.some(area => rect.x >= area.x && rect.y >= area.y
            && rect.x + rect.width <= area.x + area.width
            && rect.y + rect.height <= area.y + area.height);
    }

    _focus(id) {
        if (typeof id !== 'string')
            throw new Error('invalid focus target');
        const window = this._registry.get(id);
        if (window)
            window.activate(global.get_current_time());
        else
            this._scheduleFullSync();
    }
}
