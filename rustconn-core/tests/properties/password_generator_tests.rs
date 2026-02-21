//! Property tests for password generator functionality

use proptest::prelude::*;
use rustconn_core::password_generator::{
    CharacterSet, PasswordGenerator, PasswordGeneratorConfig, PasswordGeneratorError,
    PasswordStrength, estimate_crack_time,
};

proptest! {
    /// Property: CharacterSet::chars returns non-empty string
    #[test]
    fn character_set_chars_non_empty(_dummy in 0..1) {
        let sets = [
            CharacterSet::Lowercase,
            CharacterSet::Uppercase,
            CharacterSet::Digits,
            CharacterSet::Special,
            CharacterSet::ExtendedSpecial,
        ];

        for set in sets {
            prop_assert!(!set.chars().is_empty());
            prop_assert!(!set.is_empty());
            prop_assert!(set.len() > 0);
        }
    }

    /// Property: CharacterSet::len matches chars length
    #[test]
    fn character_set_len_matches_chars(_dummy in 0..1) {
        let sets = [
            CharacterSet::Lowercase,
            CharacterSet::Uppercase,
            CharacterSet::Digits,
            CharacterSet::Special,
            CharacterSet::ExtendedSpecial,
        ];

        for set in sets {
            prop_assert_eq!(set.len(), set.chars().len());
        }
    }

    /// Property: Lowercase set contains only lowercase letters
    #[test]
    fn lowercase_set_only_lowercase(_dummy in 0..1) {
        let chars = CharacterSet::Lowercase.chars();
        for c in chars.chars() {
            prop_assert!(c.is_ascii_lowercase());
        }
    }

    /// Property: Uppercase set contains only uppercase letters
    #[test]
    fn uppercase_set_only_uppercase(_dummy in 0..1) {
        let chars = CharacterSet::Uppercase.chars();
        for c in chars.chars() {
            prop_assert!(c.is_ascii_uppercase());
        }
    }

    /// Property: Digits set contains only digits
    #[test]
    fn digits_set_only_digits(_dummy in 0..1) {
        let chars = CharacterSet::Digits.chars();
        for c in chars.chars() {
            prop_assert!(c.is_ascii_digit());
        }
    }

    /// Property: PasswordStrength ordering is correct
    #[test]
    fn password_strength_ordering(_dummy in 0..1) {
        prop_assert!(PasswordStrength::VeryWeak < PasswordStrength::Weak);
        prop_assert!(PasswordStrength::Weak < PasswordStrength::Fair);
        prop_assert!(PasswordStrength::Fair < PasswordStrength::Strong);
        prop_assert!(PasswordStrength::Strong < PasswordStrength::VeryStrong);
    }

    /// Property: Each PasswordStrength has a description
    #[test]
    fn password_strength_has_description(_dummy in 0..1) {
        let strengths = [
            PasswordStrength::VeryWeak,
            PasswordStrength::Weak,
            PasswordStrength::Fair,
            PasswordStrength::Strong,
            PasswordStrength::VeryStrong,
        ];

        for strength in strengths {
            let desc = strength.description();
            prop_assert!(!desc.is_empty());
        }
    }

    /// Property: Each PasswordStrength has a color class
    #[test]
    fn password_strength_has_color_class(_dummy in 0..1) {
        let strengths = [
            PasswordStrength::VeryWeak,
            PasswordStrength::Weak,
            PasswordStrength::Fair,
            PasswordStrength::Strong,
            PasswordStrength::VeryStrong,
        ];

        for strength in strengths {
            let class = strength.color_class();
            prop_assert!(!class.is_empty());
        }
    }

    /// Property: Default config has reasonable values
    #[test]
    fn default_config_reasonable(_dummy in 0..1) {
        let config = PasswordGeneratorConfig::default();
        prop_assert_eq!(config.length, 16);
        prop_assert!(config.use_lowercase);
        prop_assert!(config.use_uppercase);
        prop_assert!(config.use_digits);
        prop_assert!(config.use_special);
        prop_assert!(!config.use_extended_special);
        prop_assert!(!config.exclude_ambiguous);
        prop_assert!(config.exclude_chars.is_empty());
        prop_assert!(config.require_all_sets);
    }

    /// Property: Config builder preserves length
    #[test]
    fn config_builder_preserves_length(length in 4usize..100) {
        let config = PasswordGeneratorConfig::new().with_length(length);
        prop_assert_eq!(config.length, length);
    }

    /// Property: Config builder preserves boolean flags
    #[test]
    fn config_builder_preserves_flags(
        lowercase in proptest::bool::ANY,
        uppercase in proptest::bool::ANY,
        digits in proptest::bool::ANY,
        special in proptest::bool::ANY,
        extended in proptest::bool::ANY,
        ambiguous in proptest::bool::ANY,
        require_all in proptest::bool::ANY,
    ) {
        let config = PasswordGeneratorConfig::new()
            .with_lowercase(lowercase)
            .with_uppercase(uppercase)
            .with_digits(digits)
            .with_special(special)
            .with_extended_special(extended)
            .with_exclude_ambiguous(ambiguous)
            .with_require_all_sets(require_all);

        prop_assert_eq!(config.use_lowercase, lowercase);
        prop_assert_eq!(config.use_uppercase, uppercase);
        prop_assert_eq!(config.use_digits, digits);
        prop_assert_eq!(config.use_special, special);
        prop_assert_eq!(config.use_extended_special, extended);
        prop_assert_eq!(config.exclude_ambiguous, ambiguous);
        prop_assert_eq!(config.require_all_sets, require_all);
    }

    /// Property: selected_sets returns correct count
    #[test]
    fn selected_sets_count(
        lowercase in proptest::bool::ANY,
        uppercase in proptest::bool::ANY,
        digits in proptest::bool::ANY,
        special in proptest::bool::ANY,
        extended in proptest::bool::ANY,
    ) {
        let config = PasswordGeneratorConfig::new()
            .with_lowercase(lowercase)
            .with_uppercase(uppercase)
            .with_digits(digits)
            .with_special(special)
            .with_extended_special(extended);

        let mut expected_count = 0;
        if lowercase { expected_count += 1; }
        if uppercase { expected_count += 1; }
        if digits { expected_count += 1; }
        if special { expected_count += 1; }
        if extended { expected_count += 1; }

        prop_assert_eq!(config.selected_sets().len(), expected_count);
    }

    /// Property: Generated password has correct length
    #[test]
    fn generated_password_correct_length(length in 8usize..50) {
        let config = PasswordGeneratorConfig::new().with_length(length);
        let generator = PasswordGenerator::new(config);
        let password = generator.generate().expect("generation should succeed");
        prop_assert_eq!(password.len(), length);
    }

    /// Property: Generated password contains only pool characters
    #[test]
    fn generated_password_uses_pool_chars(length in 8usize..30) {
        let config = PasswordGeneratorConfig::new().with_length(length);
        let pool = config.build_char_pool();
        let generator = PasswordGenerator::new(config);
        let password = generator.generate().expect("generation should succeed");

        for c in password.chars() {
            prop_assert!(pool.contains(c), "Character '{}' not in pool", c);
        }
    }

    /// Property: Excluding ambiguous removes correct characters
    #[test]
    fn exclude_ambiguous_removes_chars(_dummy in 0..1) {
        let config = PasswordGeneratorConfig::new()
            .with_exclude_ambiguous(true)
            .with_length(100);
        let generator = PasswordGenerator::new(config);
        let password = generator.generate().expect("generation should succeed");

        let ambiguous = "0O1lI";
        for c in ambiguous.chars() {
            prop_assert!(!password.contains(c), "Found ambiguous char '{}'", c);
        }
    }

    /// Property: No character sets returns error
    #[test]
    fn no_sets_returns_error(_dummy in 0..1) {
        let config = PasswordGeneratorConfig::new()
            .with_lowercase(false)
            .with_uppercase(false)
            .with_digits(false)
            .with_special(false)
            .with_extended_special(false);
        let generator = PasswordGenerator::new(config);

        let result = generator.generate();
        prop_assert!(matches!(result, Err(PasswordGeneratorError::NoCharacterSets)));
    }

    /// Property: Too short length returns error
    #[test]
    fn too_short_returns_error(length in 0usize..4) {
        let config = PasswordGeneratorConfig::new().with_length(length);
        let generator = PasswordGenerator::new(config);

        let result = generator.generate();
        prop_assert!(matches!(result, Err(PasswordGeneratorError::LengthTooShort(_))));
    }

    /// Property: Entropy increases with password length
    #[test]
    fn entropy_increases_with_length(
        len1 in 8usize..20,
        len2 in 21usize..40,
    ) {
        let config1 = PasswordGeneratorConfig::new().with_length(len1);
        let config2 = PasswordGeneratorConfig::new().with_length(len2);

        let gen1 = PasswordGenerator::new(config1);
        let gen2 = PasswordGenerator::new(config2);

        let pass1 = gen1.generate().expect("gen1");
        let pass2 = gen2.generate().expect("gen2");

        let entropy1 = gen1.calculate_entropy(&pass1);
        let entropy2 = gen2.calculate_entropy(&pass2);

        // Longer password should have more entropy
        prop_assert!(entropy2 > entropy1);
    }

    /// Property: Empty password has zero entropy
    #[test]
    fn empty_password_zero_entropy(_dummy in 0..1) {
        let generator = PasswordGenerator::with_defaults();
        let entropy = generator.calculate_entropy("");
        #[allow(clippy::float_cmp)]
        let is_zero = entropy == 0.0;
        prop_assert!(is_zero);
    }

    /// Property: Strength evaluation is consistent with entropy
    #[test]
    fn strength_consistent_with_entropy(_dummy in 0..1) {
        // Very short password should be weak
        let short_config = PasswordGeneratorConfig::new()
            .with_length(4)
            .with_require_all_sets(false);
        let short_gen = PasswordGenerator::new(short_config);
        let short_pass = short_gen.generate().expect("short");
        let short_strength = short_gen.evaluate_strength(&short_pass);
        prop_assert!(short_strength <= PasswordStrength::Fair);

        // Very long password should be strong
        let long_config = PasswordGeneratorConfig::new().with_length(40);
        let long_gen = PasswordGenerator::new(long_config);
        let long_pass = long_gen.generate().expect("long");
        let long_strength = long_gen.evaluate_strength(&long_pass);
        prop_assert!(long_strength >= PasswordStrength::Strong);
    }

    /// Property: estimate_crack_time returns valid string
    #[test]
    fn crack_time_returns_string(
        entropy in 0.0f64..200.0,
        attempts in 1.0f64..1e15,
    ) {
        let result = estimate_crack_time(entropy, attempts);
        prop_assert!(!result.is_empty());
    }

    /// Property: Zero entropy is instant crack
    #[test]
    fn zero_entropy_instant(_dummy in 0..1) {
        let result = estimate_crack_time(0.0, 1_000_000.0);
        prop_assert_eq!(result, "instant");
    }

    /// Property: High entropy takes long time
    #[test]
    fn high_entropy_long_time(_dummy in 0..1) {
        let result = estimate_crack_time(128.0, 1_000_000_000.0);
        prop_assert!(
            result.contains("years") || result.contains("centuries") || result.contains("millions")
        );
    }

    /// Property: min_length is at least 4
    #[test]
    fn min_length_at_least_4(
        lowercase in proptest::bool::ANY,
        uppercase in proptest::bool::ANY,
        digits in proptest::bool::ANY,
        special in proptest::bool::ANY,
    ) {
        let config = PasswordGeneratorConfig::new()
            .with_lowercase(lowercase)
            .with_uppercase(uppercase)
            .with_digits(digits)
            .with_special(special);

        prop_assert!(config.min_length() >= 4);
    }

    /// Property: build_char_pool contains selected set chars
    #[test]
    fn char_pool_contains_selected_sets(_dummy in 0..1) {
        let config = PasswordGeneratorConfig::new()
            .with_lowercase(true)
            .with_uppercase(true)
            .with_digits(true)
            .with_special(false)
            .with_extended_special(false)
            .with_exclude_ambiguous(false);

        let pool = config.build_char_pool();

        // Should contain lowercase
        prop_assert!(pool.contains('a'));
        prop_assert!(pool.contains('z'));

        // Should contain uppercase
        prop_assert!(pool.contains('A'));
        prop_assert!(pool.contains('Z'));

        // Should contain digits
        prop_assert!(pool.contains('0'));
        prop_assert!(pool.contains('9'));

        // Should NOT contain special
        prop_assert!(!pool.contains('!'));
        prop_assert!(!pool.contains('@'));
    }
}

