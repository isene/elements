//! elements — periodic table explorer for the Fe2O3 suite.
//!
//! A grid of all elements (118 confirmed + the hypothesized period-8 ones)
//! with a scrollable detail pane: full structured properties plus the
//! complete Wikipedia article, all served from a local cache
//! (~/.elements/elements.json). On wide terminals the property table sits
//! beside the grid. Eight color modes (category, phase, cosmic origin,
//! occurrence, block, and three value gradients) recolor the whole table.
//! The network is touched exactly once — on first start, `--fetch`, or the
//! `u` key — never in the UI loop, which blocks on input with zero idle
//! wakes.

mod data;
mod fetch;

use crust::{Crust, Input, Pane};
use data::Element;
use std::io::Write;

const GRID_X0: u16 = 3; // leftmost cell column
const CELL_W: u16 = 4; // 3-char symbol + gap
const DETAIL_Y: u16 = 17; // first row of the detail pane
const MIN_COLS: u16 = GRID_X0 + 18 * CELL_W; // full 18-group table
const SIDE_X: u16 = 78; // property block beside the grid starts here
const SIDE_MIN: u16 = SIDE_X + 73; // terminal width needed for the side block

const RUST: &str = "\x1b[1;38;2;247;76;0m";
const RESET: &str = "\x1b[0m";

const MODE_NAMES: [&str; 8] = [
    "category", "phase", "cosmic origin", "occurrence", "block",
    "electronegativity", "melting point", "density",
];

struct App {
    els: Vec<Element>,
    sel: usize,
    max_y: u32,
    mode: usize,
    show_help: bool,
}

