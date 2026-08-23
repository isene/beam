//! beam — remote control for the presentation workspace.
//!
//! WS10 sits on the external monitor: whatever tab is active there is
//! what the room sees. beam shows the windows parked there as a grid of
//! thumbnails; Enter shows the chosen one, while the workspace being
//! typed on and the keyboard focus never change. Fe2O3 suite: crust for
//! the TUI, glow for the pictures.
//!
//! Battery posture: fully event-driven. beam blocks on stdin between
//! keys and talks to X only when told to. Thumbnails are grabbed on
//! start, on r (the visible one) and on S (all of them, by flipping
//! tabs); never on a timer.

mod xws;

use crust::{style, Crust, Input, Pane};
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");
// Grid cell in terminal cells: picture area plus one label line.
const CELL_W: u16 = 36;
const CELL_H: u16 = 12;
const THUMB_PX: u32 = 480;

fn ws_from_args() -> u32 {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--ws" {
            if let Some(n) = args.next().and_then(|v| v.parse::<u32>().ok()) {
                return n.saturating_sub(1); // 1-based on the command line
            }
        }
    }
    9 // WS10, 0-based
}

fn thumb_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("beam");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn thumb_path(xid: u32) -> PathBuf {
    thumb_dir().join(format!("{}.ppm", xid))
}

/// Grab what can be grabbed without disturbing anything: the windows
/// that are viewable right now.
fn grab_visible(x: &xws::X, wins: &[xws::Win]) {
    for w in wins.iter().filter(|w| w.active) {
        let _ = x.grab_ppm(w.xid, &thumb_path(w.xid), THUMB_PX);
    }
}

/// Grab every window by showing each in turn, then put the original
/// back. The external flickers through the set once; that is what the
/// key is for, and why it is a key rather than a timer.
fn scan_all(x: &xws::X, wins: &[xws::Win]) {
    let shown: Vec<u32> = wins.iter().filter(|w| w.active).map(|w| w.xid).collect();
    for w in wins {
        if !x.beam(w.xid) {
            continue;
        }
        std::thread::sleep(std::time::Duration::from_millis(350));
        let _ = x.grab_ppm(w.xid, &thumb_path(w.xid), THUMB_PX);
    }
    if let Some(orig) = shown.first() {
        let _ = x.beam(*orig);
    }
}

fn main() {
    if std::env::args().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("beam — remote control for the presentation workspace (Fe2O3)");
        println!();
        println!("Usage: beam [--ws N] [--list | --show XID]   (default: workspace 10)");
        println!();
        println!("  Enter    show the selected window on that workspace");
        println!("  r        re-grab the visible window's thumbnail");
        println!("  S        scan: flip through every window once, grabbing each");
        println!("  q        quit");
        return;
    }
    if std::env::args().skip(1).any(|a| a == "-v" || a == "--version") {
        println!("beam {}", VERSION);
        return;
    }
    let ws = ws_from_args();
    let Some(x) = xws::X::connect() else {
        eprintln!("beam: no X display");
        std::process::exit(1);
    };

    // Script modes: the same calls the TUI makes, without the TUI.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--list") {
        for w in x.windows_on(ws) {
            println!("{}\t{}\t{}\t{}", w.xid,
                     if w.active { "shown" } else { "-" }, w.class, w.title);
        }
        return;
    }
    if let Some(i) = argv.iter().position(|a| a == "--show") {
        let Some(xid) = argv.get(i + 1).and_then(|v| v.parse::<u32>().ok()) else {
            eprintln!("beam: --show needs a window id");
            std::process::exit(2);
        };
        std::process::exit(if x.beam(xid) { 0 } else { 1 });
    }

    Crust::init();
    Crust::set_app_identity("Beam");
    let mut disp = glow::Display::new();
    let (mut cols, mut rows) = Crust::terminal_size();
    let mut sel = 0usize;
    let mut wins = x.windows_on(ws);
    grab_visible(&x, &wins);
    let mut flash = String::new();
    loop {
        draw(&mut disp, cols, rows, ws, &wins, sel, &flash);
        let key = Input::getchr(None); // block: no tick, no wakeups
        flash.clear();
        let per_row = grid_cols(cols);
        match key.as_deref() {
            Some("q") | Some("Q") | Some("ESC") => break,
            Some("LEFT") => sel = sel.saturating_sub(1),
            Some("RIGHT") => {
                if sel + 1 < wins.len() {
                    sel += 1;
                }
            }
            Some("UP") => sel = sel.saturating_sub(per_row),
            Some("DOWN") => {
                if sel + per_row < wins.len() {
                    sel += per_row;
                }
            }
            Some("r") => {
                wins = x.windows_on(ws);
                grab_visible(&x, &wins);
                sel = sel.min(wins.len().saturating_sub(1));
                disp.clear_all();
            }
            Some("S") => {
                flash = "scanning…".into();
                draw(&mut disp, cols, rows, ws, &wins, sel, &flash);
                scan_all(&x, &wins);
                wins = x.windows_on(ws);
                flash = "scanned".into();
                disp.clear_all();
            }
            Some("ENTER") => {
                if let Some((xid, class)) =
                    wins.get(sel).map(|w| (w.xid, w.class.clone()))
                {
                    flash = if x.beam(xid) {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        wins = x.windows_on(ws);
                        let _ = x.grab_ppm(xid, &thumb_path(xid), THUMB_PX);
                        format!("beamed: {}", class)
                    } else {
                        "the WM did not take it".into()
                    };
                }
            }
            Some("RESIZE") => {
                let (c, r) = Crust::terminal_size();
                cols = c;
                rows = r;
                disp.clear_all();
            }
            _ => {}
        }
    }
    disp.clear_all();
    Crust::cleanup();
}

