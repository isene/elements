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
const SIDE_ROWS: u16 = DETAIL_Y - 3; // rows 3..DETAIL_Y-1 are the side block's

const RUST: &str = "\x1b[1;38;2;247;76;0m";
const RESET: &str = "\x1b[0m";

const MODE_NAMES: [&str; 13] = [
    "category", "phase", "cosmic origin", "occurrence", "block",
    "electronegativity", "melting point", "density",
    "phase at T", "1st ionization", "life", "stability", "known by",
];
/// Modes reachable by digit; the rest live in the `m` menu and the cycle.
const DIGIT_MODES: usize = 9;
const MODE_TEMP: usize = 8;
const MODE_YEAR: usize = 12;

#[derive(PartialEq, Clone, Copy)]
enum View {
    Article,
    Help,
    Chat,
    Modes,
}

struct App {
    els: Vec<Element>,
    sel: usize,
    max_y: u32,
    mode: usize,
    /// Temperature for the "phase at T" mode, in kelvin.
    temp_k: f64,
    /// Year for the "known by" mode; negative is BC.
    year: i32,
    view: View,
    /// Cursor in the mode menu.
    menu_ix: usize,
    /// Q&A turns with Claude about the selected element. Cleared when the
    /// selection moves, so each element gets its own conversation.
    chat: Vec<(String, String)>,
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

