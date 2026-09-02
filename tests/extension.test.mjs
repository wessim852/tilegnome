import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const callbacks = [];
const GLib = {
    PRIORITY_DEFAULT: 0,
    PRIORITY_DEFAULT_IDLE: 0,
    SOURCE_REMOVE: false,
    idle_add(_priority, callback) {
        callbacks.push(callback);
        return callbacks.length;
    },
    timeout_add(_priority, _delay, callback) {
        callbacks.push(callback);
        return callbacks.length;
    },
    source_remove() {},
};
let source = readFileSync(new URL('../extension/extension.js', import.meta.url), 'utf8');
source = source
    .replace(/^import[\s\S]*?;\n/gm, '')
    .replace('export default class DwindleExtension', 'class DwindleExtension');
source += '\nglobalThis.DwindleExtension = DwindleExtension;';
const sandbox = {
    CONFIG_KEYS: ['inner-gap', 'outer-gap', 'split-ratio', 'resize-step', 'smart-split'],
    Extension: class {},
    GLib,
    console,
    global: {},
    ignoredApps: () => new Set(),
    isTileCandidate: () => true,
    readConfig: settings => settings.config,
    shouldTile: () => false,
};
vm.runInNewContext(source, sandbox);
const ExtensionClass = sandbox.DwindleExtension;

{
    const extension = Object.create(ExtensionClass.prototype);
    const window = {};
    let scheduled;
    let syncs = 0;
    extension._readinessExhausted = new Set();
    extension._scheduleWindow = (candidate, callback) => {
        assert.equal(candidate, window);
        scheduled = callback;
    };
    extension._syncEligibility = candidate => {
        assert.equal(candidate, window);
        syncs++;
    };
    extension._scheduleContext(window);
    scheduled();
    assert.equal(syncs, 1);
}

{
    const sticky = {is_on_all_workspaces: () => true};
    const ordinary = {is_on_all_workspaces: () => false};
    const scheduled = [];
    let focusChanges = 0;
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _registry: {values: () => [sticky, ordinary]},
        _onFocusChanged() {
            focusChanges++;
        },
        _scheduleContext(window) {
            scheduled.push(window);
        },
    });
    extension._onActiveWorkspaceChanged();
    assert.equal(focusChanges, 1);
    assert.deepEqual(scheduled, [sticky]);
}

{
    const display = {};
    const monitorManager = {};
    const workspaceManager = {};
    const config = {};
    const settings = {config};
    const handlers = new Map();
    let fullSyncs = 0;
    let sent;
    sandbox.global.display = display;
    sandbox.global.workspace_manager = workspaceManager;
    sandbox.global.backend = {get_monitor_manager: () => monitorManager};
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _settings: settings,
        _readinessExhausted: new Set(),
        _signals: {
            connect(object, signal, callback) {
                if (object === settings)
                    handlers.set(signal, callback);
            },
        },
        _send(command) {
            sent = command;
        },
        _scheduleFullSync() {
            fullSyncs++;
        },
    });
    extension._connectSignals();
    handlers.get('changed::inner-gap')();
    assert.equal(sent.command, 'configure');
    assert.equal(sent.config, config);
    assert.equal(fullSyncs, 0);
    handlers.get('changed::enabled')();
    assert.equal(fullSyncs, 1);
}

{
    let release;
    const gate = new Promise(resolve => {
        release = resolve;
    });
    let requests = 0;
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _generation: 1,
        _destroyed: false,
        _queue: gate,
        _client: {
            request() {
                requests++;
                return Promise.resolve('{"type":"ack"}');
            },
        },
    });
    extension._send({command: 'full_sync'});
    const queued = extension._queue;
    extension._generation++;
    release();
    await queued;
    assert.equal(requests, 0);
}

{
    let respond;
    let handled = 0;
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _generation: 1,
        _destroyed: false,
        _daemonWarned: false,
        _queue: Promise.resolve(),
        _client: {
            request() {
                return new Promise(resolve => {
                    respond = resolve;
                });
            },
        },
        _handleResponse() {
            handled++;
        },
    });
    extension._send({command: 'full_sync'});
    const queued = extension._queue;
    await Promise.resolve();
    extension._generation++;
    respond('{"type":"ack"}');
    await queued;
    assert.equal(handled, 0);
}

