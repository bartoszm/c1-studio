//! API keys, kept in the system keyring: the Credential Manager on Windows, the Secret
//! Service on Linux (never in a settings file). An environment variable overrides the
//! stored key, for development, tests, and machines without a keyring (WSL often has none).

use crate::AiError;

const SERVICE: &str = "c1studio";

/// Where a provider's key comes from: its account in the credential store and the
/// environment variable that overrides it.
pub struct KeySlot {
    pub account: &'static str,
    pub env: &'static str,
}

pub const ANTHROPIC: KeySlot = KeySlot { account: "anthropic-api-key", env: "ANTHROPIC_API_KEY" };

fn entry(slot: &KeySlot) -> Result<keyring::Entry, AiError> {
    keyring::Entry::new(SERVICE, slot.account).map_err(|e| AiError::Keys(e.to_string()))
}

/// The key to use, if any: the environment's, else the stored one. A machine without a
/// keyring (WSL often has no Secret Service) has no stored key, rather than an error that
/// would keep the AI settings from opening; saving one says what to do instead.
pub fn get(slot: &KeySlot) -> Result<Option<String>, AiError> {
    if let Ok(key) = std::env::var(slot.env)
        && !key.trim().is_empty()
    {
        return Ok(Some(key.trim().to_string()));
    }
    let entry = match keyring::Entry::new(SERVICE, slot.account) {
        Ok(entry) => entry,
        Err(e) if no_keyring(&e) => return Ok(None),
        Err(e) => return Err(AiError::Keys(e.to_string())),
    };
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) if no_keyring(&e) => Ok(None),
        Err(e) => Err(AiError::Keys(e.to_string())),
    }
}

/// The keyring itself is missing or unreachable (no Secret Service on D-Bus, no session
/// bus), as opposed to a key that could not be read from one that is there.
/// Windows always has its Credential Manager: an error there is reported as it was.
fn no_keyring(e: &keyring::Error) -> bool {
    !cfg!(windows) && matches!(e, keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_))
}

/// Stores a key; an empty one removes it.
pub fn set(slot: &KeySlot, key: &str) -> Result<(), AiError> {
    let entry = entry(slot)?;
    let key = key.trim();
    if key.is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AiError::Keys(e.to_string())),
        };
    }
    entry.set_password(key).map_err(|e| {
        let hint = if cfg!(windows) {
            String::new()
        } else {
            format!(" (no keyring to keep it in? Set {} instead)", slot.env)
        };
        AiError::Keys(format!("{e}{hint}"))
    })
}

/// Where the key in use comes from, for the settings screen: `None` without one.
pub fn origin(slot: &KeySlot) -> Option<&'static str> {
    if std::env::var(slot.env).is_ok_and(|k| !k.trim().is_empty()) {
        return Some("environment");
    }
    entry(slot).ok()?.get_password().ok().map(|_| "stored")
}

/// A key shown without giving it away: its first and last few characters.
pub fn masked(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return "…".repeat(3);
    }
    let head: String = chars[..7].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}
