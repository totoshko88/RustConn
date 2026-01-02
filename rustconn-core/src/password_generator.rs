//! Password generator module
//!
//! Provides secure password generation with configurable character sets,
//! length, and entropy estimation.

use ring::rand::{SecureRandom, SystemRandom};
use thiserror::Error;

/// Maximum number of attempts to generate a password meeting all requirements
const MAX_ATTEMPTS: usize = 100;

/// Password generator error types
#[derive(Debug, Error)]
pub enum PasswordGeneratorError {
    /// Password length is too short
    #[error("Password length must be at least {0} characters")]
    LengthTooShort(usize),

    /// No character sets selected
    #[error("At least one character set must be selected")]
    NoCharacterSets,

    /// Failed to generate password meeting requirements
    #[error("Failed to generate password meeting all requirements after {0} attempts")]
    GenerationFailed(usize),

    /// Random number generation failed
    #[error("Random number generation failed")]
    RngError,
}

/// Result type for password generator operations
pub type PasswordGeneratorResult<T> = Result<T, PasswordGeneratorError>;

/// Character sets available for password generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterSet {
    /// Lowercase letters (a-z)
    Lowercase,
    /// Uppercase letters (A-Z)
    Uppercase,
    /// Digits (0-9)
    Digits,
    /// Special characters (!@#$%^&*...)
    Special,
    /// Extended special characters (brackets, quotes, etc.)
    ExtendedSpecial,
}

impl CharacterSet {
    /// Returns the characters in this set
    #[must_use]
    pub const fn chars(&self) -> &'static str {
        match self {
            Self::Lowercase => "abcdefghijklmnopqrstuvwxyz",
            Self::Uppercase => "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            Self::Digits => "0123456789",
            Self::Special => "!@#$%^&*-_=+",
            Self::ExtendedSpecial => "()[]{}|;:,.<>?/~`'\"\\",
        }
    }

    /// Returns the number of characters in this set
    #[must_use]
    pub fn len(&self) -> usize {
        self.chars().len()
    }

    /// Returns true if this set is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chars().is_empty()
    }
}

/// Password strength level
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PasswordStrength {
    /// Very weak password (< 28 bits entropy)
    VeryWeak,
    /// Weak password (28-35 bits entropy)
    Weak,
    /// Fair password (36-59 bits entropy)
    Fair,
    /// Strong password (60-127 bits entropy)
    Strong,
    /// Very strong password (>= 128 bits entropy)
    VeryStrong,
}

impl PasswordStrength {
    /// Returns a human-readable description of the strength
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::VeryWeak => "Very Weak",
            Self::Weak => "Weak",
            Self::Fair => "Fair",
            Self::Strong => "Strong",
            Self::VeryStrong => "Very Strong",
        }
    }

    /// Returns a color hint for UI display (CSS class name)
    #[must_use]
    pub const fn color_class(&self) -> &'static str {
        match self {
            Self::VeryWeak => "error",
            Self::Weak => "warning",
            Self::Fair => "accent",
            Self::Strong | Self::VeryStrong => "success",
        }
    }
}

/// Configuration for password generation
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct PasswordGeneratorConfig {
    /// Desired password length
    pub length: usize,
    /// Include lowercase letters
    pub use_lowercase: bool,
    /// Include uppercase letters
    pub use_uppercase: bool,
    /// Include digits
    pub use_digits: bool,
    /// Include special characters
    pub use_special: bool,
    /// Include extended special characters
    pub use_extended_special: bool,
    /// Exclude ambiguous characters (0, O, l, 1, I)
    pub exclude_ambiguous: bool,
    /// Custom characters to exclude
    pub exclude_chars: String,
    /// Require at least one character from each selected set
    pub require_all_sets: bool,
}

impl Default for PasswordGeneratorConfig {
    fn default() -> Self {
        Self {
            length: 16,
            use_lowercase: true,
            use_uppercase: true,
            use_digits: true,
            use_special: true,
            use_extended_special: false,
            exclude_ambiguous: false,
            exclude_chars: String::new(),
            require_all_sets: true,
        }
    }
}

