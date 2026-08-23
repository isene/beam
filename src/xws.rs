//! The X side: which windows sit on the beam workspace, and telling the
//! WM to show one of them there.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;

pub struct X {
    conn: RustConnection,
    root: Window,
    atom_wm_desktop: Atom,
    atom_active: Atom,
    atom_class: Atom,
    atom_name: Atom,
}

pub struct Win {
    pub xid: Window,
    pub class: String,
    pub title: String,
    pub active: bool,
}

impl X {
    pub fn connect() -> Option<X> {
        let (conn, screen_num) = RustConnection::connect(None).ok()?;
        let root = conn.setup().roots[screen_num].root;
        let atom_wm_desktop = intern(&conn, b"_NET_WM_DESKTOP")?;
        let atom_active = intern(&conn, b"_NET_ACTIVE_WINDOW")?;
        let atom_class = AtomEnum::WM_CLASS.into();
        let atom_name = AtomEnum::WM_NAME.into();
        Some(X { conn, root, atom_wm_desktop, atom_active, atom_class, atom_name })
    }

    /// Every client on `ws` (0-based), with the one the WM shows marked.
    /// The WM's idea of "shown" is the tab it maps; we read the active
    /// window property per workspace from what is actually viewable.
    pub fn windows_on(&self, ws: u32) -> Vec<Win> {
        let mut out = Vec::new();
        let Ok(tree) = self.conn.query_tree(self.root) else { return out };
        let Ok(tree) = tree.reply() else { return out };
        // Queue everything first, one round trip for the lot.
        let mut cookies = Vec::new();
        for w in tree.children {
            let d = self.conn.get_property(
                false, w, self.atom_wm_desktop, AtomEnum::CARDINAL, 0, 1);
            let c = self.conn.get_property(
                false, w, self.atom_class, AtomEnum::STRING, 0, 64);
            let n = self.conn.get_property(
                false, w, self.atom_name, AtomEnum::STRING, 0, 64);
            let a = self.conn.get_window_attributes(w);
            cookies.push((w, d, c, n, a));
        }
        for (w, d, c, n, a) in cookies {
            let Some(desk) = d.ok().and_then(|k| k.reply().ok()).and_then(|r| {
                if r.format == 32 { r.value32().and_then(|mut v| v.next()) } else { None }
            }) else { continue };
            if desk != ws {
                continue;
            }
            let class = c.ok().and_then(|k| k.reply().ok())
                .map(|r| String::from_utf8_lossy(
                    r.value.split(|b| *b == 0).next().unwrap_or(&[])).to_string())
                .unwrap_or_default();
            let title = n.ok().and_then(|k| k.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.value).to_string())
                .unwrap_or_default();
            let viewable = a.ok().and_then(|k| k.reply().ok())
                .map(|r| r.map_state == x11rb::protocol::xproto::MapState::VIEWABLE)
                .unwrap_or(false);
            out.push(Win { xid: w, class, title, active: viewable });
        }
        out.sort_by_key(|w| w.xid);
        out
    }

    /// Ask the WM to show this window on its workspace: the EWMH
    /// _NET_ACTIVE_WINDOW ClientMessage on the root. tile raises the
    /// tab without switching the visible workspace or the focus.
    pub fn beam(&self, xid: Window) -> bool {
        let ev = ClientMessageEvent::new(32, xid, self.atom_active, [2, 0, 0, 0, 0]);
        let ok = self.conn.send_event(
            false, self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            ev,
        ).is_ok();
        ok && self.conn.flush().is_ok()
    }
}

fn intern(conn: &RustConnection, name: &[u8]) -> Option<Atom> {
    Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
}

impl X {
    /// Window geometry in pixels.
    pub fn geometry(&self, xid: Window) -> Option<(u16, u16)> {
        let g = self.conn.get_geometry(xid).ok()?.reply().ok()?;
        Some((g.width, g.height))
    }

    /// Grab the window's pixels and write a thumbnail as PPM (P6), which
    /// every image path downstream understands. Only works while the
    /// window is viewable: X keeps no pixels for an unmapped tab, which
    /// is why the scan key exists. Downscaled by integer box-sampling to
    /// at most `max_w` pixels wide.
    pub fn grab_ppm(&self, xid: Window, path: &std::path::Path, max_w: u32) -> bool {
        let Some((w, h)) = self.geometry(xid) else { return false };
        if w == 0 || h == 0 {
            return false;
        }
        let img = self.conn.get_image(
            x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
            xid, 0, 0, w, h, !0,
        );
        let Ok(img) = img else { return false };
        let Ok(img) = img.reply() else { return false };
        if img.depth < 24 {
            return false;
        }
        let (w, h) = (w as u32, h as u32);
        let data = &img.data; // BGRX rows, 4 bytes per pixel
        if (data.len() as u32) < w * h * 4 {
            return false;
        }
        let step = (w / max_w).max(1);
        let (tw, th) = (w / step, h / step);
        let mut out = Vec::with_capacity((tw * th * 3) as usize + 32);
        out.extend_from_slice(format!("P6\n{} {}\n255\n", tw, th).as_bytes());
        for y in 0..th {
            for x in 0..tw {
                let i = (((y * step) * w + (x * step)) * 4) as usize;
                out.push(data[i + 2]);
                out.push(data[i + 1]);
                out.push(data[i]);
            }
        }
        std::fs::write(path, out).is_ok()
    }
}
