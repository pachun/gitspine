use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const TRIAL_DAYS: u64 = 14;
const VALIDATION_CACHE_DAYS: u64 = 2; // Re-validate license every 48 hours
const OFFLINE_GRACE_DAYS: u64 = 30; // Allow offline use for 30 days

#[derive(Serialize, Deserialize, Default)]
pub struct LicenseData {
    /// Timestamp of first run (for trial)
    pub first_run: Option<u64>,
    /// License key from Lemon Squeezy
    pub license_key: Option<String>,
    /// Last successful validation timestamp
    pub last_validated: Option<u64>,
    /// Whether the license was valid at last check
    pub is_valid: bool,
    /// Customer name (from validation response)
    pub customer_name: Option<String>,
}

#[derive(Debug)]
pub enum LicenseStatus {
    /// In trial period, X days remaining
    Trial { days_left: u64 },
    /// Trial expired, needs license
    TrialExpired,
    /// Valid license (possibly cached)
    Valid { customer_name: Option<String> },
    /// License invalid or expired subscription
    Invalid,
}

impl LicenseData {
    fn config_dir() -> PathBuf {
        // Use ~/.config/gg for consistency across platforms (XDG convention)
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("gg")
    }

    fn config_path() -> PathBuf {
        Self::config_dir().join("license.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(contents) = fs::read_to_string(&path) {
            match serde_json::from_str(&contents) {
                Ok(data) => data,
                Err(e) => {
                    eprintln!("Warning: failed to parse license file: {}", e);
                    LicenseData::default()
                }
            }
        } else {
            LicenseData::default()
        }
    }

    pub fn save(&self) {
        let dir = Self::config_dir();
        let _ = fs::create_dir_all(&dir);
        let path = Self::config_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn days_since(timestamp: u64) -> u64 {
        let now = Self::now();
        if now > timestamp {
            (now - timestamp) / 86400
        } else {
            0
        }
    }

    /// Initialize first run timestamp if not set, or reset if tampered (set to future)
    pub fn init_trial(&mut self) {
        let now = Self::now();
        let needs_reset = match self.first_run {
            None => true,
            Some(ts) if ts > now => true, // Tampered: set to future
            _ => false,
        };
        if needs_reset {
            self.first_run = Some(now);
            self.save();
        }
    }

    /// Get current license status
    pub fn status(&self) -> LicenseStatus {
        // If we have a license key, check its validity
        if self.license_key.is_some() {
            if self.is_valid {
                // Check if we need to revalidate
                if let Some(last) = self.last_validated {
                    let days_since_validation = Self::days_since(last);
                    // Still within cache period or offline grace period
                    if days_since_validation <= VALIDATION_CACHE_DAYS + OFFLINE_GRACE_DAYS {
                        return LicenseStatus::Valid {
                            customer_name: self.customer_name.clone(),
                        };
                    }
                }
            }
            return LicenseStatus::Invalid;
        }

        // No license - check trial
        if let Some(first_run) = self.first_run {
            let days_used = Self::days_since(first_run);
            if days_used < TRIAL_DAYS {
                LicenseStatus::Trial {
                    days_left: TRIAL_DAYS - days_used,
                }
            } else {
                LicenseStatus::TrialExpired
            }
        } else {
            // First run - full trial
            LicenseStatus::Trial { days_left: TRIAL_DAYS }
        }
    }

    /// Check if user can use the app
    pub fn can_use(&self) -> bool {
        match self.status() {
            LicenseStatus::Trial { .. } => true,
            LicenseStatus::Valid { .. } => true,
            LicenseStatus::TrialExpired => false,
            LicenseStatus::Invalid => false,
        }
    }

    /// Validate license key with Lemon Squeezy API
    pub fn validate_license(&mut self, key: &str) -> Result<(), String> {
        self.license_key = Some(key.to_string());

        // Try to validate online
        match self.validate_online(key) {
            Ok((valid, customer_name)) => {
                self.is_valid = valid;
                self.last_validated = Some(Self::now());
                self.customer_name = customer_name;
                self.save();
                if valid {
                    Ok(())
                } else {
                    Err("License key is invalid or subscription expired".to_string())
                }
            }
            Err(e) => {
                // Network error - if we had a previously valid license, allow offline grace
                if self.is_valid && self.last_validated.is_some() {
                    Ok(()) // Allow continued use
                } else {
                    Err(format!("Could not validate license: {}", e))
                }
            }
        }
    }

    fn validate_online(&self, key: &str) -> Result<(bool, Option<String>), String> {
        // Lemon Squeezy license validation API
        let url = "https://api.lemonsqueezy.com/v1/licenses/validate";

        let response = ureq::post(url)
            .set("Accept", "application/json")
            .set("Content-Type", "application/json")
            .send_json(ureq::json!({
                "license_key": key
            }));

        match response {
            Ok(resp) => {
                let json: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
                let valid = json["valid"].as_bool().unwrap_or(false);
                let customer_name = json["meta"]["customer_name"].as_str().map(|s| s.to_string());
                Ok((valid, customer_name))
            }
            Err(ureq::Error::Status(code, _)) => {
                Err(format!("API returned status {}", code))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Attempt to revalidate if needed (call periodically)
    pub fn maybe_revalidate(&mut self) {
        if let Some(key) = self.license_key.clone() {
            if let Some(last) = self.last_validated {
                if Self::days_since(last) > VALIDATION_CACHE_DAYS {
                    // Try to revalidate, but don't fail if offline
                    let _ = self.validate_license(&key);
                }
            }
        }
    }
}

/// Format license status for display in help menu
pub fn status_line(license: &LicenseData) -> (String, bool) {
    match license.status() {
        LicenseStatus::Trial { days_left } => {
            (format!("{} days left - L to purchase", days_left), false)
        }
        LicenseStatus::Valid { customer_name } => {
            let name = customer_name.unwrap_or_else(|| "Licensed".to_string());
            (name, true)
        }
        LicenseStatus::TrialExpired => {
            ("Trial expired - L to purchase".to_string(), false)
        }
        LicenseStatus::Invalid => {
            ("License invalid - L to renew".to_string(), false)
        }
    }
}