fn main() {
    let mut force_fetch = false;
    let mut start: Option<String> = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--fetch" => force_fetch = true,
            "-h" | "--help" => {
                println!("elements — periodic table explorer (Fe2O3 suite)");
                println!();
                println!("Usage: elements [ELEMENT] [--fetch]");
                println!();
                println!("  ELEMENT     start at an element (name, symbol, or atomic number)");
                println!("  --fetch     rebuild the local dataset from Wikipedia");
                println!("  -v          print version");
                println!();
                println!("Data is fetched once from Wikipedia and cached at ~/.elements/elements.json.");
                return;
            }
            "-v" | "--version" => {
                println!("elements {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => start = Some(other.to_string()),
        }
    }

    let els = if force_fetch { None } else { data::load() };
    let els = match els {
        Some(e) => e,
        None => {
            println!("elements: building the local dataset (one-time fetch from Wikipedia) …");
            match fetch::fetch_all() {
                Ok(e) => {
                    if let Err(err) = data::save(&e) {
                        eprintln!("elements: could not save cache: {err}");
                    }
                    e
                }
                Err(err) => {
                    eprintln!("elements: fetch failed: {err}");
                    std::process::exit(1);
                }
            }
        }
    };

    // Default to the suite's namesake.
    let mut sel = els.iter().position(|e| e.symbol == "Fe").unwrap_or(0);
    if let Some(q) = start {
        match find(&els, &q) {
            Some(i) => sel = i,
            None => {
                eprintln!("elements: no element matches '{q}'");
                std::process::exit(1);
            }
        }
    }

    // Not a terminal (piped / scripted): print the element as plain text
    // instead of entering the TUI — `elements iron | less` just works.
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        let text = detail_text(&els[sel], false);
        if std::io::stdout().is_terminal() {
            println!("{text}");
        } else {
            println!("{}", crust::strip_ansi(&text));
        }
        return;
    }

    let max_y = els.iter().map(|e| e.ypos).max().unwrap_or(10);
    let mut app = App { els, sel, max_y, mode: 0, show_help: false };

    Crust::init();
    Crust::set_app_identity("Elements");
    let (mut cols, mut rows) = Crust::terminal_size();
    let mut detail = Pane::new(1, DETAIL_Y, cols, rows.saturating_sub(DETAIL_Y).max(1), 253, 0);
    let mut status = Pane::new(1, rows, cols, 1, 250, 236);
    status.scroll = false;

    draw_all(&app, &mut detail, &mut status, cols, rows);

    loop {
        let key = match Input::getchr(None) {
            Some(k) => k,
            None => continue,
        };
        match key.as_str() {
            "q" | "ESC" => break,
            "LEFT" | "h" | "<" | "-" => {
                let t = app.sel.saturating_sub(1);
                select(&mut app, t, &mut detail, cols);
            }
            "RIGHT" | "l" | ">" | "+" => {
                let t = (app.sel + 1).min(app.els.len() - 1);
                select(&mut app, t, &mut detail, cols);
            }
            "UP" | "k" => {
                let t = moved(&app, -1);
                select(&mut app, t, &mut detail, cols);
            }
            "DOWN" | "j" => {
                let t = moved(&app, 1);
                select(&mut app, t, &mut detail, cols);
            }
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" => {
                app.mode = key.parse::<usize>().unwrap() - 1;
                draw_header(&app, cols);
                draw_grid(&app, cols);
            }
            "C-RIGHT" => {
                app.mode = (app.mode + 1) % MODE_NAMES.len();
                draw_header(&app, cols);
                draw_grid(&app, cols);
            }
            "C-LEFT" => {
                app.mode = (app.mode + MODE_NAMES.len() - 1) % MODE_NAMES.len();
                draw_header(&app, cols);
                draw_grid(&app, cols);
            }
            "J" | "S-DOWN" => detail.linedown(),
            "K" | "S-UP" => detail.lineup(),
            " " | "PgDOWN" => detail.pagedown(),
            "PgUP" => detail.pageup(),
            "g" | "HOME" => detail.top(),
            "G" | "END" => detail.bottom(),
            "/" => {
                let q = status.ask_or_cancel("Find (name, symbol, or number): ", "");
                print!("\x1b[?25l");
                std::io::stdout().flush().ok();
                match q.as_deref().map(|q| find(&app.els, q)) {
                    Some(Some(i)) => {
                        select(&mut app, i, &mut detail, cols);
                        status.say(&help_line());
                    }
                    Some(None) => status.say("\x1b[38;2;255;120;120mno match\x1b[0m"),
                    None => status.say(&help_line()),
                }
            }
            "w" => {
                let url = &app.els[app.sel].source;
                if !url.is_empty() {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(url)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                }
            }
            "u" => {
                Crust::cleanup();
                println!("elements: re-fetching all element data from Wikipedia …");
                let result = fetch::fetch_all();
                let msg = match result {
                    Ok(e) => {
                        if let Err(err) = data::save(&e) {
                            format!("\x1b[38;2;255;120;120mcould not save cache: {err}\x1b[0m")
                        } else {
                            let cur = app.els[app.sel].number;
                            app.els = e;
                            app.sel = app.els.iter().position(|x| x.number == cur).unwrap_or(0);
                            app.max_y = app.els.iter().map(|e| e.ypos).max().unwrap_or(10);
                            "data updated from Wikipedia".to_string()
                        }
                    }
                    Err(err) => format!("\x1b[38;2;255;120;120mfetch failed: {err}\x1b[0m"),
                };
                Crust::init();
                Crust::set_app_identity("Elements");
                draw_all(&app, &mut detail, &mut status, cols, rows);
                status.say(&msg);
            }
            "?" => {
                app.show_help = !app.show_help;
                set_detail(&app, &mut detail, cols);
            }
            "RESIZE" => {
                let (c, r) = Crust::terminal_size();
                cols = c;
                rows = r;
                detail.w = cols;
                detail.h = rows.saturating_sub(DETAIL_Y).max(1);
                status.y = rows;
                status.w = cols;
                draw_all(&app, &mut detail, &mut status, cols, rows);
            }
            _ => {}
        }
    }

    Crust::cleanup();
}