#[test]
fn test_character_set_equality() {
    assert_eq!(CharacterSet::Lowercase, CharacterSet::Lowercase);
    assert_ne!(CharacterSet::Lowercase, CharacterSet::Uppercase);
}

#[test]
fn test_password_strength_equality() {
    assert_eq!(PasswordStrength::Strong, PasswordStrength::Strong);
    assert_ne!(PasswordStrength::Strong, PasswordStrength::Weak);
}

#[test]
fn test_generator_config_method() {
    let config = PasswordGeneratorConfig::new().with_length(20);
    let generator = PasswordGenerator::new(config);

    assert_eq!(generator.config().length, 20);
}

#[test]
fn test_generator_set_config() {
    let config1 = PasswordGeneratorConfig::new().with_length(10);
    let config2 = PasswordGeneratorConfig::new().with_length(30);

    let mut generator = PasswordGenerator::new(config1);
    assert_eq!(generator.config().length, 10);

    generator.set_config(config2);
    assert_eq!(generator.config().length, 30);
}

#[test]
fn test_exclude_custom_chars() {
    let config = PasswordGeneratorConfig::new()
        .with_exclude_chars("abc123")
        .with_length(100);
    let generator = PasswordGenerator::new(config);
    let password = generator.generate().expect("generation should succeed");

    assert!(!password.contains('a'));
    assert!(!password.contains('b'));
    assert!(!password.contains('c'));
    assert!(!password.contains('1'));
    assert!(!password.contains('2'));
    assert!(!password.contains('3'));
}

#[test]
fn test_password_generator_error_display() {
    let err = PasswordGeneratorError::LengthTooShort(8);
    assert!(err.to_string().contains("8"));

    let err = PasswordGeneratorError::NoCharacterSets;
    assert!(err.to_string().contains("character set"));

    let err = PasswordGeneratorError::GenerationFailed(100);
    assert!(err.to_string().contains("100"));

    let err = PasswordGeneratorError::RngError;
    assert!(err.to_string().contains("Random"));
}

#[test]
fn test_digits_only_password() {
    let config = PasswordGeneratorConfig::new()
        .with_lowercase(false)
        .with_uppercase(false)
        .with_digits(true)
        .with_special(false)
        .with_require_all_sets(false)
        .with_length(10);

    let generator = PasswordGenerator::new(config);
    let password = generator.generate().expect("generation should succeed");

    for c in password.chars() {
        assert!(c.is_ascii_digit());
    }
}