{
    const rect = {x: 1, y: 2, width: 300, height: 200};
    let frame = {x: 0, y: 0, width: 100, height: 100};
    let moves = 0;
    const window = {
        get_frame_rect: () => frame,
        is_fullscreen: () => false,
        is_maximized: () => false,
        move_resize_frame(_userOperation, x, y, width, height) {
            moves++;
            frame = {x, y, width, height};
        },
    };
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _destroyed: false,
        _placementSources: new Map(),
        _maximizedWindows: new Set(),
        _registry: {has: candidate => candidate === window},
        _updateBorder() {},
    });
    extension._placeWindow(window, rect, 0);
    assert.equal(moves, 1);
    assert.equal(callbacks.length, 1);
    callbacks.shift()();
    assert.equal(moves, 1);
    assert.equal(extension._placementSources.size, 0);
}

{
    const window = {};
    let contextRefreshes = 0;
    let fullSyncs = 0;
    sandbox.workAreaFor = () => ({x: 0, y: 0, width: 100, height: 100});
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _registry: {get: () => window},
        _stalePlacements: new Set(),
        _safeRect: () => false,
        _scheduleContext() { contextRefreshes++; },
        _scheduleFullSync() {
            fullSyncs++;
        },
    });
    extension._applyPlacements([{
        window_id: '1',
        rect: {x: 200, y: 0, width: 100, height: 100},
    }]);
    assert.equal(contextRefreshes, 0);
    assert.equal(fullSyncs, 0);
}

{
    let maximized = false;
    let placementsCancelled = 0;
    let contextRefreshes = 0;
    let borderUpdates = 0;
    const window = {
        is_maximized: () => maximized,
        maximize() {
            maximized = true;
        },
        unmaximize() {
            maximized = false;
        },
    };
    sandbox.global.display = {get_focus_window: () => window};
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _settings: {get_boolean: () => true},
        _maximizedWindows: new Set(),
        _cancelPlacement: () => placementsCancelled++,
        _scheduleWindow: (_window, callback) => callback(),
        _scheduleContext: () => contextRefreshes++,
        _updateBorder: () => borderUpdates++,
    });
    extension._onKeybinding('toggle_maximize');
    assert.equal(maximized, true);
    assert.equal(extension._maximizedWindows.has(window), true);
    assert.equal(placementsCancelled, 1);
    assert.equal(borderUpdates, 1);
    extension._onKeybinding('toggle_maximize');
    assert.equal(maximized, false);
    assert.equal(extension._maximizedWindows.has(window), false);
    assert.equal(contextRefreshes, 1);
}

{
    let reconciliations = 0;
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _scheduleFullSync() {
            reconciliations++;
        },
    });
    extension._handleResponse({type: 'error', message: 'rejected'}, {command: 'add_window'});
    assert.equal(reconciliations, 1);
    extension._handleResponse({type: 'error', message: 'rejected'}, {command: 'full_sync'});
    assert.equal(reconciliations, 1);
}

{
    const window = {};
    let schedules = 0;
    const extension = Object.assign(Object.create(ExtensionClass.prototype), {
        _destroyed: false,
        _settings: {get_boolean: () => true},
        _registry: {has: () => false},
        _readinessExhausted: new Set(),
        _scheduleWindow() {
            schedules++;
        },
    });
    extension._syncEligibility(window, 50);
    assert.equal(extension._readinessExhausted.has(window), true);
    assert.equal(schedules, 0);
    extension._scheduleEligibility(window);
    assert.equal(extension._readinessExhausted.has(window), false);
    assert.equal(schedules, 1);
}