// ─────────────────────────── selection ───────────────────────────────

fn find(els: &[Element], q: &str) -> Option<usize> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    if let Ok(n) = q.parse::<u32>() {
        return els.iter().position(|e| e.number == n);
    }
    let ql = q.to_lowercase();
    els.iter()
        .position(|e| e.symbol.to_lowercase() == ql)
        .or_else(|| els.iter().position(|e| e.name.to_lowercase().starts_with(&ql)))
        .or_else(|| els.iter().position(|e| e.name.to_lowercase().contains(&ql)))
}

/// Walk vertically from the selected cell until another element is hit in
/// the same column (the table has gaps), or stay put at the edge.
fn moved(app: &App, dy: i32) -> usize {
    let x = app.els[app.sel].xpos as i32;
    let mut y = app.els[app.sel].ypos as i32;
    loop {
        y += dy;
        if !(1..=app.max_y as i32).contains(&y) {
            return app.sel;
        }
        if let Some(i) = app
            .els
            .iter()
            .position(|e| e.xpos as i32 == x && e.ypos as i32 == y)
        {
            return i;
        }
    }
}

fn select(app: &mut App, new: usize, detail: &mut Pane, cols: u16) {
    if new == app.sel && !app.show_help {
        return;
    }
    app.sel = new;
    app.show_help = false;
    draw_header(app, cols);
    draw_grid(app, cols);
    draw_side(app, cols);
    set_detail(app, detail, cols);
}

// ─────────────────────────── color modes ─────────────────────────────

fn cat_rgb(cat: &str) -> (u8, u8, u8) {
    let c = cat.to_ascii_lowercase();
    if c.contains("alkali metal") {
        (255, 99, 71)
    } else if c.contains("alkaline earth") {
        (255, 165, 0)
    } else if c.contains("post-transition") {
        (120, 220, 120)
    } else if c.contains("transition metal") {
        (255, 208, 0)
    } else if c.contains("metalloid") {
        (0, 206, 209)
    } else if c.contains("nonmetal") {
        (100, 150, 255)
    } else if c.contains("noble gas") {
        (190, 120, 255)
    } else if c.contains("lanthanide") {
        (255, 121, 198)
    } else if c.contains("actinide") {
        (240, 90, 150)
    } else {
        (150, 150, 150) // unknown / hypothetical
    }
}

const CAT_LEGEND: [(&str, (u8, u8, u8)); 10] = [
    ("alkali", (255, 99, 71)),
    ("alkEarth", (255, 165, 0)),
    ("transit", (255, 208, 0)),
    ("postTr", (120, 220, 120)),
    ("metallo", (0, 206, 209)),
    ("nonmet", (100, 150, 255)),
    ("noble", (190, 120, 255)),
    ("lanth", (255, 121, 198)),
    ("actin", (240, 90, 150)),
    ("unk", (150, 150, 150)),
];

const PHASE_LEGEND: [(&str, (u8, u8, u8)); 4] = [
    ("solid", (230, 210, 160)),
    ("liquid", (85, 170, 255)),
    ("gas", (255, 119, 136)),
    ("unknown", (150, 150, 150)),
];

/// Dominant nucleosynthetic source per element, simplified from the
/// familiar "origin of the elements" chart (mixes reduced to the largest
/// contributor to Solar System abundance).
const ORIGIN_LEGEND: [(&str, (u8, u8, u8)); 7] = [
    ("BigBang", (102, 170, 255)),
    ("CosmicRays", (255, 119, 221)),
    ("LowMassStars", (102, 221, 102)),
    ("MassiveSN", (255, 204, 68)),
    ("WhiteDwarfSN", (255, 136, 85)),
    ("NSmerger", (204, 136, 255)),
    ("Synthetic", (150, 150, 150)),
];

