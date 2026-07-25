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
            "LEFT" | "h" => {
                let t = moved(&app, -1, 0);
                select(&mut app, t, &mut detail, cols);
            }
            "RIGHT" | "l" => {
                let t = moved(&app, 1, 0);
                select(&mut app, t, &mut detail, cols);
            }
            "UP" | "k" => {
                let t = moved(&app, 0, -1);
                select(&mut app, t, &mut detail, cols);
            }
            "DOWN" | "j" => {
                let t = moved(&app, 0, 1);
                select(&mut app, t, &mut detail, cols);
            }
            "<" | "-" => {
                let t = app.sel.saturating_sub(1);
                select(&mut app, t, &mut detail, cols);
            }
            ">" | "+" => {
                let t = (app.sel + 1).min(app.els.len() - 1);
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

/// Walk from the selected cell in a straight line until another element is
/// hit (the table has gaps), or stay put at the edge.
fn moved(app: &App, dx: i32, dy: i32) -> usize {
    let (mut x, mut y) = (app.els[app.sel].xpos as i32, app.els[app.sel].ypos as i32);
    loop {
        x += dx;
        y += dy;
        if !(1..=18).contains(&x) || !(1..=app.max_y as i32).contains(&y) {
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
    "\x1b[2m←↓↑→ move · </> Z± · J/K scroll · / find · w wiki · u update · ? help · q quit\x1b[0m".to_string()
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
         \x20 ← ↑ ↓ → / h j k l   move around the table\n\
         \x20 < >                 previous / next element by atomic number\n\
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
    let mut s = String::new();

    s.push_str(&format!(
        "\x1b[1;38;2;{r};{g};{b}m{} ({}){RESET}  Z={}  \x1b[38;2;{r};{g};{b}m{}{RESET}",
        e.name, e.symbol, e.number, e.category
    ));
    if !e.phase.is_empty() {
        s.push_str(&format!("  {dim}phase{RESET} {}", e.phase));
    }
    s.push('\n');

    let mut line = |props: Vec<String>| {
        if !props.is_empty() {
            s.push_str(&props.join("  ·  "));
            s.push('\n');
        }
    };

    let mut p = Vec::new();
    if let Some(m) = e.atomic_mass {
        p.push(format!("{dim}mass{RESET} {m} u"));
    }
    if let Some(d) = e.density {
        let unit = if e.phase == "Gas" { "g/L" } else { "g/cm³" };
        p.push(format!("{dim}density{RESET} {d} {unit}"));
    }
    if let Some(m) = e.melt {
        p.push(format!("{dim}melt{RESET} {}", kelvin(m)));
    }
    if let Some(bp) = e.boil {
        p.push(format!("{dim}boil{RESET} {}", kelvin(bp)));
    }
    if let Some(h) = e.molar_heat {
        p.push(format!("{dim}molar heat{RESET} {h} J/(mol·K)"));
    }
    line(p);

    let mut p = Vec::new();
    if let Some(gr) = e.group {
        p.push(format!("{dim}group{RESET} {gr}"));
    }
    if let Some(pe) = e.period {
        p.push(format!("{dim}period{RESET} {pe}"));
    }
    if !e.block.is_empty() {
        p.push(format!("{dim}block{RESET} {}", e.block));
    }
    if !e.shells.is_empty() {
        let shells: Vec<String> = e.shells.iter().map(|n| n.to_string()).collect();
        p.push(format!("{dim}shells{RESET} {}", shells.join(",")));
    }
    if !e.electron_configuration_semantic.is_empty() {
        p.push(format!("{dim}config{RESET} {}", e.electron_configuration_semantic));
    }
    line(p);

    let mut p = Vec::new();
    if let Some(en) = e.electronegativity_pauling {
        p.push(format!("{dim}electronegativity{RESET} {en}"));
    }
    if let Some(ea) = e.electron_affinity {
        p.push(format!("{dim}e⁻ affinity{RESET} {ea} kJ/mol"));
    }
    if let Some(ie) = e.ionization_energies.first() {
        p.push(format!("{dim}1st ionization{RESET} {ie} kJ/mol"));
    }
    line(p);

    let mut p = Vec::new();
    if let Some(a) = &e.appearance {
        p.push(format!("{dim}appearance{RESET} {a}"));
    }
    line(p);

    let mut p = Vec::new();
    if let Some(d) = &e.discovered_by {
        p.push(format!("{dim}discovered by{RESET} {d}"));
    }
    if let Some(n) = &e.named_by {
        p.push(format!("{dim}named by{RESET} {n}"));
    }
    line(p);

    let rule = format!("{dim}{}{RESET}\n", "─".repeat(72));
    if !e.summary.is_empty() {
        s.push_str(&rule);
        s.push_str(&e.summary);
        s.push('\n');
    }
    if !e.article.is_empty() {
        s.push_str(&rule);
        s.push_str(&style_article(&e.article));
    }
    s
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
