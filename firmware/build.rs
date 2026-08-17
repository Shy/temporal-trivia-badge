use std::{collections::HashMap, fs, path::Path};

fn read_env(path: &Path) -> HashMap<String, String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
            Some((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

fn export(required: &[&str], values: &HashMap<String, String>) {
    for key in required {
        println!(
            "cargo:rustc-env={key}={}",
            values.get(*key).map(String::as_str).unwrap_or("")
        );
    }
}

fn main() {
    embuild::espidf::sysenv::output();
    println!("cargo:rerun-if-env-changed=BADGE_BUILD_UNIX_EPOCH");
    println!(
        "cargo:rustc-env=BADGE_BUILD_UNIX_EPOCH={}",
        std::env::var("BADGE_BUILD_UNIX_EPOCH").unwrap_or_else(|_| "0".to_owned())
    );

    let firmware = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temporal_root = firmware
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("repository is under the Temporal workspace");
    let wifi_path = firmware.join(".env.wifi");
    let temporal_path = temporal_root.join("TrafficLight/.env");

    println!("cargo:rerun-if-changed={}", wifi_path.display());
    println!("cargo:rerun-if-changed={}", temporal_path.display());
    export(
        &["BADGE_WIFI_SSID", "BADGE_WIFI_PASS"],
        &read_env(&wifi_path),
    );
    export(
        &["TEMPORAL_ADDRESS", "TEMPORAL_NAMESPACE", "TEMPORAL_API_KEY"],
        &read_env(&temporal_path),
    );
}