fn origin_idx(z: u32) -> usize {
    match z {
        1 | 2 => 0,                                             // Big Bang fusion
        4 | 5 => 1,                                             // cosmic-ray spallation
        3 | 6 | 7 | 9 | 38..=42 | 48 | 50 | 56..=60 | 70 | 80..=82 => 2, // dying low-mass stars (s-process)
        8 | 10..=21 | 27 | 29..=37 => 3,                        // exploding massive stars
        22..=26 | 28 => 4,                                      // exploding white dwarfs
        43 | 61 | 93.. => 6,                                    // human-made
        _ => 5,                                                 // merging neutron stars (r-process)
    }
}

const OCC_LEGEND: [(&str, (u8, u8, u8)); 4] = [
    ("primordial", (102, 221, 136)),
    ("fromDecay", (255, 187, 68)),
    ("synthetic", (255, 102, 102)),
    ("hypothetical", (150, 150, 150)),
];

fn occurrence_rgb(z: u32) -> (u8, u8, u8) {
    match z {
        119.. => OCC_LEGEND[3].1,
        43 | 61 | 95..=118 => OCC_LEGEND[2].1,
        84..=89 | 91 | 93 | 94 => OCC_LEGEND[1].1,
        _ => OCC_LEGEND[0].1,
    }
}

const BLOCK_LEGEND: [(&str, (u8, u8, u8)); 5] = [
    ("s", (255, 119, 102)),
    ("p", (102, 170, 255)),
    ("d", (255, 204, 68)),
    ("f", (221, 119, 255)),
    ("g", (150, 150, 150)),
];

fn mode_value(e: &Element, mode: usize) -> Option<f64> {
    match mode {
        5 => e.electronegativity_pauling,
        6 => e.melt,
        7 => e.density.map(|d| d.max(1e-6).log10()),
        _ => None,
    }
}

/// Blue → yellow → red, t in 0..1.
fn gradient(t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: (u8, u8, u8), b: (u8, u8, u8), t: f64| -> (u8, u8, u8) {
        (
            (a.0 as f64 + (b.0 as f64 - a.0 as f64) * t) as u8,
            (a.1 as f64 + (b.1 as f64 - a.1 as f64) * t) as u8,
            (a.2 as f64 + (b.2 as f64 - a.2 as f64) * t) as u8,
        )
    };
    if t < 0.5 {
        lerp((70, 130, 255), (250, 220, 90), t * 2.0)
    } else {
        lerp((250, 220, 90), (255, 80, 60), t * 2.0 - 1.0)
    }
}

fn cell_rgb(e: &Element, mode: usize, mm: Option<(f64, f64)>) -> (u8, u8, u8) {
    match mode {
        1 => match e.phase.as_str() {
            "Solid" => PHASE_LEGEND[0].1,
            "Liquid" => PHASE_LEGEND[1].1,
            "Gas" => PHASE_LEGEND[2].1,
            _ => PHASE_LEGEND[3].1,
        },
        2 => ORIGIN_LEGEND[origin_idx(e.number)].1,
        3 => occurrence_rgb(e.number),
        4 => BLOCK_LEGEND
            .iter()
            .find(|(b, _)| *b == e.block)
            .map(|(_, c)| *c)
            .unwrap_or((150, 150, 150)),
        5..=7 => match (mode_value(e, mode), mm) {
            (Some(v), Some((lo, hi))) if hi > lo => gradient((v - lo) / (hi - lo)),
            _ => (110, 110, 110),
        },
        _ => cat_rgb(&e.category),
    }
}

// ─────────────────────────── rendering ───────────────────────────────

fn move_to(row: u16, col: u16) -> String {
    format!("\x1b[{};{}H", row, col)
}

fn grid_row(ypos: u32) -> u16 {
    // Blank row + label row below the header, then periods 1-8; the f-block
    // and the hypothesized g-block row sit below a one-row gap.
    if ypos <= 8 { 3 + ypos as u16 } else { 4 + ypos as u16 }
}

