// World data the server shares with the game: the world-layer catalog (world-layers.json, read
// from a player's cache), park layouts and level names.
// Engine/Game/World/{world_layers,world_layer_types,park_rotation,world_names}.h and
// Engine/Vfs/world_layer_scan.cpp (read only).
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;

// ---- World layers ----------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldMap {
    Bam,
    Grom,
    Ftue,
    Stadium1,
    Stadium2,
    Mpr,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // kept as world-layers.json carries them
pub struct WorldLayerNode {
    pub bundle: String,
    pub map: WorldMap,
    pub parent: i32,
    pub autoload: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // kept as world-layers.json carries them
pub struct WorldLayer {
    pub key: String,
    pub label: String,
    pub detail: String,
    pub map: WorldMap,
    pub leaf: u32,
    pub switch_slot: u32,
    pub category: String,
}

#[derive(Clone, Debug, Default)]
pub struct WorldLayerCatalog {
    pub nodes: Vec<WorldLayerNode>,
    pub anchors: Vec<u32>,
    pub layers: Vec<WorldLayer>,
}

static CATALOG: OnceLock<WorldLayerCatalog> = OnceLock::new();

// Installed once at startup, before anything reads it.
pub fn install_world_layer_catalog(catalog: WorldLayerCatalog) {
    let _ = CATALOG.set(catalog);
}

pub fn world_layers() -> &'static [WorldLayer] {
    &CATALOG.get_or_init(WorldLayerCatalog::default).layers
}

pub const WORLD_LAYER_MODES: [&str; 3] = ["default", "on", "off"];

pub fn valid_world_layer_mode(mode: &str) -> bool {
    mode == "default" || mode == "on" || mode == "off"
}

pub fn default_world_layers() -> Vec<String> {
    vec!["default".to_string(); world_layers().len()]
}

pub fn pack_world_layers(choices: &[String]) -> Vec<u8> {
    if choices.len() != world_layers().len() {
        panic!("World layer choices do not match the catalog");
    }
    choices
        .iter()
        .map(|choice| match choice.as_str() {
            "on" => 1,
            "off" => 2,
            "default" => 0,
            _ => panic!("Invalid world layer choice"),
        })
        .collect()
}

fn map_of(key: &str) -> Result<WorldMap, String> {
    Ok(match key {
        "bam" => WorldMap::Bam,
        "grom" => WorldMap::Grom,
        "ftue" => WorldMap::Ftue,
        "stadium_1" => WorldMap::Stadium1,
        "stadium_2" => WorldMap::Stadium2,
        "mpr" => WorldMap::Mpr,
        _ => return Err("Unknown world map in world-layer cache".into()),
    })
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("Missing JSON field: {key}"))
}
fn text(value: &Value, key: &str) -> Result<String, String> {
    field(value, key)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("JSON field {key} must be a string"))
}
fn whole(value: &Value) -> Result<i64, String> {
    value
        .as_i64()
        .or_else(|| value.as_u64().map(|v| v as i64))
        .or_else(|| value.as_f64().map(|v| v as i64))
        .ok_or_else(|| "JSON value must be numeric".to_string())
}
fn list<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    field(value, key)?
        .as_array()
        .ok_or_else(|| format!("JSON field {key} must be an array"))
}

// A catalog written by the game's launcher (world-layers.json), read as it is.
pub fn read_world_layers(file: &Path) -> Result<WorldLayerCatalog, String> {
    let text_value = std::fs::read_to_string(file).map_err(|_| format!("Cannot open {}", file.display()))?;
    let document: Value = serde_json::from_str(&text_value).map_err(|e| e.to_string())?;
    let mut catalog = WorldLayerCatalog::default();
    for node in list(&document, "nodes")? {
        catalog.nodes.push(WorldLayerNode {
            bundle: text(node, "bundle")?,
            map: map_of(&text(node, "map")?)?,
            parent: whole(field(node, "parent")?)? as i32,
            autoload: field(node, "autoload")?
                .as_bool()
                .ok_or_else(|| "JSON field autoload must be a boolean".to_string())?,
        });
    }
    for anchor in list(&document, "anchors")? {
        catalog.anchors.push(whole(anchor)? as u32);
    }
    for row in list(&document, "rows")? {
        catalog.layers.push(WorldLayer {
            key: text(row, "key")?,
            label: text(row, "label")?,
            detail: text(row, "detail")?,
            map: map_of(&text(row, "map")?)?,
            leaf: whole(field(row, "leaf")?)? as u32,
            switch_slot: whole(field(row, "switch")?)? as u32,
            category: text(row, "category")?,
        });
    }
    // The same invariants the game relies on: parents first, one map per path.
    let invalid = || "Invalid world-layer cache".to_string();
    for (i, node) in catalog.nodes.iter().enumerate() {
        let parent = node.parent;
        if parent < -1 || parent >= i as i32 || (parent >= 0 && catalog.nodes[parent as usize].map != node.map) {
            return Err(invalid());
        }
    }
    for &anchor in &catalog.anchors {
        if anchor as usize >= catalog.nodes.len() || catalog.nodes[anchor as usize].parent != -1 {
            return Err(invalid());
        }
    }
    for layer in &catalog.layers {
        if layer.leaf as usize >= catalog.nodes.len()
            || layer.switch_slot as usize >= catalog.nodes.len()
            || catalog.nodes[layer.leaf as usize].map != layer.map
            || catalog.nodes[layer.switch_slot as usize].map != layer.map
        {
            return Err(invalid());
        }
    }
    Ok(catalog)
}