impl PasswordGeneratorConfig {
    /// Creates a new config with default settings
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the password length
    #[must_use]
    pub const fn with_length(mut self, length: usize) -> Self {
        self.length = length;
        self
    }

    /// Enables or disables lowercase letters
    #[must_use]
    pub const fn with_lowercase(mut self, enabled: bool) -> Self {
        self.use_lowercase = enabled;
        self
    }

    /// Enables or disables uppercase letters
    #[must_use]
    pub const fn with_uppercase(mut self, enabled: bool) -> Self {
        self.use_uppercase = enabled;
        self
    }

    /// Enables or disables digits
    #[must_use]
    pub const fn with_digits(mut self, enabled: bool) -> Self {
        self.use_digits = enabled;
        self
    }

    /// Enables or disables special characters
    #[must_use]
    pub const fn with_special(mut self, enabled: bool) -> Self {
        self.use_special = enabled;
        self
    }

    /// Enables or disables extended special characters
    #[must_use]
    pub const fn with_extended_special(mut self, enabled: bool) -> Self {
        self.use_extended_special = enabled;
        self
    }

    /// Enables or disables ambiguous character exclusion
    #[must_use]
    pub const fn with_exclude_ambiguous(mut self, enabled: bool) -> Self {
        self.exclude_ambiguous = enabled;
        self
    }

    /// Sets custom characters to exclude
    #[must_use]
    pub fn with_exclude_chars(mut self, chars: &str) -> Self {
        self.exclude_chars = chars.to_string();
        self
    }

    /// Enables or disables requiring all selected character sets
    #[must_use]
    pub const fn with_require_all_sets(mut self, enabled: bool) -> Self {
        self.require_all_sets = enabled;
        self
    }

    /// Returns the minimum required length based on selected character sets
    #[must_use]
    pub fn min_length(&self) -> usize {
        if self.require_all_sets {
            self.selected_sets().len().max(4)
        } else {
            4
        }
    }

    /// Returns the selected character sets
    #[must_use]
    pub fn selected_sets(&self) -> Vec<CharacterSet> {
        let mut sets = Vec::new();
        if self.use_lowercase {
            sets.push(CharacterSet::Lowercase);
        }
        if self.use_uppercase {
            sets.push(CharacterSet::Uppercase);
        }
        if self.use_digits {
            sets.push(CharacterSet::Digits);
        }
        if self.use_special {
            sets.push(CharacterSet::Special);
        }
        if self.use_extended_special {
            sets.push(CharacterSet::ExtendedSpecial);
        }
        sets
    }

    /// Builds the character pool based on configuration
    #[must_use]
    pub fn build_char_pool(&self) -> String {
        let ambiguous = "0O1lI";
        let mut pool = String::new();

        for set in self.selected_sets() {
            pool.push_str(set.chars());
        }

        // Remove ambiguous characters if requested
        if self.exclude_ambiguous {
            pool = pool.chars().filter(|c| !ambiguous.contains(*c)).collect();
        }

        // Remove custom excluded characters
        if !self.exclude_chars.is_empty() {
            pool = pool
                .chars()
                .filter(|c| !self.exclude_chars.contains(*c))
                .collect();
        }

        pool
    }
}

/// Password generator
pub struct PasswordGenerator {
    config: PasswordGeneratorConfig,
}

impl PasswordGenerator {
    /// Creates a new password generator with the given configuration
    #[must_use]
    pub const fn new(config: PasswordGeneratorConfig) -> Self {
        Self { config }
    }

