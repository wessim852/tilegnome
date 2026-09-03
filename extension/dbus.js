import Gio from 'gi://Gio';

const BUS_NAME = 'dev.dwindlers.Engine';
const OBJECT_PATH = '/dev/dwindlers/Engine';
const INTERFACE_XML = `
<node>
  <interface name="dev.dwindlers.Engine">
    <method name="Request">
      <arg type="s" name="json" direction="in"/>
      <arg type="s" name="json" direction="out"/>
    </method>
  </interface>
</node>`;

const EngineProxy = Gio.DBusProxy.makeProxyWrapper(INTERFACE_XML);

export class EngineClient {
    constructor(onAvailable) {
        this._onAvailable = onAvailable;
        this._ownerSignal = 0;
        this._hadOwner = false;
        this._destroyed = false;
        this._proxy = new EngineProxy(
            Gio.DBus.session,
            BUS_NAME,
            OBJECT_PATH,
            (proxy, error) => {
                if (this._destroyed)
                    return;
                if (error) {
                    console.warn(`[TileGNOME] D-Bus proxy unavailable: ${error.message}`);
                    return;
                }
                this._ownerSignal = proxy.connect(
                    'notify::g-name-owner',
                    () => this._ownerChanged()
                );
                this._ownerChanged();
            }
        );
    }

    get available() {
        return Boolean(this._proxy?.get_name_owner());
    }

    request(command) {
        if (!this.available)
            return Promise.reject(new Error('Rust daemon is not available'));
        return new Promise((resolve, reject) => {
            this._proxy.RequestRemote(JSON.stringify(command), (result, error) => {
                if (error)
                    reject(error);
                else
                    resolve(result[0]);
            });
        });
    }

    destroy() {
        this._destroyed = true;
        if (this._ownerSignal)
            this._proxy.disconnect(this._ownerSignal);
        this._ownerSignal = 0;
        this._proxy = null;
        this._onAvailable = null;
    }

    _ownerChanged() {
        const hasOwner = this.available;
        if (hasOwner && !this._hadOwner) {
            console.log('[TileGNOME] Rust daemon connected');
            this._onAvailable?.();
        } else if (!hasOwner && this._hadOwner) {
            console.warn('[TileGNOME] Rust daemon disconnected; tiling paused');
        }
        this._hadOwner = hasOwner;
    }
}