    // Caches written before discovery years existed: one cheap Wikidata
    // query fills them in, no article refetch. Runs once, then persists.
    let mut els = els;
    let mut dirty = false;
    if els.iter().all(|e| e.discovered_year.is_empty()) {
        if let Ok(years) = fetch::fetch_years() {
            for e in els.iter_mut() {
                if let Some(y) = years.get(&e.number) {
                    e.discovered_year = y.clone();
                }
            }
            dirty = true;
        }
    }
    // Elements Wikidata has no date for, patched locally (no network).
    let mut gaps = std::collections::HashMap::new();
    fetch::fill_gaps(&mut gaps);
    for e in els.iter_mut() {
        if e.discovered_year.is_empty() {
            if let Some(y) = gaps.get(&e.number) {
                e.discovered_year = y.clone();
                dirty = true;
            }
        }
    }
    if dirty {
        let _ = data::save(&els);
    }

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
    // Start the year slider with the whole table known: the latest find.
    let latest = els.iter().filter_map(element_year).max().unwrap_or(2010);
    let mut app = App {
        els,
        sel,
        max_y,
        mode: 0,
        temp_k: 293.15, // room temperature
        year: latest,
        view: View::Article,
        menu_ix: 0,
        chat: Vec::new(),
    };

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
            "q" => break,
            // ESC leaves help / chat first, quits only from the article.
            "ESC" => {
                if app.view == View::Article {
                    break;
                }
                app.view = View::Article;
                set_detail(&app, &mut detail, cols);
            }
            // In the mode menu the movement keys pick a mode instead.
            "UP" | "k" | "DOWN" | "j" if app.view == View::Modes => {
                let up = key == "UP" || key == "k";
                let n = MODE_NAMES.len();
                app.menu_ix = if up {
                    (app.menu_ix + n - 1) % n
                } else {
                    (app.menu_ix + 1) % n
                };
                set_detail(&app, &mut detail, cols);
            }
            "ENTER" if app.view == View::Modes => {
                app.mode = app.menu_ix;
                app.view = View::Article;
                draw_header(&app, cols);
                draw_grid(&app, cols);
                set_detail(&app, &mut detail, cols);
            }
            "LEFT" | "h" | "<" => {
                let t = app.sel.saturating_sub(1);
                select(&mut app, t, &mut detail, cols);
            }
            "RIGHT" | "l" | ">" => {
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
            // Temperature for the "phase at T" mode. Coarse with the
            // shifted pair, and only meaningful while that mode is up.
            "+" | "-" | "*" | "_" => {
                let up = key == "+" || key == "*";
                let coarse = key == "*" || key == "_";
                if app.mode == MODE_TEMP {
                    let step = if coarse { 250.0 } else { 25.0 };
                    app.temp_k = (app.temp_k + if up { step } else { -step }).clamp(0.0, 7000.0);
                    draw_header(&app, cols);
                    draw_grid(&app, cols);
                } else if app.mode == MODE_YEAR {
                    let step = if coarse { 250 } else { 5 };
                    let hi = app.els.iter().filter_map(element_year).max().unwrap_or(2010);
                    app.year = (app.year + if up { step } else { -step }).clamp(-10000, hi);
                    draw_header(&app, cols);
                    draw_grid(&app, cols);
                } else {
                    let t = if key == "+" {
                        (app.sel + 1).min(app.els.len() - 1)
                    } else {
                        app.sel.saturating_sub(1)
                    };
                    select(&mut app, t, &mut detail, cols);
                }
            }
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                let n = key.parse::<usize>().unwrap() - 1;
                if n < DIGIT_MODES {
                    app.mode = n;
                    app.view = View::Article;
                    draw_header(&app, cols);
                    draw_grid(&app, cols);
                    set_detail(&app, &mut detail, cols);
                }
            }
            "m" => {
                app.view = if app.view == View::Modes { View::Article } else { View::Modes };
                app.menu_ix = app.mode;
                set_detail(&app, &mut detail, cols);
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
                app.view = if app.view == View::Help { View::Article } else { View::Help };
                set_detail(&app, &mut detail, cols);
            }
            "c" => {
                let prompt = if app.chat.is_empty() {
                    format!("Ask Claude about {}: ", app.els[app.sel].name)
                } else {
                    "Follow-up: ".to_string()
                };
                let q = status.ask_or_cancel(&prompt, "");
                print!("\x1b[?25l");
                std::io::stdout().flush().ok();
                match q {
                    Some(q) if !q.trim().is_empty() => {
                        status.say("\x1b[38;2;120;200;255m asking claude…\x1b[0m");
                        let answer = ask_claude(&app, q.trim());
                        match answer {
                            Ok(a) if !a.is_empty() => {
                                app.chat.push((q.trim().to_string(), a));
                                app.view = View::Chat;
                                set_detail(&app, &mut detail, cols);
                                status.say(&help_line());
                            }
                            Ok(_) => status.say("\x1b[38;2;255;120;120mclaude returned nothing\x1b[0m"),
                            Err(e) => status.say(&format!("\x1b[38;2;255;120;120mclaude: {e}\x1b[0m")),
                        }
                    }
                    _ => status.say(&help_line()),
                }
            }
            "C" => {
                app.view = if app.view == View::Chat { View::Article } else { View::Chat };
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
    if new == app.sel && app.view == View::Article {
        return;
    }
    if new != app.sel {
        app.chat.clear(); // each element gets its own conversation
    }
    app.sel = new;
    app.view = View::Article;
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

/// Role in human biology. Bulk elements are ~99% of body mass; the
/// macro-minerals and trace metals are established as essential; the
/// last tier is the genuinely debated set.
const LIFE_LEGEND: [(&str, (u8, u8, u8)); 5] = [
    ("bulk", (102, 221, 136)),
    ("mineral", (120, 200, 255)),
    ("trace", (255, 204, 68)),
    ("debated", (170, 140, 200)),
    ("none", (110, 110, 110)),
];

fn life_idx(z: u32) -> usize {
    match z {
        1 | 6 | 7 | 8 | 15 | 16 => 0,                       // H C N O P S
        11 | 12 | 17 | 19 | 20 => 1,                        // Na Mg Cl K Ca
        25..=27 | 29 | 30 | 34 | 42 | 53 => 2,              // Mn Fe Co Cu Zn Se Mo I
        5 | 9 | 14 | 23 | 24 | 28 | 33 => 3,                // B F Si V Cr Ni As
        _ => 4,
    }
}

/// Discovery year as a number: "1922" → 1922, "5000 BC" → -5000,
/// "ancient" → far enough back to count as always known.
fn element_year(e: &Element) -> Option<i32> {
    let y = e.discovered_year.trim();
    if y.is_empty() {
        return None;
    }
    if y == "ancient" {
        return Some(-10000);
    }
    match y.strip_suffix(" BC") {
        Some(n) => n.trim().parse::<i32>().ok().map(|n| -n),
        None => y.parse::<i32>().ok(),
    }
}

fn fmt_year(y: i32) -> String {
    if y < 0 { format!("{} BC", -y) } else { y.to_string() }
}

const YEAR_LEGEND: [(&str, (u8, u8, u8)); 4] = [
    ("just found", (255, 110, 40)),
    ("known", (235, 205, 150)),
    ("not yet", (58, 58, 66)),
    ("no date", (120, 100, 130)),
];

/// Index into YEAR_LEGEND for element `e` as of `year`.
fn year_idx(e: &Element, year: i32) -> usize {
    match element_year(e) {
        None => 3,
        Some(y) if y > year => 2,
        // Flare for a decade after the find, so scrubbing shows the
        // frontier moving rather than a static wall of "known".
        Some(y) if year - y <= 10 => 0,
        Some(_) => 1,
    }
}

const STABILITY_LEGEND: [(&str, (u8, u8, u8)); 3] = [
    ("stable isotope", (102, 221, 136)),
    ("no stable isotope", (255, 119, 102)),
    ("hypothetical", (150, 150, 150)),
];

fn stability_idx(z: u32) -> usize {
    match z {
        119.. => 2,
        // Tc and Pm are the two holes in the stable table; everything
        // from bismuth up is radioactive (Bi-209 since 2003).
        43 | 61 | 83.. => 1,
        _ => 0,
    }
}

fn mode_value(e: &Element, mode: usize) -> Option<f64> {
    match mode {
        5 => e.electronegativity_pauling,
        6 => e.melt,
        7 => e.density.map(|d| d.max(1e-6).log10()),
        9 => e.ionization_energies.first().copied(),
        _ => None,
    }
}

/// Solid / liquid / gas at `t` kelvin, as an index into PHASE_LEGEND.
fn phase_at(e: &Element, t: f64) -> usize {
    match (e.melt, e.boil) {
        (Some(m), Some(b)) => {
            if t < m { 0 } else if t < b { 1 } else { 2 }
        }
        // Melting point only: solid below it, otherwise unknown.
        (Some(m), None) => if t < m { 0 } else { 3 },
        _ => 3,
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

fn cell_rgb(e: &Element, mode: usize, mm: Option<(f64, f64)>, temp_k: f64, year: i32) -> (u8, u8, u8) {
    match mode {
        MODE_TEMP => PHASE_LEGEND[phase_at(e, temp_k)].1,
        10 => LIFE_LEGEND[life_idx(e.number)].1,
        11 => STABILITY_LEGEND[stability_idx(e.number)].1,
        MODE_YEAR => YEAR_LEGEND[year_idx(e, year)].1,
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
        5..=7 | 9 => match (mode_value(e, mode), mm) {
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
    let mut s = if app.mode == MODE_TEMP {
        format!(
            "\x1b[1m{} {} \x1b[38;2;255;170;80m{:.0} K\x1b[0m \x1b[2m({:.0} °C, +/-)\x1b[0m ",
            app.mode + 1,
            MODE_NAMES[app.mode],
            app.temp_k,
            app.temp_k - 273.15
        )
    } else if app.mode == MODE_YEAR {
        let known = app.els.iter().filter(|e| year_idx(e, app.year) < 2).count();
        format!(
            "\x1b[1m{} {} \x1b[38;2;255;170;80m{}\x1b[0m \x1b[2m({} known, +/-)\x1b[0m ",
            app.mode + 1,
            MODE_NAMES[app.mode],
            fmt_year(app.year),
            known
        )
    } else {
        format!("\x1b[1m{} {}\x1b[0m ", app.mode + 1, MODE_NAMES[app.mode])
    };
    let items: &[(&str, (u8, u8, u8))] = match app.mode {
        0 => &CAT_LEGEND,
        1 | MODE_TEMP => &PHASE_LEGEND,
        2 => &ORIGIN_LEGEND,
        3 => &OCC_LEGEND,
        4 => &BLOCK_LEGEND,
        10 => &LIFE_LEGEND,
        11 => &STABILITY_LEGEND,
        MODE_YEAR => &YEAR_LEGEND,
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
        let mm = if (5..=7).contains(&app.mode) || app.mode == 9 {
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
            let (r, g, b) = cell_rgb(e, app.mode, mm, app.temp_k, app.year);
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
    "\x1b[2m←→ Z± · ↑↓ col · 1-9/m color · J/K scroll · / find · c claude · ? help · q quit\x1b[0m"
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
    let text = match app.view {
        View::Help => help_text(),
        View::Chat => chat_text(app),
        View::Modes => modes_text(app),
        View::Article => detail_text(&app.els[app.sel], side),
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
    // Long values (notably the full ionization series) continue on further
    // lines indented to the value column instead of being cut off.
    let w = avail.saturating_sub(14).max(8);
    let mut items: Vec<(&str, String)> = Vec::new();
    if let Some(a) = &e.appearance {
        items.push(("appearance", a.clone()));
    }
    if !e.ionization_energies.is_empty() {
        let list: Vec<String> = e.ionization_energies.iter().map(|v| v.to_string()).collect();
        items.push(("ionization", format!("{} kJ/mol", list.join(", "))));
    }
    if let Some(d) = discovery_line(e) {
        items.push(("discovered", d));
    }
    if let Some(n) = &e.named_by {
        items.push(("named by", n.clone()));
    }
    let mut blocks: Vec<Vec<String>> = items
        .iter()
        .map(|(_, v)| wrap_words(v, w))
        .collect();
    // Over budget: shave lines off the tallest block (ionization, in
    // practice) rather than losing whole rows off the bottom.
    let budget = SIDE_ROWS as usize;
    let mut total = lines.len() + blocks.iter().map(|b| b.len()).sum::<usize>();
    while total > budget {
        let tallest = blocks
            .iter()
            .enumerate()
            .max_by_key(|(_, b)| b.len())
            .map(|(i, _)| i)
            .unwrap_or(0);
        if blocks[tallest].len() <= 1 {
            break;
        }
        blocks[tallest].pop();
        if let Some(last) = blocks[tallest].last_mut() {
            while last.chars().count() >= w {
                last.pop();
            }
            last.push('…');
        }
        total -= 1;
    }
    for ((label, _), block) in items.iter().zip(blocks) {
        for (i, chunk) in block.into_iter().enumerate() {
            if i == 0 {
                lines.push(format!("{dim}{label:<14}{RESET}{chunk}"));
            } else {
                lines.push(format!("{:14}{chunk}", ""));
            }
        }
    }

    let blank = " ".repeat(avail);
    let mut s = String::new();
    for row in 0..SIDE_ROWS {
        s.push_str(&move_to(3 + row, SIDE_X));
        s.push_str(&blank);
    }
    for (i, l) in lines.iter().take(SIDE_ROWS as usize).enumerate() {
        s.push_str(&move_to(3 + i as u16, SIDE_X));
        s.push_str(l);
    }
    print!("{s}");
    std::io::stdout().flush().ok();
}

/// The `m` menu: every color mode, with the digit that reaches it.
fn modes_text(app: &App) -> String {
    let head = "\x1b[1;38;2;247;140;60m";
    let mut s = format!("{head}Color modes{RESET}\n\n");
    for (i, name) in MODE_NAMES.iter().enumerate() {
        let key = if i < DIGIT_MODES {
            format!("{}", i + 1)
        } else {
            " ".to_string()
        };
        let marker = if i == app.mode { "●" } else { " " };
        let line = format!("  {marker} {key:>2}  {name}");
        if i == app.menu_ix {
            s.push_str(&format!("\x1b[7m{}\x1b[0m\n", crust::pad_display(&line, 34)));
        } else {
            s.push_str(&format!("{line}\n"));
        }
    }
    s.push_str("\n\x1b[2mj/k move · ENTER pick · 1-9 direct · Ctrl+←/→ cycle · ESC back\x1b[0m\n");
    s
}

fn help_text() -> String {
    format!(
        "{RUST}elements — keys{RESET}\n\n\
         \x20 ← → / h l           previous / next element (walks the whole table)\n\
         \x20 ↑ ↓ / k j           up / down within the column\n\
         \x20 < >                 same as ← →\n\
         \x20 1-9, Ctrl+← →       color mode: 1 category · 2 phase · 3 cosmic origin ·\n\
         \x20                     4 occurrence · 5 block · 6 electronegativity ·\n\
         \x20                     7 melting point · 8 density (log) · 9 phase at T\n\
         \x20 m                   mode menu (all 13, including the ones past the digits)\n\
         \x20 + -                 in mode 9: temperature ±25 K  (* and _ step 250 K)\n\
         \x20                     in mode 13: year ±5  (* and _ step 250 years)\n\
         \x20 J K / Shift-↓ ↑     scroll the article one line\n\
         \x20 Space, PgDn/PgUp    scroll the article one page\n\
         \x20 g G                 top / bottom of the article\n\
         \x20 /                   find an element (name, symbol, or atomic number)\n\
         \x20 c                   ask Claude about this element (follow-ups keep context)\n\
         \x20 C                   toggle the Claude conversation view\n\
         \x20 w                   open the element's Wikipedia page in the browser\n\
         \x20 u                   re-fetch all data from Wikipedia\n\
         \x20 ?                   toggle this help\n\
         \x20 ESC                 back to the article (quits from the article view)\n\
         \x20 q                   quit\n\n\
         The Claude view runs `claude -p` with this element's data and article as\n\
         context; the conversation resets when you move to another element.\n\n\
         The cosmic-origin mode shows the DOMINANT nucleosynthetic source per\n\
         element (simplified — most elements are a mix of sources).\n\n\
         Structured properties come from the Wikipedia-derived Periodic-Table-JSON\n\
         dataset; each element also carries its full Wikipedia article. Everything\n\
         is cached at ~/.elements/elements.json — the UI never touches the network.\n\
         The hypothesized elements 119–126 are included (g-block row at the bottom)."
    )
}

/// "Dirk Coster (1923)", "1923", or "Henry Cavendish" — whatever is known.
fn discovery_line(e: &Element) -> Option<String> {
    let who = e.discovered_by.as_deref().unwrap_or("").trim();
    let year = e.discovered_year.trim();
    // The dataset sometimes puts a bare date in discovered_by ("5000 BC").
    let who_is_date = who.chars().next().is_some_and(|c| c.is_ascii_digit());
    // "ancient" adds nothing when the discoverer field already carries a
    // date ("unknown, before 3500 BC") — but it does inform "Middle East".
    let year = if year == "ancient" && who.chars().any(|c| c.is_ascii_digit()) {
        ""
    } else {
        year
    };
    match (who.is_empty() || who_is_date, year.is_empty()) {
        (true, true) => (!who.is_empty()).then(|| who.to_string()),
        (true, false) => Some(year.to_string()),
        (false, true) => Some(who.to_string()),
        (false, false) => Some(format!("{who} ({year})")),
    }
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
        if let Some(d) = discovery_line(e) {
            wide("discovered", &d);
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

/// Greedy word wrap at `w` columns (no ANSI in the input).
fn wrap_words(s: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let need = if line.is_empty() { word.chars().count() } else { line.chars().count() + 1 + word.chars().count() };
        if need > w && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
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
            let level = t.chars().take_while(|c| *c == '=').count();
            let title = t.trim_matches(|c: char| c == '=' || c == ' ');
            if TAIL_SECTIONS.contains(&title.to_lowercase().as_str()) {
                break;
            }
            // Deeper sections: indented and progressively more muted.
            out.push(match level {
                2 => format!("\x1b[1;38;2;247;140;60m{title}{RESET}"),
                3 => format!("  \x1b[1;38;2;250;200;130m{title}{RESET}"),
                _ => format!("    \x1b[1;38;2;200;170;140m{title}{RESET}"),
            });
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
            // One blank line between paragraphs: the extract runs them
            // together, which reads as a wall of text in a wide pane.
            if !line.trim().is_empty()
                && matches!(out.last(), Some(l) if !l.trim().is_empty() && !l.contains('\x1b'))
            {
                out.push(String::new());
            }
            out.push(line.to_string());
        }
    }
    // Collapse runs of blank lines to one so spacing stays uniform.
    let mut s = String::with_capacity(a.len() + 2048);
    let mut blank = false;
    for l in out {
        let empty = l.trim().is_empty();
        if empty && blank {
            continue;
        }
        blank = empty;
        s.push_str(&l);
        s.push('\n');
    }
    s
}

// ─────────────────────────── claude chat ─────────────────────────────

/// Pipe `input` into `claude -p <prompt>` and return its answer.
/// Same one-shot pattern the other Fe2O3 apps use (scribe's `:claude`).
fn claude_run(prompt: &str, input: &str) -> Result<String, String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("claude")
        .args(["-p", prompt])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "claude not on PATH".to_string(),
            _ => format!("spawn: {e}"),
        })?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("stdin write: {e}"))?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let snippet = err.lines().next().unwrap_or("(no message)");
        return Err(snippet.chars().take(80).collect());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Ask Claude about the selected element, carrying the earlier turns of
/// this element's conversation so follow-ups work.
fn ask_claude(app: &App, question: &str) -> Result<String, String> {
    let e = &app.els[app.sel];
    let mut ctx = String::new();
    ctx.push_str(&format!(
        "Element: {} ({}), atomic number {}, category {}, phase {}.\n",
        e.name, e.symbol, e.number, e.category, e.phase
    ));
    if let Some(m) = e.atomic_mass {
        ctx.push_str(&format!("Atomic mass: {m} u\n"));
    }
    if !e.electron_configuration_semantic.is_empty() {
        ctx.push_str(&format!(
            "Electron configuration: {}\n",
            e.electron_configuration_semantic
        ));
    }
    if !e.article.is_empty() {
        ctx.push_str("\nWikipedia article (may be truncated):\n");
        let article: String = e.article.chars().take(12000).collect();
        ctx.push_str(&article);
    }
    if !app.chat.is_empty() {
        ctx.push_str("\n\nEarlier in this conversation:\n");
        for (q, a) in &app.chat {
            ctx.push_str(&format!("User: {q}\nYou: {a}\n\n"));
        }
    }
    ctx.push_str(&format!("\n\nUser's question: {question}\n"));

    let prompt = format!(
        "You are a chemistry and physics tutor answering inside a terminal periodic-table app. \
         The user is looking at {}. Answer their question from the reference material and your \
         own knowledge. Plain text only — no markdown headings, bullets with '-' are fine. \
         Keep it tight: a few short paragraphs at most. Do not use any tools; just answer.",
        e.name
    );
    claude_run(&prompt, &ctx)
}

fn chat_text(app: &App) -> String {
    let e = &app.els[app.sel];
    let head = "\x1b[1;38;2;247;140;60m";
    let mut s = format!("{head}Claude — {} ({}){RESET}\n\n", e.name, e.symbol);
    if app.chat.is_empty() {
        s.push_str("\x1b[2mPress c to ask a question about this element.\x1b[0m\n");
        return s;
    }
    for (q, a) in &app.chat {
        s.push_str(&format!("\x1b[1;38;2;120;200;255m? {q}{RESET}\n\n"));
        s.push_str(a);
        s.push_str("\n\n");
    }
    s.push_str("\x1b[2mc: ask a follow-up · ESC: back to the article\x1b[0m\n");
    s
}
