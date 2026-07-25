//! Element data model and the on-disk cache (~/.elements/elements.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Element {
    pub number: u32,
    pub symbol: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub phase: String,
    pub atomic_mass: Option<f64>,
    pub density: Option<f64>,
    pub melt: Option<f64>,
    pub boil: Option<f64>,
    pub molar_heat: Option<f64>,
    pub group: Option<u32>,
    pub period: Option<u32>,
    #[serde(default)]
    pub block: String,
    pub xpos: u32,
    pub ypos: u32,
    #[serde(default)]
    pub shells: Vec<u32>,
    #[serde(default)]
    pub electron_configuration: String,
    #[serde(default)]
    pub electron_configuration_semantic: String,
    pub electronegativity_pauling: Option<f64>,
    pub electron_affinity: Option<f64>,
    #[serde(default)]
    pub ionization_energies: Vec<f64>,
    pub appearance: Option<String>,
    pub discovered_by: Option<String>,
    pub named_by: Option<String>,
    #[serde(default)]
    pub summary: String,
    /// Wikipedia page URL.
    #[serde(default)]
    pub source: String,
    /// Full Wikipedia article as plain text.
    #[serde(default)]
    pub article: String,
}

pub fn cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".elements").join("elements.json")
}

pub fn load() -> Option<Vec<Element>> {
    let raw = std::fs::read_to_string(cache_path()).ok()?;
    let mut els: Vec<Element> = serde_json::from_str(&raw).ok()?;
    els.sort_by_key(|e| e.number);
    if els.is_empty() { None } else { Some(els) }
}

pub fn save(els: &[Element]) -> std::io::Result<()> {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Atomic write: a killed fetch must never truncate a good cache.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(els)?)?;
    std::fs::rename(tmp, path)
}
