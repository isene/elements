//! One-time data fetch: structured properties from the Wikipedia-derived
//! Periodic-Table-JSON dataset, plus the full Wikipedia article for every
//! element — including the hypothesized period-8 ones. Runs only on first
//! start, `--fetch`, or the `u` key; the TUI loop never touches the network.

use crate::data::Element;
use std::io::Write;
use std::sync::{Arc, Mutex};

const STRUCTURED_URL: &str =
    "https://raw.githubusercontent.com/Bowserinator/Periodic-Table-JSON/master/PeriodicTableJSON.json";

/// Wikidata: atomic number + time of discovery (P575) for every element.
const YEARS_URL: &str = "https://query.wikidata.org/sparql?format=json&query=SELECT%20%3Fnum%20%3Fdate%20WHERE%20%7B%20%3Fe%20wdt%3AP31%20wd%3AQ11344%20%3B%20wdt%3AP1086%20%3Fnum%20%3B%20wdt%3AP575%20%3Fdate%20%7D%20ORDER%20BY%20%3Fnum";

/// Hypothesized elements beyond the structured dataset (which ends at 119).
/// Systematic IUPAC names; Wikipedia has an article or redirect for each.
/// (number, symbol, name, group, period, block)
const HYPOTHESIZED: &[(u32, &str, &str, Option<u32>, u32, &str)] = &[
    (120, "Ubn", "Unbinilium",  Some(2), 8, "s"),
    (121, "Ubu", "Unbiunium",   Some(3), 8, "g"),
    (122, "Ubb", "Unbibium",    None,    8, "g"),
    (123, "Ubt", "Unbitrium",   None,    8, "g"),
    (124, "Ubq", "Unbiquadium", None,    8, "g"),
    (125, "Ubp", "Unbipentium", None,    8, "g"),
    (126, "Ubh", "Unbihexium",  None,    8, "g"),
];

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("elements/0.1 (https://github.com/isene/elements)")
        .build()
}

/// Discovery years keyed by atomic number. Ancient metals that Wikidata
/// leaves undated (Sn, Sb, Hg, Pb, Bi) fall back to "ancient".
pub fn fetch_years() -> Result<std::collections::HashMap<u32, String>, String> {
    let json: serde_json::Value = agent()
        .get(YEARS_URL)
        .set("Accept", "application/sparql-results+json")
        .call()
        .map_err(|e| format!("wikidata: {e}"))?
        .into_json()
        .map_err(|e| format!("wikidata parse: {e}"))?;
    let mut out: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let rows = json["results"]["bindings"]
        .as_array()
        .ok_or("wikidata: no bindings")?;
    for r in rows {
        let num: u32 = match r["num"]["value"].as_str().and_then(|s| s.parse().ok()) {
            Some(n) => n,
            None => continue,
        };
        let date = match r["date"]["value"].as_str() {
            Some(d) => d,
            None => continue,
        };
        // ISO-8601; negative years are BC ("-5000-01-01T…").
        let (year, bc) = match date.strip_prefix('-') {
            Some(rest) => (rest.split('-').next().unwrap_or(""), true),
            None => (date.split('-').next().unwrap_or(""), false),
        };
        let y: i64 = match year.parse() {
            Ok(y) => y,
            Err(_) => continue,
        };
        let label = if bc { format!("{y} BC") } else { y.to_string() };
        // Several sources per element: keep the earliest report.
        out.entry(num)
            .and_modify(|cur| {
                let key = |s: &str| -> i64 {
                    let n: i64 = s.trim_end_matches(" BC").parse().unwrap_or(0);
                    if s.ends_with(" BC") { -n } else { n }
                };
                if key(&label) < key(cur) {
                    *cur = label.clone();
                }
            })
            .or_insert(label);
    }
    for z in [50, 51, 80, 82, 83] {
        out.entry(z).or_insert_with(|| "ancient".to_string());
    }
    Ok(out)
}