{
    let windowsSource = readFileSync(new URL('../extension/windows.js', import.meta.url), 'utf8');
    windowsSource = windowsSource
        .replace(/^import .*;\n/gm, '')
        .replaceAll('export ', '');
    windowsSource += `
globalThis.WindowRegistry = WindowRegistry;
globalThis.isTileCandidate = isTileCandidate;
globalThis.minimumSize = minimumSize;
globalThis.shouldTile = shouldTile;
globalThis.windowContextReady = windowContextReady;`;
    const windowsSandbox = {
        Meta: {WindowType: {NORMAL: 0}},
        global: {
            display: {get_n_monitors: () => 2},
            workspace_manager: {
                n_workspaces: 3,
                get_active_workspace: () => ({index: () => 1}),
            },
        },
    };
    vm.runInNewContext(windowsSource, windowsSandbox);
    let windowAlive = true;
    const managedWindow = {
        get_stable_sequence() {
            if (!windowAlive)
                throw new Error('window finalized');
            return 42;
        },
    };
    const registry = new windowsSandbox.WindowRegistry();
    registry.add(managedWindow);
    windowAlive = false;
    assert.equal(registry.has(managedWindow), true);
    assert.equal(registry.remove(managedWindow), '42');
    assert.equal(registry.get('42'), undefined);

    const window = {
        get_monitor: () => 1,
        get_workspace: () => ({index: () => 2}),
        is_on_all_workspaces: () => false,
    };
    assert.equal(windowsSandbox.windowContextReady(window), true);
    window.get_monitor = () => 2;
    assert.equal(windowsSandbox.windowContextReady(window), false);
    window.get_monitor = () => 1;
    window.get_workspace = () => null;
    assert.equal(windowsSandbox.windowContextReady(window), true);

    let resizeable = false;
    const appWindow = {
        get_gtk_application_id: () => 'com.brave.Browser',
        get_sandboxed_app_id: () => null,
        get_wm_class: () => 'brave-browser',
        get_window_type: () => 0,
        is_attached_dialog: () => false,
        is_override_redirect: () => false,
        is_skip_taskbar: () => false,
        is_fullscreen: () => false,
        allows_move: () => true,
        allows_resize: () => resizeable,
        get_monitor: () => 1,
        get_workspace: () => ({index: () => 2}),
        get_min_size: () => [true, 800, 600],
    };
    assert.equal(windowsSandbox.isTileCandidate(appWindow, new Set()), true);
    assert.equal(windowsSandbox.shouldTile(appWindow, new Set()), false);
    resizeable = true;
    assert.equal(windowsSandbox.shouldTile(appWindow, new Set()), true);
    assert.deepEqual(
        {...windowsSandbox.minimumSize(appWindow)},
        {min_width: 800, min_height: 600}
    );
}

{
    let bordersSource = readFileSync(new URL('../extension/borders.js', import.meta.url), 'utf8');
    bordersSource = bordersSource
        .replace(/^import .*;\n/gm, '')
        .replaceAll('export ', '');
    bordersSource += '\nglobalThis.WindowBorders = WindowBorders;';
    const borderSandbox = {
        St: {
            Widget: class {
                set_position(x, y) {
                    this.position = {x, y};
                }

                set_size(width, height) {
                    this.size = {width, height};
                }

                set_style() {}
                destroy() {}
            },
        },
    };
    vm.runInNewContext(bordersSource, borderSandbox);
    const windowActor = {
        add_child(actor) {
            this.child = actor;
        },
        set_child_above_sibling() {},
    };
    let fullscreen = false;
    const window = {
        get_compositor_private: () => windowActor,
        get_frame_rect: () => ({x: 10, y: 20, width: 900, height: 600}),
        get_buffer_rect: () => ({x: -30, y: -20, width: 980, height: 680}),
        is_fullscreen: () => fullscreen,
    };
    const borders = new borderSandbox.WindowBorders();
    borders.update(window, true);
    assert.deepEqual({...windowActor.child.position}, {x: 40, y: 40});
    assert.deepEqual({...windowActor.child.size}, {width: 900, height: 600});
    fullscreen = true;
    borders.update(window, true);
    assert.equal(borders._actors.size, 0);
}
