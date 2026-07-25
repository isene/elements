//! elements — periodic table explorer for the Fe2O3 suite.
//!
//! A grid of all elements (118 confirmed + the hypothesized period-8 ones)
//! with a scrollable detail pane: full structured properties plus the
//! complete Wikipedia article, served from a local cache
//! (~/.elements/elements.json). The network is touched exactly once — on
//! first start, `--fetch`, or the `u` key — never in the UI loop, which
//! blocks on input with zero idle wakes.

mod data;
mod fetch;

use crust::{Crust, Input, Pane};
use data::Element;
use std::io::Write;

const GRID_X0: u16 = 3; // leftmost cell column
const CELL_W: u16 = 4; // 3-char symbol + gap
const DETAIL_Y: u16 = 16; // first row of the detail pane
const MIN_COLS: u16 = GRID_X0 + 18 * CELL_W; // full 18-group table

const RUST: &str = "\x1b[1;38;2;247;76;0m";
const RESET: &str = "\x1b[0m";

struct App {
    els: Vec<Element>,
    sel: usize,
    max_y: u32,
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
        let text = detail_text(&els[sel]);
        if std::io::stdout().is_terminal() {
            println!("{text}");
        } else {
            println!("{}", crust::strip_ansi(&text));
        }
        return;
    }

    let max_y = els.iter().map(|e| e.ypos).max().unwrap_or(10);
    let mut app = App { els, sel, max_y, show_help: false };

    Crust::init();
    Crust::set_app_identity("Elements");
    let (mut cols, mut rows) = Crust::terminal_size();
    let mut detail = Pane::new(1, DETAIL_Y, cols, rows.saturating_sub(DETAIL_Y).max(1), 253, 0);
    let mut status = Pane::new(1, rows, cols, 1, 244, 0);
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
            "J" => detail.linedown(),
            "K" => detail.lineup(),
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
                set_detail(&app, &mut detail);
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
    set_detail(app, detail);
}

// ─────────────────────────── rendering ───────────────────────────────

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

fn move_to(row: u16, col: u16) -> String {
    format!("\x1b[{};{}H", row, col)
}

fn grid_row(ypos: u32) -> u16 {
    // Periods 1-8 directly below the header; f-block and the hypothesized
    // g-block row sit below a one-row gap.
    if ypos <= 8 { 2 + ypos as u16 } else { 3 + ypos as u16 }
}

fn draw_header(app: &App, _cols: u16) {
    let e = &app.els[app.sel];
    let (r, g, b) = cat_rgb(&e.category);
    print!(
        "{}\x1b[2K {RUST}elements{RESET}  \x1b[1m{}{RESET} ({})  \x1b[2mZ={}\x1b[0m  \x1b[38;2;{r};{g};{b}m{}{RESET}",
        move_to(1, 1),
        e.name,
        e.symbol,
        e.number,
        e.category
    );
    std::io::stdout().flush().ok();
}

fn draw_grid(app: &App, cols: u16) {
    let mut s = String::new();
    if cols < MIN_COLS {
        s.push_str(&move_to(3, 2));
        s.push_str("\x1b[2mterminal too narrow for the table (need 75 cols) — / still works\x1b[0m");
    } else {
        for (i, e) in app.els.iter().enumerate() {
            let col = GRID_X0 + (e.xpos as u16 - 1) * CELL_W;
            let (r, g, b) = cat_rgb(&e.category);
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
    "\x1b[2m←→ Z± · ↑↓ column · J/K scroll · / find · w wiki · u update · ? help · q quit\x1b[0m".to_string()
}

fn draw_all(app: &App, detail: &mut Pane, status: &mut Pane, cols: u16, _rows: u16) {
    Crust::clear_screen();
    draw_header(app, cols);
    draw_grid(app, cols);
    status.invalidate();
    status.say(&help_line());
    detail.invalidate();
    set_detail(app, detail);
}

fn set_detail(app: &App, detail: &mut Pane) {
    let text = if app.show_help { help_text() } else { detail_text(&app.els[app.sel]) };
    detail.set_text(&text);
    detail.ix = 0;
    detail.refresh();
}

fn help_text() -> String {
    format!(
        "{RUST}elements — keys{RESET}\n\n\
         \x20 ← → / h l           previous / next element (walks the whole table)\n\
         \x20 ↑ ↓ / k j           up / down within the column\n\
         \x20 < >                 same as ← →\n\
         \x20 J K                 scroll the article one line\n\
         \x20 Space, PgDn/PgUp    scroll the article one page\n\
         \x20 g G                 top / bottom of the article\n\
         \x20 /                   find an element (name, symbol, or atomic number)\n\
         \x20 w                   open the element's Wikipedia page in the browser\n\
         \x20 u                   re-fetch all data from Wikipedia\n\
         \x20 ?                   toggle this help\n\
         \x20 q, ESC              quit\n\n\
         Structured properties come from the Wikipedia-derived Periodic-Table-JSON\n\
         dataset; each element also carries its full Wikipedia article. Everything\n\
         is cached at ~/.elements/elements.json — the UI never touches the network.\n\
         The hypothesized elements 119–126 are included (grey; g-block row at the\n\
         bottom)."
    )
}

fn kelvin(v: f64) -> String {
    format!("{} K ({:.2} °C)", v, v - 273.15)
}

fn detail_text(e: &Element) -> String {
    let (r, g, b) = cat_rgb(&e.category);
    let dim = "\x1b[2m";
    let head = "\x1b[1;38;2;247;140;60m";
    let mut s = String::new();

    s.push_str(&format!(
        "\x1b[1;38;2;{r};{g};{b}m{} ({}){RESET}  Z={}  \x1b[38;2;{r};{g};{b}m{}{RESET}\n\n",
        e.name, e.symbol, e.number, e.category
    ));

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
    if !phys.is_empty() || !atom.is_empty() {
        s.push_str(&prop_table("Physical", &phys, "Atomic", &atom));
    }

    // Full-width rows for values too long for a table cell.
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

    if !e.summary.is_empty() {
        s.push_str(&format!("\n{head}Summary{RESET}\n{}\n", e.summary));
    }
    if !e.article.is_empty() {
        s.push_str(&format!("\n{head}Wikipedia article{RESET}\n"));
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
        let mut t: String = v.chars().take(w - 1).collect();
        t.push('…');
        t
    }
}

/// Bold the "== Section ==" headings of a Wikipedia plain-text extract.
fn style_article(a: &str) -> String {
    let mut out = String::with_capacity(a.len() + 512);
    for line in a.lines() {
        let t = line.trim();
        if t.len() > 4 && t.starts_with("==") && t.ends_with("==") {
            let title = t.trim_matches(|c: char| c == '=' || c == ' ');
            out.push_str(&format!("\x1b[1;38;2;247;140;60m{title}{RESET}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