pub fn fetch_all() -> Result<Vec<Element>, String> {
    println!("Fetching structured element data …");
    let json: serde_json::Value = agent()
        .get(STRUCTURED_URL)
        .call()
        .map_err(|e| format!("structured data: {e}"))?
        .into_json()
        .map_err(|e| format!("structured data parse: {e}"))?;
    let list = json["elements"].as_array().ok_or("no elements array in dataset")?;
    let mut els: Vec<Element> = list.iter().map(parse_structured).collect();

    println!("Fetching discovery years …");
    match fetch_years() {
        Ok(years) => {
            for e in els.iter_mut() {
                if let Some(y) = years.get(&e.number) {
                    e.discovered_year = y.clone();
                }
            }
        }
        Err(e) => eprintln!("  (discovery years unavailable: {e})"),
    }

    // Hypothesized elements: 120 continues period 8 next to 119; the g-block
    // ones get their own row below the actinides.
    let hyp_row = els.iter().map(|e| e.ypos).max().unwrap_or(10) + 1;
    for &(number, symbol, name, group, period, block) in HYPOTHESIZED {
        let (xpos, ypos) = if number == 120 { (2, 8) } else { (number - 118, hyp_row) };
        els.push(Element {
            number,
            xpos,
            ypos,
            symbol: symbol.into(),
            name: name.into(),
            category: "hypothetical".into(),
            phase: "Unknown".into(),
            group,
            period: Some(period),
            block: block.into(),
            source: format!("https://en.wikipedia.org/wiki/{name}"),
            ..Default::default()
        });
    }

    let total = els.len();
    println!("Fetching the full Wikipedia article for all {total} elements …");
    let els = Arc::new(Mutex::new(els));
    let next = Arc::new(Mutex::new(0usize));
    let done = Arc::new(Mutex::new(0usize));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let els = Arc::clone(&els);
        let next = Arc::clone(&next);
        let done = Arc::clone(&done);
        workers.push(std::thread::spawn(move || {
            let agent = agent();
            loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    let i = *n;
                    *n += 1;
                    i
                };
                if i >= total {
                    break;
                }
                let (title, name) = {
                    let els = els.lock().unwrap();
                    (wiki_title(&els[i]), els[i].name.clone())
                };
                let article = fetch_article(&agent, &title);
                {
                    let mut els = els.lock().unwrap();
                    match article {
                        Ok((resolved, text)) => {
                            if els[i].summary.is_empty() {
                                els[i].summary =
                                    text.split("\n\n").next().unwrap_or("").trim().to_string();
                            }
                            els[i].article = if resolved != title.replace('_', " ") {
                                format!("(Wikipedia redirects to “{resolved}”)\n\n{text}")
                            } else {
                                text
                            };
                        }
                        Err(e) => els[i].article = format!("(article fetch failed: {e})"),
                    }
                }
                let mut d = done.lock().unwrap();
                *d += 1;
                print!("\r  [{:3}/{}] {:<16}", *d, total, name);
                std::io::stdout().flush().ok();
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    println!();
    let mut els = Arc::try_unwrap(els)
        .map_err(|_| "worker thread leaked")?
        .into_inner()
        .unwrap();
    els.sort_by_key(|e| e.number);
    Ok(els)
}

fn parse_structured(v: &serde_json::Value) -> Element {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let os = |k: &str| v[k].as_str().map(|x| x.to_string());
    let f = |k: &str| v[k].as_f64();
    let u = |k: &str| v[k].as_u64().map(|x| x as u32);
    Element {
        number: u("number").unwrap_or(0),
        symbol: s("symbol"),
        name: s("name"),
        category: s("category"),
        phase: s("phase"),
        atomic_mass: f("atomic_mass"),
        density: f("density"),
        melt: f("melt"),
        boil: f("boil"),
        molar_heat: f("molar_heat"),
        group: u("group"),
        period: u("period"),
        block: s("block"),
        xpos: u("xpos").unwrap_or(1),
        ypos: u("ypos").unwrap_or(1),
        shells: v["shells"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default(),
        electron_configuration: s("electron_configuration"),
        electron_configuration_semantic: s("electron_configuration_semantic"),
        electronegativity_pauling: f("electronegativity_pauling"),
        electron_affinity: f("electron_affinity"),
        ionization_energies: v["ionization_energies"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default(),
        appearance: os("appearance"),
        discovered_by: os("discovered_by"),
        named_by: os("named_by"),
        discovered_year: String::new(), // filled from Wikidata below
        summary: s("summary"),
        source: s("source"),
        article: String::new(),
    }
}

fn wiki_title(e: &Element) -> String {
    e.source
        .rsplit_once("/wiki/")
        .map(|(_, t)| t.to_string())
        .unwrap_or_else(|| e.name.clone())
}

/// Full plain-text article via the Wikipedia TextExtracts API.
/// Returns (resolved title, text); follows redirects.
fn fetch_article(agent: &ureq::Agent, title: &str) -> Result<(String, String), String> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=extracts&explaintext=1&redirects=1&format=json&formatversion=2&titles={}",
        title.replace(' ', "%20")
    );
    let mut last_err = String::new();
    for _ in 0..2 {
        match agent.get(&url).call() {
            Ok(resp) => match resp.into_json::<serde_json::Value>() {
                Ok(json) => {
                    let page = &json["query"]["pages"][0];
                    let resolved = page["title"].as_str().unwrap_or(title).to_string();
                    let text = page["extract"].as_str().unwrap_or("").trim().to_string();
                    if text.is_empty() {
                        return Err("empty extract".into());
                    }
                    return Ok((resolved, text));
                }
                Err(e) => last_err = e.to_string(),
            },
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
}
