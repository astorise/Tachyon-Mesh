use std::{env, fs, path::PathBuf};

fn main() {
    let manifest = PathBuf::from("../../systems/manifest.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let raw = fs::read_to_string(&manifest).expect("systems/manifest.toml must be readable");
    let systems = parse_manifest(&raw);
    let workspace = PathBuf::from("../..");
    for system in &systems {
        let crate_dir = workspace.join("systems").join(&system.crate_name);
        assert!(
            crate_dir.join("Cargo.toml").exists(),
            "systems/manifest.toml references missing crate `{}`",
            system.crate_name
        );
    }

    let mut generated = String::from(
        "#[derive(Clone, Copy, Debug)]\npub struct CatalogEntry {\n    pub slug: &'static str,\n    pub crate_name: &'static str,\n    pub version: &'static str,\n    pub description: &'static str,\n}\n\npub const REGISTERED_SYSTEMS: &[CatalogEntry] = &[\n",
    );
    for system in systems {
        generated.push_str(&format!(
            "    CatalogEntry {{ slug: {:?}, crate_name: {:?}, version: {:?}, description: {:?} }},\n",
            system.slug, system.crate_name, system.version, system.description
        ));
    }
    generated.push_str("];\n");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    fs::write(out_dir.join("static_catalog.rs"), generated)
        .expect("static catalog should be generated");
}

#[derive(Default)]
struct SystemEntry {
    slug: String,
    crate_name: String,
    version: String,
    description: String,
}

fn parse_manifest(raw: &str) -> Vec<SystemEntry> {
    let mut entries = Vec::new();
    let mut current = SystemEntry::default();
    let mut in_entry = false;
    for line in raw.lines().map(str::trim) {
        if line == "[[system]]" {
            if in_entry {
                entries.push(current);
                current = SystemEntry::default();
            }
            in_entry = true;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim() {
            "slug" => current.slug = value,
            "crate_name" => current.crate_name = value,
            "version" => current.version = value,
            "description" => current.description = value,
            _ => {}
        }
    }
    if in_entry {
        entries.push(current);
    }
    entries
}
