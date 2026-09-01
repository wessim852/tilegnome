import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BINDINGS = {
    'dwindle-focus-left': ['focus_direction', 'left'],
    'dwindle-focus-down': ['focus_direction', 'down'],
    'dwindle-focus-up': ['focus_direction', 'up'],
    'dwindle-focus-right': ['focus_direction', 'right'],
    'dwindle-move-left': ['move_direction', 'left'],
    'dwindle-move-down': ['move_direction', 'down'],
    'dwindle-move-up': ['move_direction', 'up'],
    'dwindle-move-right': ['move_direction', 'right'],
    'dwindle-resize-left': ['resize', 'left'],
    'dwindle-resize-down': ['resize', 'down'],
    'dwindle-resize-up': ['resize', 'up'],
    'dwindle-resize-right': ['resize', 'right'],
    'dwindle-toggle-floating': ['toggle_floating', null],
    'dwindle-toggle-fullscreen': ['toggle_fullscreen', null],
    'dwindle-toggle-group': ['toggle_group', null],
    'dwindle-group-next': ['cycle_group', 'next'],
    'dwindle-group-previous': ['cycle_group', 'previous'],
};

export class Keybindings {
    constructor(settings, callback) {
        this._names = Object.keys(BINDINGS);
        for (const [name, [action, direction]] of Object.entries(BINDINGS)) {
            const result = Main.wm.addKeybinding(
                name,
                settings,
                Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
                Shell.ActionMode.NORMAL,
                () => callback(action, direction)
            );
            if (result === Meta.KeyBindingAction.NONE)
                console.warn(`[DwindleRS] keybinding unavailable: ${name}`);
        }
    }

    destroy() {
        for (const name of this._names)
            Main.wm.removeKeybinding(name);
        this._names = [];
    }
}