/// Colored legend for the active color mode (lives in the header row).
fn legend_string(app: &App) -> String {
    let mut s = format!("\x1b[1m{} {}\x1b[0m ", app.mode + 1, MODE_NAMES[app.mode]);
    let items: &[(&str, (u8, u8, u8))] = match app.mode {
        0 => &CAT_LEGEND,
        1 => &PHASE_LEGEND,
        2 => &ORIGIN_LEGEND,
        3 => &OCC_LEGEND,
        4 => &BLOCK_LEGEND,
        _ => &[],
    };
    if items.is_empty() {
        // Gradient modes: a color ramp.
        s.push_str("\x1b[2mlow \x1b[0m");
        for i in 0..16 {
            let (r, g, b) = gradient(i as f64 / 15.0);
            s.push_str(&format!("\x1b[38;2;{r};{g};{b}m█\x1b[0m"));
        }
        s.push_str("\x1b[2m high\x1b[0m");
    } else {
        for (lbl, (r, g, b)) in items {
            s.push_str(&format!("\x1b[38;2;{r};{g};{b}m{lbl}\x1b[0m "));
        }
    }
    s
}

fn draw_header(app: &App, cols: u16) {
    let e = &app.els[app.sel];
    let (r, g, b) = cat_rgb(&e.category);
    let bg = "\x1b[48;5;236m";
    let info = format!(
        " {RUST}elements{RESET}  \x1b[1m{}{RESET} ({})  \x1b[2mZ={}\x1b[0m  \x1b[38;2;{r};{g};{b}m{}{RESET}",
        e.name, e.symbol, e.number, e.category
    );
    let iw = crust::display_width(&info);
    // Align the legend with the property table beside the grid when the
    // wide layout is active (and the element info leaves room for it).
    let content = if cols >= SIDE_MIN && iw < SIDE_X as usize - 1 {
        format!("{info}{}{}", " ".repeat(SIDE_X as usize - 1 - iw), legend_string(app))
    } else {
        format!("{info}   {}", legend_string(app))
    };
    // Re-arm the bar background after every SGR reset in the content.
    let line = content.replace(RESET, &format!("{RESET}{bg}"));
    let pad = (cols as usize).saturating_sub(crust::display_width(&content));
    print!("{}{bg}{line}{}{RESET}", move_to(1, 1), " ".repeat(pad));
    std::io::stdout().flush().ok();
}

/// Group numbers above the columns, period numbers left of the rows.
fn draw_grid_labels(cols: u16) {
    if cols < MIN_COLS {
        return;
    }
    let mut s = String::new();
    for g in 1..=18u16 {
        s.push_str(&move_to(3, GRID_X0 + (g - 1) * CELL_W));
        s.push_str(&format!("\x1b[2m{:^3}\x1b[0m", g));
    }
    for p in 1..=8u16 {
        s.push_str(&move_to(grid_row(p as u32), 1));
        s.push_str(&format!("\x1b[2m{p}\x1b[0m"));
    }
    print!("{s}");
    std::io::stdout().flush().ok();
}

fn draw_grid(app: &App, cols: u16) {
    let mut s = String::new();
    if cols < MIN_COLS {
        s.push_str(&move_to(4, 2));
        s.push_str("\x1b[2mterminal too narrow for the table (need 75 cols) — / still works\x1b[0m");
    } else {
        let mm = if (5..=7).contains(&app.mode) {
            let vals: Vec<f64> = app
                .els
                .iter()
                .filter_map(|e| mode_value(e, app.mode))
                .collect();
            let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            Some((lo, hi))
        } else {
            None
        };
        for (i, e) in app.els.iter().enumerate() {
            let col = GRID_X0 + (e.xpos as u16 - 1) * CELL_W;
            let (r, g, b) = cell_rgb(e, app.mode, mm);
            s.push_str(&move_to(grid_row(e.ypos), col));
            if i == app.sel {
                s.push_str(&format!("\x1b[7;1;38;2;{r};{g};{b}m{:<3}\x1b[0m", e.symbol));
            } else {
                s.push_str(&format!("\x1b[38;2;{r};{g};{b}m{:<3}\x1b[0m", e.symbol));
            }
        }
    }
    print!("{}", s);
    std::io::stdout().flush().ok();
}

