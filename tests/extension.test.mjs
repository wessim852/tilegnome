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
    readConfig: settings => settings.config,
    shouldTile: () => true,
    windowContextReady: () => false,
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
    windowsSource += '\nglobalThis.windowContextReady = windowContextReady;';
    const windowsSandbox = {
        Meta: {},
        global: {
            display: {get_n_monitors: () => 2},
            workspace_manager: {
                n_workspaces: 3,
                get_active_workspace: () => ({index: () => 1}),
            },
        },
    };
    vm.runInNewContext(windowsSource, windowsSandbox);
    const window = {
        get_monitor: () => 1,
        get_workspace: () => ({index: () => 2}),
        is_on_all_workspaces: () => false,
    };
    assert.equal(windowsSandbox.windowContextReady(window), true);
    window.get_monitor = () => 2;
    assert.equal(windowsSandbox.windowContextReady(window), false);
}
