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
    Extension: class {},
    GLib,
    console,
    global: {},
};
vm.runInNewContext(source, sandbox);
const ExtensionClass = sandbox.DwindleExtension;

{
    const extension = Object.create(ExtensionClass.prototype);
    const window = {};
    let scheduled;
    let syncs = 0;
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