fn help_line() -> String {
    "\x1b[2m←→ Z± · ↑↓ col · 1-8/^←→ color · J/K scroll · / find · ? help · q quit\x1b[0m"
        .to_string()
}

fn draw_all(app: &App, detail: &mut Pane, status: &mut Pane, cols: u16, _rows: u16) {
    Crust::clear_screen();
    draw_header(app, cols);
    draw_grid(app, cols);
    draw_grid_labels(cols);
    draw_side(app, cols);
    status.invalidate();
    status.say(&help_line());
    detail.invalidate();
    set_detail(app, detail, cols);
}

fn set_detail(app: &App, detail: &mut Pane, cols: u16) {
    let side = cols >= SIDE_MIN;
    let text = if app.show_help {
        help_text()
    } else {
        detail_text(&app.els[app.sel], side)
    };
    detail.set_text(&text);
    detail.ix = 0;
    detail.refresh();
}

/// Property block beside the grid (wide terminals): the table plus the
/// full-width rows, drawn at SIDE_X over rows 2..=14.
fn draw_side(app: &App, cols: u16) {
    if cols < SIDE_MIN {
        return;
    }
    let avail = (cols - SIDE_X + 1) as usize;
    let e = &app.els[app.sel];
    let mut lines: Vec<String> = Vec::new();
    let (phys, atom) = prop_rows(e);
    if !phys.is_empty() || !atom.is_empty() {
        for l in prop_table("Physical", &phys, "Atomic", &atom).lines() {
            lines.push(l.to_string());
        }
    }
    let dim = "\x1b[2m";
    if let Some(a) = &e.appearance {
        lines.push(format!("{dim}{:<14}{RESET}{}", "appearance", fit(a, avail - 14)));
    }
    if !e.ionization_energies.is_empty() {
        let list: Vec<String> = e.ionization_energies.iter().map(|v| v.to_string()).collect();
        let val = format!("{} kJ/mol", list.join(", "));
        lines.push(format!("{dim}{:<14}{RESET}{}", "ionization", fit(&val, avail - 14)));
    }
    if let Some(d) = &e.discovered_by {
        lines.push(format!("{dim}{:<14}{RESET}{}", "discovered by", fit(d, avail - 14)));
    }
    if let Some(n) = &e.named_by {
        lines.push(format!("{dim}{:<14}{RESET}{}", "named by", fit(n, avail - 14)));
    }

    let blank = " ".repeat(avail);
    let mut s = String::new();
    for row in 0..13u16 {
        s.push_str(&move_to(3 + row, SIDE_X));
        s.push_str(&blank);
    }
    for (i, l) in lines.iter().take(13).enumerate() {
        s.push_str(&move_to(3 + i as u16, SIDE_X));
        s.push_str(l);
    }
    print!("{s}");
    std::io::stdout().flush().ok();
}

fn help_text() -> String {
    format!(
        "{RUST}elements — keys{RESET}\n\n\
         \x20 ← → / h l           previous / next element (walks the whole table)\n\
         \x20 ↑ ↓ / k j           up / down within the column\n\
         \x20 < >                 same as ← →\n\
         \x20 1-8, Ctrl+← →       color mode: 1 category · 2 phase · 3 cosmic origin ·\n\
         \x20                     4 occurrence · 5 block · 6 electronegativity ·\n\
         \x20                     7 melting point · 8 density (log scale)\n\
         \x20 J K / Shift-↓ ↑     scroll the article one line\n\
         \x20 Space, PgDn/PgUp    scroll the article one page\n\
         \x20 g G                 top / bottom of the article\n\
         \x20 /                   find an element (name, symbol, or atomic number)\n\
         \x20 w                   open the element's Wikipedia page in the browser\n\
         \x20 u                   re-fetch all data from Wikipedia\n\
         \x20 ?                   toggle this help\n\
         \x20 q, ESC              quit\n\n\
         The cosmic-origin mode shows the DOMINANT nucleosynthetic source per\n\
         element (simplified — most elements are a mix of sources).\n\n\
         Structured properties come from the Wikipedia-derived Periodic-Table-JSON\n\
         dataset; each element also carries its full Wikipedia article. Everything\n\
         is cached at ~/.elements/elements.json — the UI never touches the network.\n\
         The hypothesized elements 119–126 are included (g-block row at the bottom)."
    )
}

