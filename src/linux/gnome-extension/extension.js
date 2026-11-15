// Wortfluss listening indicator for GNOME Shell.
//
// Wortfluss (https://github.com/1a-khan/wortfluss) calls Frame() about 30 times a second
// while it listens or works, and Hide() when done. This extension draws a small bubble
// with a sound wave next to the text cursor of the focused app. The cursor position
// comes from the accessibility bus (AT-SPI), like the screen magnifier uses it; apps that
// don't report it get the bubble at the bottom center of the screen.
//
// Copyright Ammad Khan, MIAK IT. Apache License 2.0.

import Atspi from 'gi://Atspi';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {FocusCaretTracker} from 'resource:///org/gnome/shell/ui/focusCaretTracker.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'de.miakit.Wortfluss';
const OBJECT_PATH = '/de/miakit/Wortfluss/Indicator';
const IFACE = `
<node>
  <interface name="de.miakit.Wortfluss.Indicator">
    <method name="Frame">
      <arg type="b" name="working" direction="in"/>
      <arg type="d" name="level" direction="in"/>
      <arg type="d" name="t" direction="in"/>
    </method>
    <method name="Hide"/>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

const BARS = 7; // same as indicator::BARS in Wortfluss
const BAR_MAX = 16;
const GAP = 8; // between text cursor and bubble
const WATCHDOG_MS = 1500; // hide if Wortfluss stops sending frames
const FOLLOW_MS = 150; // look for the text cursor this often while visible
const SLOW_MS = 100; // an app slower than this to answer: stop following until next time

// Bar heights 0..1, kept in sync with indicator::bars() in Wortfluss.
function heights(working, level, t) {
    const mid = (BARS - 1) / 2;
    const h = [];
    for (let i = 0; i < BARS; i++) {
        if (working) {
            h.push(0.18 + 0.32 * Math.max(0, Math.sin(t * 7 - i * 0.9)));
        } else {
            const envelope = 1 - Math.pow(Math.abs(i - mid) / (mid + 1), 1.5);
            const wobble = 0.65 + 0.35 * Math.sin(t * 9 + i * 1.3);
            h.push(0.18 + 0.82 * level * envelope * wobble);
        }
    }
    return h;
}

export default class WortflussExtension extends Extension {
    enable() {
        this._bubble = new St.BoxLayout({
            style_class: 'wortfluss-bubble',
            reactive: false,
            visible: false,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._bars = [];
        for (let i = 0; i < BARS; i++) {
            const bar = new St.Widget({
                style_class: 'wortfluss-bar',
                y_align: Clutter.ActorAlign.CENTER,
                height: 3,
            });
            this._bars.push(bar);
            this._bubble.add_child(bar);
        }
        Main.layoutManager.addTopChrome(this._bubble); // not reactive: clicks go through

        // Remember the focused widget; follow the text cursor only while visible.
        this._focused = null;
        this._placedAt = 0;
        this._slow = false;
        this._tracker = new FocusCaretTracker();
        this._focusId = this._tracker.connect('focus-changed', (_t, event) => {
            if (event.detail1 === 1)
                this._focused = event.source;
        });
        this._caretId = this._tracker.connect('caret-moved', (_t, event) => {
            this._focused = event.source;
        });
        this._tracker.registerFocusListener();

        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name_on_connection(Gio.DBus.session, BUS_NAME,
            Gio.BusNameOwnerFlags.REPLACE, null, null);
    }

    disable() {
        Gio.bus_unown_name(this._nameId);
        this._dbus.unexport();
        this._dbus = null;
        this._tracker.deregisterCaretListener();
        this._tracker.deregisterFocusListener();
        this._tracker.disconnect(this._focusId);
        this._tracker.disconnect(this._caretId);
        this._tracker = null;
        this._focused = null;
        this._clearWatchdog();
        this._bubble.destroy();
        this._bubble = null;
        this._bars = null;
    }

    get Version() {
        return 1;
    }

    Frame(working, level, t) {
        if (!this._bubble)
            return;
        const now = GLib.get_monotonic_time() / 1000;
        if (!this._bubble.visible) {
            this._tracker.registerCaretListener();
            this._bubble.show();
            this._slow = false;
            this._place(now);
        } else if (!this._slow && now - this._placedAt >= FOLLOW_MS) {
            this._place(now); // text typed meanwhile moves the cursor
        }
        if (working)
            this._bubble.add_style_class_name('wortfluss-working');
        else
            this._bubble.remove_style_class_name('wortfluss-working');
        const h = heights(working, level, t);
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        this._bars.forEach((bar, i) => {
            bar.height = Math.max(3, Math.round(h[i] * BAR_MAX)) * scale;
        });
        this._clearWatchdog();
        this._watchdog = GLib.timeout_add(GLib.PRIORITY_DEFAULT, WATCHDOG_MS, () => {
            this._watchdog = 0;
            this.Hide();
            return GLib.SOURCE_REMOVE;
        });
    }

    Hide() {
        this._clearWatchdog();
        if (!this._bubble?.visible)
            return;
        this._bubble.hide();
        this._tracker.deregisterCaretListener();
    }

    _clearWatchdog() {
        if (this._watchdog) {
            GLib.source_remove(this._watchdog);
            this._watchdog = 0;
        }
    }

    // Next to the text cursor, else the focused field, else bottom center of the screen.
    _place(now = GLib.get_monotonic_time() / 1000) {
        const [w, h] = [this._bubble.width, this._bubble.height];
        const caret = this._caretRect();
        this._placedAt = GLib.get_monotonic_time() / 1000;
        // Asking the app is a blocking call: never let a slow app make the desktop stutter.
        if (this._placedAt - now > SLOW_MS)
            this._slow = true;
        const monitor = Main.layoutManager.currentMonitor ?? Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;
        let x, y;
        if (caret) {
            x = caret.x + caret.width + GAP;
            y = caret.y + caret.height / 2 - h / 2;
        } else {
            x = monitor.x + monitor.width / 2 - w / 2;
            y = monitor.y + monitor.height - h - 96;
        }
        x = Math.min(Math.max(x, monitor.x + 4), monitor.x + monitor.width - w - 4);
        y = Math.min(Math.max(y, monitor.y + 4), monitor.y + monitor.height - h - 4);
        this._bubble.set_position(Math.round(x), Math.round(y));
    }

    _caretRect() {
        const acc = this._focused;
        if (!acc)
            return null;
        try {
            const text = acc.get_text_iface();
            const count = text ? text.get_character_count() : 0;
            if (text && count > 0) {
                // The cursor sits between two characters. Measure the one before it
                // (its right edge), unless that is a line break: then the one after it.
                // (Apps report a line break at the start of the next line.)
                const at = text.get_caret_offset();
                // (Atspi.Accessible has its own get_text(), which hides the Text one.)
                const before = at > 0 ? Atspi.Text.prototype.get_text.call(text, at - 1, at) : '\n';
                if (before !== '\n') {
                    const r = text.get_character_extents(at - 1, Atspi.CoordType.WINDOW);
                    if (r.height > 0)
                        return this._toScreen(acc, {x: r.x + r.width, y: r.y, width: 0, height: r.height});
                } else if (at < count) {
                    // Start of a line: the character after the cursor.
                    const r = text.get_character_extents(at, Atspi.CoordType.WINDOW);
                    if (r.height > 0)
                        return this._toScreen(acc, {x: r.x, y: r.y, width: 0, height: r.height});
                } else {
                    // Empty last line: the final line break has no size, so one line
                    // below the character before it, at the left margin.
                    const r = text.get_character_extents(Math.max(at - 2, 0), Atspi.CoordType.WINDOW);
                    const first = text.get_character_extents(0, Atspi.CoordType.WINDOW);
                    if (r.height > 0)
                        return this._toScreen(acc, {x: first.x, y: r.y + r.height, width: 0, height: r.height});
                }
            }
            const component = acc.get_component_iface();
            if (component) {
                // Empty field or no cursor position: the left end of the focused field.
                const r = component.get_extents(Atspi.CoordType.WINDOW);
                if (r.width > 0 && r.height > 0)
                    return this._toScreen(acc, {x: r.x + 4, y: r.y, width: 0, height: Math.min(r.height, 32)});
            }
        } catch (e) {
            // App gone or not answering: fall back to the screen position.
        }
        return null;
    }

    // Window coordinates of the focused app to screen coordinates (as the magnifier does).
    _toScreen(acc, r) {
        // Only for the app whose window has the focus.
        const win = global.display.focus_window;
        if (!win || acc.get_process_id() !== win.get_pid())
            return null;
        const frame = win.get_client_content_rect();
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        const rect = {
            x: frame.x + scale * r.x,
            y: frame.y + scale * r.y,
            width: scale * r.width,
            height: scale * r.height,
        };
        // Reject positions outside the focused window (some apps report nonsense).
        const outer = win.get_frame_rect();
        const inside = rect.x >= outer.x - 4 && rect.x <= outer.x + outer.width + 4 &&
            rect.y >= outer.y - 4 && rect.y <= outer.y + outer.height + 4;
        return inside ? rect : null;
    }
}
