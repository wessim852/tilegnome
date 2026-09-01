import Clutter from 'gi://Clutter';
import St from 'gi://St';

const NORMAL_STYLE = 'border: 2px solid rgba(150, 160, 175, 0.85); border-radius: 4px;';
const FOCUSED_STYLE = 'border: 3px solid #33d6ff; border-radius: 4px;';

export class WindowBorders {
    constructor() {
        this._actors = new Map();
    }

    sync(windows, focused) {
        const current = new Set(windows);
        for (const window of this._actors.keys()) {
            if (!current.has(window))
                this.remove(window);
        }
        for (const window of current)
            this.update(window, window === focused);
    }

    update(window, focused) {
        const windowActor = window.get_compositor_private();
        if (!windowActor) {
            this.remove(window);
            return;
        }
        let entry = this._actors.get(window);
        if (entry?.windowActor !== windowActor) {
            this.remove(window);
            const actor = new St.Widget({reactive: false, x: 0, y: 0});
            actor.add_constraint(new Clutter.BindConstraint({
                source: windowActor,
                coordinate: Clutter.BindCoordinate.SIZE,
            }));
            windowActor.add_child(actor);
            entry = {actor, windowActor};
            this._actors.set(window, entry);
        }
        entry.actor.set_style(focused ? FOCUSED_STYLE : NORMAL_STYLE);
        windowActor.set_child_above_sibling(entry.actor, null);
    }

    remove(window) {
        this._actors.get(window)?.actor.destroy();
        this._actors.delete(window);
    }

    clear() {
        for (const {actor} of this._actors.values())
            actor.destroy();
        this._actors.clear();
    }
}