fn kelvin(v: f64) -> String {
    format!("{} K ({:.2} °C)", v, v - 273.15)
}

/// The two property columns (Physical | Atomic) for an element.
fn prop_rows(e: &Element) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let mut phys: Vec<(String, String)> = Vec::new();
    if !e.phase.is_empty() {
        phys.push(("phase".into(), e.phase.clone()));
    }
    if let Some(v) = e.atomic_mass {
        phys.push(("mass".into(), format!("{v} u")));
    }
    if let Some(v) = e.density {
        let unit = if e.phase == "Gas" { "g/L" } else { "g/cm³" };
        phys.push(("density".into(), format!("{v} {unit}")));
    }
    if let Some(v) = e.melt {
        phys.push(("melt".into(), kelvin(v)));
    }
    if let Some(v) = e.boil {
        phys.push(("boil".into(), kelvin(v)));
    }
    if let Some(v) = e.molar_heat {
        phys.push(("molar heat".into(), format!("{v} J/(mol·K)")));
    }

    let mut atom: Vec<(String, String)> = Vec::new();
    if let Some(v) = e.group {
        atom.push(("group".into(), v.to_string()));
    }
    if let Some(v) = e.period {
        atom.push(("period".into(), v.to_string()));
    }
    if !e.block.is_empty() {
        atom.push(("block".into(), e.block.clone()));
    }
    if !e.shells.is_empty() {
        let shells: Vec<String> = e.shells.iter().map(|n| n.to_string()).collect();
        atom.push(("shells".into(), shells.join(",")));
    }
    if !e.electron_configuration_semantic.is_empty() {
        atom.push(("config".into(), e.electron_configuration_semantic.clone()));
    }
    if let Some(v) = e.electronegativity_pauling {
        atom.push(("electroneg.".into(), v.to_string()));
    }
    if let Some(v) = e.electron_affinity {
        atom.push(("e⁻ affinity".into(), format!("{v} kJ/mol")));
    }
    (phys, atom)
}

fn detail_text(e: &Element, side: bool) -> String {
    let (r, g, b) = cat_rgb(&e.category);
    let dim = "\x1b[2m";
    let head = "\x1b[1;38;2;247;140;60m";
    let mut s = String::new();

    // On wide terminals the property block sits beside the grid, so the
    // pane carries only the prose. Stacked layouts get everything here.
    if !side {
        s.push_str(&format!(
            "\x1b[1;38;2;{r};{g};{b}m{} ({}){RESET}  Z={}  \x1b[38;2;{r};{g};{b}m{}{RESET}\n\n",
            e.name, e.symbol, e.number, e.category
        ));
        let (phys, atom) = prop_rows(e);
        if !phys.is_empty() || !atom.is_empty() {
            s.push_str(&prop_table("Physical", &phys, "Atomic", &atom));
        }
        let mut wide = |label: &str, value: &str| {
            s.push_str(&format!("{dim}{:<14}{RESET}{}\n", label, value));
        };
        if let Some(a) = &e.appearance {
            wide("appearance", a);
        }
        if !e.ionization_energies.is_empty() {
            let list: Vec<String> = e.ionization_energies.iter().map(|v| v.to_string()).collect();
            wide("ionization", &format!("{} kJ/mol", list.join(", ")));
        }
        if let Some(d) = &e.discovered_by {
            wide("discovered by", d);
        }
        if let Some(n) = &e.named_by {
            wide("named by", n);
        }
        if !e.source.is_empty() {
            wide("wikipedia", &e.source);
        }
        s.push('\n');
    }

    if !e.summary.is_empty() {
        s.push_str(&format!("{head}Summary{RESET}\n{}\n\n", e.summary));
    }
    if !e.article.is_empty() {
        s.push_str(&format!("{head}Wikipedia article{RESET}\n"));
        s.push_str(&style_article(&e.article));
    }
    s
}