    /// Creates a password generator with default configuration
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(PasswordGeneratorConfig::default())
    }

    /// Returns a reference to the current configuration
    #[must_use]
    pub const fn config(&self) -> &PasswordGeneratorConfig {
        &self.config
    }

    /// Updates the configuration
    pub fn set_config(&mut self, config: PasswordGeneratorConfig) {
        self.config = config;
    }

    /// Generates a password based on the current configuration
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No character sets are selected
    /// - Password length is too short
    /// - Failed to generate a password meeting all requirements
    ///
    /// # Panics
    ///
    /// Panics if the system random number generator fails (extremely rare).
    pub fn generate(&self) -> PasswordGeneratorResult<String> {
        let sets = self.config.selected_sets();
        if sets.is_empty() {
            return Err(PasswordGeneratorError::NoCharacterSets);
        }

        let min_length = self.config.min_length();
        if self.config.length < min_length {
            return Err(PasswordGeneratorError::LengthTooShort(min_length));
        }

        let pool = self.config.build_char_pool();
        if pool.is_empty() {
            return Err(PasswordGeneratorError::NoCharacterSets);
        }

        let pool_chars: Vec<char> = pool.chars().collect();
        let rng = SystemRandom::new();

        for _ in 0..MAX_ATTEMPTS {
            let password: String = (0..self.config.length)
                .map(|_| {
                    let mut buf = [0u8; 4];
                    rng.fill(&mut buf).expect("RNG fill failed");
                    let idx = u32::from_le_bytes(buf) as usize % pool_chars.len();
                    pool_chars[idx]
                })
                .collect();

            if !self.config.require_all_sets || self.meets_requirements(&password, &sets) {
                return Ok(password);
            }
        }

        Err(PasswordGeneratorError::GenerationFailed(MAX_ATTEMPTS))
    }

    /// Checks if a password meets all character set requirements
    fn meets_requirements(&self, password: &str, sets: &[CharacterSet]) -> bool {
        let ambiguous = "0O1lI";

        for set in sets {
            let set_chars: String = if self.config.exclude_ambiguous {
                set.chars()
                    .chars()
                    .filter(|c| !ambiguous.contains(*c))
                    .collect()
            } else {
                set.chars().to_string()
            };

            // Filter out custom excluded chars
            let set_chars: String = if self.config.exclude_chars.is_empty() {
                set_chars
            } else {
                set_chars
                    .chars()
                    .filter(|c| !self.config.exclude_chars.contains(*c))
                    .collect()
            };

            if !set_chars.is_empty() && !password.chars().any(|c| set_chars.contains(c)) {
                return false;
            }
        }
        true
    }

    /// Calculates the entropy of a password in bits
    #[must_use]
    pub fn calculate_entropy(&self, password: &str) -> f64 {
        if password.is_empty() {
            return 0.0;
        }

        let pool_size = self.config.build_char_pool().len();
        if pool_size == 0 {
            return 0.0;
        }

        #[allow(clippy::cast_precision_loss)]
        let entropy_per_char = (pool_size as f64).log2();
        #[allow(clippy::cast_precision_loss)]
        let total_entropy = entropy_per_char * password.len() as f64;

        total_entropy
    }

    /// Evaluates the strength of a password
    #[must_use]
    pub fn evaluate_strength(&self, password: &str) -> PasswordStrength {
        let entropy = self.calculate_entropy(password);

        if entropy < 28.0 {
            PasswordStrength::VeryWeak
        } else if entropy < 36.0 {
            PasswordStrength::Weak
        } else if entropy < 60.0 {
            PasswordStrength::Fair
        } else if entropy < 128.0 {
            PasswordStrength::Strong
        } else {
            PasswordStrength::VeryStrong
        }
    }
}

/// Estimates the time to crack a password given attempts per second
#[must_use]
pub fn estimate_crack_time(entropy_bits: f64, attempts_per_second: f64) -> String {
    if entropy_bits <= 0.0 || attempts_per_second <= 0.0 {
        return "instant".to_string();
    }

    let combinations = entropy_bits.exp2();
    let seconds = combinations / attempts_per_second / 2.0; // Average case

    format_duration(seconds)
}

