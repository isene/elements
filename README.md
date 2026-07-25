# elements

<img src="img/elements.svg" align="right" width="150">

**The periodic table in your terminal, with the full Wikipedia article for every element. Written in Rust.**

![Rust](https://img.shields.io/badge/language-Rust-f74c00) ![License](https://img.shields.io/badge/license-Unlicense-green) ![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue) ![Stay Amazing](https://img.shields.io/badge/Stay-Amazing-important)

Interactive periodic table: all 118 confirmed elements plus the hypothesized 119–126. Move around the category-colored 18-group layout and read each element's structured properties and complete Wikipedia article in the pane below — entirely offline. Built on [Crust](https://github.com/isene/crust), part of the [Fe2O3 suite](https://github.com/isene/fe2o3).

## Features

- **Full table**: 118 confirmed elements, plus hypothesized 119–126 (period 8 and the predicted g-block row)
- **Eight color modes** (keys 1-8 or Ctrl+←/→): category, phase at STP, cosmic origin (Big Bang, dying stars, supernovae, neutron-star mergers …), occurrence (primordial / from decay / synthetic), block, electronegativity, melting point, density — with the legend in the header bar
- **Group and period labels** on the table; math markup and reference/link tail sections stripped from the articles
- **Wide-terminal layout**: on terminals ≥151 columns the property table sits beside the periodic table, leaving the pane below for the article
- **Structured data**: mass, density, melt/boil (K and °C), molar heat, group/period/block, shells, electron configuration, electronegativity, electron affinity, ionization energy, appearance, discovery
- **The complete Wikipedia article** for every element, scrollable in the detail pane
- **Offline**: one-time fetch, cached at `~/.elements/elements.json` (~4 MB); the UI never touches the network
- **Scriptable**: `elements tungsten | less` prints plain text when piped
- **Zero idle cost**: event-driven, no timers, no polling
- **Instant**: starts in under 20 ms, cache included

## Install

Download the prebuilt binary from [Releases](https://github.com/isene/elements/releases), or build from source:

```bash
cargo build --release
cp target/release/elements ~/.local/bin/
```

First start fetches the dataset from Wikipedia (about a minute), then everything is local.

## Key Bindings

| Key | Action |
|-----|--------|
| ← →, h/l | Previous / next element by atomic number (walks the whole table, row to row) |
| ↑ ↓, k/j | Up / down within the column |
| < > | Same as ← → |
| 1-8, Ctrl+←/→ | Color mode (category, phase, cosmic origin, occurrence, block, electronegativity, melting point, density) |
| J K, Shift+↓/↑ | Scroll the article one line |
| Space, PgUp/PgDn | Scroll the article one page |
| g G | Top / bottom of the article |
| / | Find an element (name, symbol, or atomic number) |
| w | Open the element's Wikipedia page in the browser |
| u | Re-fetch all data from Wikipedia |
| ? | Help |
| q, ESC | Quit |

## CLI

```
elements [ELEMENT] [--fetch]
```

`ELEMENT` starts at (or, when piped, prints) that element — by name, symbol, or atomic number. `--fetch` rebuilds the local dataset.

## Data

Structured properties come from the Wikipedia-derived [Periodic-Table-JSON](https://github.com/Bowserinator/Periodic-Table-JSON) dataset; each element also carries its full Wikipedia article via the TextExtracts API. The article texts are CC BY-SA (Wikipedia); the code is public domain.

## License

Public domain (Unlicense). Created by [Geir Isene](https://isene.com).