/// Two side-by-side label/value columns in a box-drawn table (73 cells wide).
fn prop_table(lt: &str, left: &[(String, String)], rt: &str, right: &[(String, String)]) -> String {
    const CELL: usize = 35; // inner width of each cell
    const LBL: usize = 12;
    const VAL: usize = CELL - LBL - 2;
    let d = "\x1b[2m";
    let dash = |n: usize| "─".repeat(n);
    let mut s = String::new();
    s.push_str(&format!(
        "{d}┌─{RESET} \x1b[1m{lt}{RESET} {d}{}┬─{RESET} \x1b[1m{rt}{RESET} {d}{}┐{RESET}\n",
        dash(CELL - 3 - lt.chars().count()),
        dash(CELL - 3 - rt.chars().count())
    ));
    for i in 0..left.len().max(right.len()) {
        let cell = |c: Option<&(String, String)>| -> String {
            match c {
                Some((l, v)) => format!(" {d}{l:<LBL$}{RESET}{} ", fit(v, VAL)),
                None => " ".repeat(CELL),
            }
        };
        s.push_str(&format!(
            "{d}│{RESET}{}{d}│{RESET}{}{d}│{RESET}\n",
            cell(left.get(i)),
            cell(right.get(i))
        ));
    }
    s.push_str(&format!("{d}└{}┴{}┘{RESET}\n", dash(CELL), dash(CELL)));
    s
}

/// Pad or ellipsize `v` to exactly `w` chars.
fn fit(v: &str, w: usize) -> String {
    let n = v.chars().count();
    if n <= w {
        format!("{v}{}", " ".repeat(w - n))
    } else {
        let mut t: String = v.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Reference/link sections at the article tail — not worth screen space.
const TAIL_SECTIONS: [&str; 9] = [
    "see also", "references", "notes", "citations", "sources",
    "further reading", "external links", "bibliography", "explanatory notes",
];

/// Clean up a Wikipedia plain-text extract for display:
/// - bold the "== Section ==" headings,
/// - stop at the reference/link tail sections,
/// - collapse <math> dumps (a stack of indented glyph lines followed by a
///   "{\displaystyle …}" annotation) into the readable LaTeX body.
fn style_article(a: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in a.lines() {
        let t = line.trim();
        if t.len() > 4 && t.starts_with("==") && t.ends_with("==") {
            let title = t.trim_matches(|c: char| c == '=' || c == ' ');
            if TAIL_SECTIONS.contains(&title.to_lowercase().as_str()) {
                break;
            }
            out.push(format!("\x1b[1;38;2;247;140;60m{title}{RESET}"));
        } else if let Some(p) = line
            .find("{\\displaystyle")
            .or_else(|| line.find("{\\textstyle"))
        {
            // Drop the glyph stack that precedes the annotation.
            while matches!(out.last(), Some(l) if l.is_empty() || l.starts_with(' ')) {
                out.pop();
            }
            let rest = &line[p..];
            let inner = rest
                .find(' ')
                .map(|i| rest[i + 1..].trim_end())
                .unwrap_or("");
            let inner = inner.strip_suffix('}').unwrap_or(inner).trim();
            if !inner.is_empty() {
                out.push(format!("    \x1b[38;2;150;200;255m{inner}\x1b[0m"));
            }
        } else {
            out.push(line.to_string());
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}