fn grid_cols(cols: u16) -> usize {
    ((cols.saturating_sub(2)) / CELL_W).max(1) as usize
}

fn draw(disp: &mut glow::Display, cols: u16, rows: u16, ws: u32,
        wins: &[xws::Win], sel: usize, flash: &str) {
    let mut pane = Pane::new(1, 1, cols, rows, 250, 0);
    let per_row = grid_cols(cols);
    let mut out = String::new();
    out.push_str(&style::styled(
        &format!(" beam — workspace {}  ·  {} window{} ",
                 ws + 1, wins.len(), if wins.len() == 1 { "" } else { "s" }),
        Some(231), Some(234), "b"));
    out.push('\n');
    pane.set_text(&out);
    pane.refresh();

    // The grid: pictures are drawn straight to the terminal, labels as
    // one styled line under each.
    for (i, w) in wins.iter().enumerate() {
        let gx = 2 + (i % per_row) as u16 * CELL_W;
        let gy = 3 + (i / per_row) as u16 * CELL_H;
        if gy + CELL_H > rows {
            break; // below the fold; selection still reaches it after resize
        }
        let tp = thumb_path(w.xid);
        if tp.exists() {
            disp.show(&tp.to_string_lossy(), gx, gy, CELL_W - 2, CELL_H - 2);
        }
        let mut label = Pane::new(gx, gy + CELL_H - 1, CELL_W - 1, 1, 250, 0);
        let marker = if w.active { "● " } else { "  " };
        let name = if w.title.is_empty() || w.title == w.class {
            format!("{} {}", w.class, w.xid)
        } else {
            w.title.clone()
        };
        let name: String = name.chars().take(CELL_W as usize - 6).collect();
        let text = format!("{}{}", marker, name);
        if i == sel {
            label.set_text(&style::styled(
                &format!("{:<w$}", text, w = CELL_W as usize - 1),
                Some(231), Some(238), "b"));
        } else {
            let coloured = if w.active {
                format!("{}{}", style::fg("● ", 46), name)
            } else {
                format!("  {}", style::fg(&name, 250))
            };
            label.set_text(&coloured);
        }
        label.refresh();
        if !tp.exists() {
            let mut hole = Pane::new(gx + 2, gy + CELL_H / 2 - 1, CELL_W - 6, 1, 240, 0);
            hole.set_text(&style::dim("no thumbnail — S to scan"));
            hole.refresh();
        }
    }

    let mut foot = Pane::new(1, rows, cols, 1, 244, 236);
    if flash.is_empty() {
        foot.set_text(" ←→↑↓ select · Enter show it · r regrab · S scan all · q quit");
    } else {
        foot.set_text(&style::fg(&format!(" {}", flash), 220));
    }
    foot.refresh();
}