/// Formats a duration in seconds to a human-readable string
fn format_duration(seconds: f64) -> String {
    const MINUTE: f64 = 60.0;
    const HOUR: f64 = MINUTE * 60.0;
    const DAY: f64 = HOUR * 24.0;
    const YEAR: f64 = DAY * 365.25;
    const CENTURY: f64 = YEAR * 100.0;

    if seconds < 1.0 {
        "instant".to_string()
    } else if seconds < MINUTE {
        format!("{seconds:.0} seconds")
    } else if seconds < HOUR {
        format!("{:.0} minutes", seconds / MINUTE)
    } else if seconds < DAY {
        format!("{:.0} hours", seconds / HOUR)
    } else if seconds < YEAR {
        format!("{:.0} days", seconds / DAY)
    } else if seconds < CENTURY {
        format!("{:.0} years", seconds / YEAR)
    } else if seconds < CENTURY * 1000.0 {
        format!("{:.0} centuries", seconds / CENTURY)
    } else {
        "millions of years".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PasswordGeneratorConfig::default();
        assert_eq!(config.length, 16);
        assert!(config.use_lowercase);
        assert!(config.use_uppercase);
        assert!(config.use_digits);
        assert!(config.use_special);
        assert!(!config.use_extended_special);
    }

    #[test]
    fn test_generate_password() {
        let generator = PasswordGenerator::with_defaults();
        let password = generator.generate().unwrap();
        assert_eq!(password.len(), 16);
    }

    #[test]
    fn test_generate_with_custom_length() {
        let config = PasswordGeneratorConfig::new().with_length(32);
        let generator = PasswordGenerator::new(config);
        let password = generator.generate().unwrap();
        assert_eq!(password.len(), 32);
    }

    #[test]
    fn test_no_character_sets_error() {
        let config = PasswordGeneratorConfig::new()
            .with_lowercase(false)
            .with_uppercase(false)
            .with_digits(false)
            .with_special(false);
        let generator = PasswordGenerator::new(config);
        assert!(matches!(
            generator.generate(),
            Err(PasswordGeneratorError::NoCharacterSets)
        ));
    }

    #[test]
    fn test_length_too_short_error() {
        let config = PasswordGeneratorConfig::new().with_length(2);
        let generator = PasswordGenerator::new(config);
        assert!(matches!(
            generator.generate(),
            Err(PasswordGeneratorError::LengthTooShort(_))
        ));
    }

    #[test]
    fn test_exclude_ambiguous() {
        let config = PasswordGeneratorConfig::new()
            .with_exclude_ambiguous(true)
            .with_length(100);
        let generator = PasswordGenerator::new(config);
        let password = generator.generate().unwrap();
        assert!(!password.contains('0'));
        assert!(!password.contains('O'));
        assert!(!password.contains('l'));
        assert!(!password.contains('1'));
        assert!(!password.contains('I'));
    }

    #[test]
    fn test_password_strength() {
        // Short password should be weak
        let config = PasswordGeneratorConfig::new().with_length(4);
        let gen = PasswordGenerator::new(config);
        let short_pass = gen.generate().unwrap();
        assert!(gen.evaluate_strength(&short_pass) <= PasswordStrength::Fair);

        // Long password should be strong
        let config = PasswordGeneratorConfig::new().with_length(32);
        let gen = PasswordGenerator::new(config);
        let long_pass = gen.generate().unwrap();
        assert!(gen.evaluate_strength(&long_pass) >= PasswordStrength::Strong);
    }

    #[test]
    fn test_entropy_calculation() {
        let config = PasswordGeneratorConfig::new()
            .with_lowercase(true)
            .with_uppercase(false)
            .with_digits(false)
            .with_special(false)
            .with_require_all_sets(false);
        let generator = PasswordGenerator::new(config);

        // 26 lowercase letters, 10 chars = log2(26) * 10 ≈ 47 bits
        let entropy = generator.calculate_entropy("abcdefghij");
        assert!(entropy > 45.0 && entropy < 50.0);
    }

    #[test]
    fn test_crack_time_estimation() {
        // Very low entropy should be instant
        assert_eq!(estimate_crack_time(0.0, 1_000_000.0), "instant");

        // High entropy should take a long time
        let time = estimate_crack_time(128.0, 1_000_000_000.0);
        assert!(time.contains("years") || time.contains("centuries"));
    }
}