// ---- Parks -----------------------------------------------------------------------------------
pub struct ParkLot {
    pub key: &'static str,
    pub label: &'static str,
    // Flump, mega, skate, street: authored variants in this client.
    pub counts: [u32; 4],
}

pub const PARK_LOTS: [ParkLot; 3] = [
    ParkLot { key: "construction", label: "Construction site / Hedgemont", counts: [8, 4, 4, 9] },
    ParkLot { key: "historic", label: "Piers 1 / Historic", counts: [10, 5, 7, 6] },
    ParkLot { key: "financial", label: "Piers 2 / Financial", counts: [8, 4, 7, 6] },
];
pub const PARK_FAMILIES: [&str; 4] = ["flumppark", "megapark", "skatepark", "streetpark"];
pub const PARK_FAMILY_LABELS: [&str; 4] = ["Flump Park", "Mega Park", "Skate Park", "Street Park"];
pub type ParkChoices = [String; 3];

pub fn park_id(family: usize, variant: u32) -> String {
    format!("{}{}{}", PARK_FAMILIES[family], if variant < 10 { "_0" } else { "_" }, variant)
}

pub fn valid_park(lot: usize, id: &str) -> bool {
    if lot >= PARK_LOTS.len() {
        return false;
    }
    if id.is_empty() || id == "empty" {
        return true;
    }
    for family in 0..PARK_FAMILIES.len() {
        for variant in 1..=PARK_LOTS[lot].counts[family] {
            if id == park_id(family, variant) {
                return true;
            }
        }
    }
    false
}

pub fn park_label(id: &str) -> String {
    if id.is_empty() {
        return "Choose a layout".into();
    }
    if id == "empty" {
        return "Empty lot".into();
    }
    for (i, family) in PARK_FAMILIES.iter().enumerate() {
        let start = format!("{family}_");
        if let Some(rest) = id.strip_prefix(&start) {
            return format!("{} {}", PARK_FAMILY_LABELS[i], rest);
        }
    }
    "Unknown layout".into()
}

// ---- Level names -----------------------------------------------------------------------------
pub fn world_level_short_name(asset: &str) -> String {
    let start = asset.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    let mut name = asset[start..].to_string();
    let mut folded = name.to_ascii_lowercase();
    if folded.starts_with("dingolevel_") {
        name.drain(..11);
        folded.drain(..11);
    }
    if folded.ends_with("_levelroot") {
        name.truncate(name.len() - 10);
    }
    name.replace('_', " ")
}

pub fn world_level_name(asset: &str) -> String {
    let name = world_level_short_name(asset);
    match name.to_ascii_lowercase().as_str() {
        "bam" => "San Vansterdam".into(),
        "ftue island" => "Tutorial Island".into(),
        "mpr" => "Super Ultra Mega Resort".into(),
        "sdm int 001" => "Stadium 1".into(),
        "sdm int 002" => "Stadium 2".into(),
        "isle of grom" => "Isle of Grom".into(),
        "level dev activitysandbox" => "Activity Sandbox (Dev)".into(),
        _ => name,
    }
}

// Multiplayer destinations contain a root and an optional detached level.
pub fn world_destination_asset(destination: &str) -> &str {
    match destination.find('|') {
        None => destination,
        Some(split) if split + 1 < destination.len() => &destination[split + 1..],
        Some(split) => &destination[..split],
    }
}
