use anyhow::{Result, bail};

#[derive(Clone, Debug)]
pub struct BadgeIdentity {
    pub id: String,
    pub callsign: String,
}

pub fn factory_identity() -> Result<BadgeIdentity> {
    let mut mac = [0_u8; 6];
    // SAFETY: `mac` provides six writable bytes, which is the buffer contract
    // of `esp_efuse_mac_get_default`; the pointer is not retained after return.
    let result = unsafe { esp_idf_svc::sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) };
    if result != 0 {
        bail!("read factory MAC failed with code {result}");
    }
    let adjectives = [
        "BRAVE", "QUICK", "BOLD", "CALM", "KEEN", "LUCKY", "RUSTY", "SWIFT", "WILD", "BRIGHT",
        "COZY", "EPIC", "NIMBLE", "SOLID", "SUPER", "TINY",
    ];
    let mascots = [
        "CRAB", "FERRIS", "FOX", "OWL", "OTTER", "YAK", "MOTH", "GECKO", "BEAR", "MOOSE", "PANDA",
        "RAVEN", "SEAL", "TIGER", "WOLF", "WREN",
    ];
    let adjective = adjectives[((mac[4] >> 4) & 0x0f) as usize];
    let mascot = mascots[(mac[4] & 0x0f) as usize];
    Ok(BadgeIdentity {
        id: format!(
            "esp32-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ),
        callsign: format!("{adjective}-{mascot}-{:02X}", mac[5]),
    })
}
