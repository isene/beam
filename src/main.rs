//! beam — remote control for the presentation workspace.
//!
//! WS10 sits on the external monitor: whatever tab is active there is
//! what the room sees. beam lists the windows parked on it and Enter
//! shows the chosen one, while the workspace being typed on never
//! changes. Fe2O3 suite; crust for the TUI.
//!
//! Battery posture: fully event-driven. beam blocks on stdin between
//! keys and asks X only when redrawing. No tick, no polling.

mod xws;

use crust::{style, Crust, Input, Pane};

const VERSION: &str = env!("CARGO_PKG_VERSION");

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

fn main() {
    if std::env::args().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("beam — remote control for the presentation workspace (Fe2O3)");
        println!();
        println!("Usage: beam [--ws N] [--list | --show XID]   (default: workspace 10)");
        println!();
        println!("  Enter  show the selected window on that workspace");
        println!("  r      refresh the list");
        println!("  q      quit");
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
    let (mut cols, mut rows) = Crust::terminal_size();
    let mut sel = 0usize;
    let mut wins = x.windows_on(ws);
    let mut flash = String::new();
    loop {
        draw(cols, rows, ws, &wins, sel, &flash);
        let key = Input::getchr(None); // block: no tick, no wakeups
        flash.clear();
        match key.as_deref() {
            Some("q") | Some("Q") | Some("ESC") => break,
            Some("UP") => sel = sel.saturating_sub(1),
            Some("DOWN") => {
                if sel + 1 < wins.len() {
                    sel += 1;
                }
            }
            Some("r") => {
                wins = x.windows_on(ws);
                sel = sel.min(wins.len().saturating_sub(1));
            }
            Some("ENTER") => {
                if let Some((xid, class)) =
                    wins.get(sel).map(|w| (w.xid, w.class.clone()))
                {
                    flash = if x.beam(xid) {
                        wins = x.windows_on(ws);
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
            }
            _ => {}
        }
    }
    Crust::cleanup();
}

fn draw(cols: u16, rows: u16, ws: u32, wins: &[xws::Win], sel: usize, flash: &str) {
    let mut pane = Pane::new(1, 1, cols, rows, 250, 0);
    let mut out = String::new();
    out.push_str(&style::styled(
        &format!(" beam — workspace {} ", ws + 1), Some(231), Some(234), "b"));
    out.push('\n');
    out.push('\n');
    if wins.is_empty() {
        out.push_str(&style::dim("  nothing parked on this workspace\n"));
    }
    for (i, w) in wins.iter().enumerate() {
        let marker = if w.active { style::fg("● ", 46) } else { "  ".into() };
        let title: String = w.title.chars().take(cols as usize - 26).collect();
        let line = format!("{}{:<14} {}", marker, w.class, title);
        if i == sel {
            out.push_str(&format!("  \x1b[48;5;238m{}\x1b[49m\n", line));
        } else {
            out.push_str(&format!("  {}\n", line));
        }
    }
    out.push('\n');
    if flash.is_empty() {
        out.push_str(&style::dim(" ↑↓ select · Enter show it · r refresh · q quit"));
    } else {
        out.push_str(&style::fg(&format!(" {}", flash), 220));
    }
    pane.set_text(out.trim_end_matches('\n'));
    pane.refresh();
}
