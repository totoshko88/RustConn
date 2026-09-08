//! Interactive "ask at connect time" variable specifications.
//!
//! A variable whose *value* is an ASK directive is not stored as a literal —
//! it tells RustConn to prompt the user for the value each time a connection
//! that references it is opened, mirroring Ásbrú Connection Manager's
//! `<ASK:...>` mask. The prompt spec cannot be inlined into a `${...}`
//! reference (that syntax admits only `[a-zA-Z_][a-zA-Z0-9_]*`), so it lives in
//! the variable's value instead and this module parses it.
//!
//! ## Directive syntax
//!
//! | Value | Meaning |
//! |-------|---------|
//! | `@ask:Enter the ticket number` | free-text prompt |
//! | `@ask?:Enter the one-time code` | hidden/secret prompt (input not echoed) |
//! | `@ask:Select host\|prod.example\|staging.example` | choose from a list |
//!
//! The description is everything up to the first `|`; each following
//! `|`-separated segment is a selectable option. A `?` immediately after `ask`
//! marks the input secret. Resolution is a two-step dance the GUI drives:
//! collect the ASK requests referenced by a template, prompt for each, then
//! substitute the answers as connection-scoped variables. Core stays pure — it
//! never blocks for input.

/// A parsed interactive-prompt specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskSpec {
    /// The description shown to the user (never empty after parsing).
    pub prompt: String,
    /// Selectable options; empty for a free-text prompt.
    pub options: Vec<String>,
    /// Whether the entered value is sensitive and must not be echoed or logged.
    pub secret: bool,
}

/// Prefix that marks a variable value as a visible interactive prompt.
const ASK_PREFIX: &str = "@ask:";
/// Prefix that marks a variable value as a hidden (secret) interactive prompt.
const ASK_SECRET_PREFIX: &str = "@ask?:";

impl AskSpec {
    /// Parses a variable value into an [`AskSpec`], or `None` when the value is
    /// an ordinary literal rather than an ASK directive.
    ///
    /// A directive with an empty description (`@ask:` or `@ask:|a|b`) is
    /// rejected as `None`: without a prompt there is nothing to ask, and
    /// treating it as literal is the safe, non-surprising fallback. Empty
    /// option segments (e.g. a trailing `|`) are dropped.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        // Order matters: the secret prefix is a longer match, test it first.
        let (rest, secret) = value
            .strip_prefix(ASK_SECRET_PREFIX)
            .map(|rest| (rest, true))
            .or_else(|| value.strip_prefix(ASK_PREFIX).map(|rest| (rest, false)))?;

        let mut parts = rest.split('|');
        let prompt = parts.next().unwrap_or_default().trim().to_string();
        if prompt.is_empty() {
            return None;
        }

        let options: Vec<String> = parts
            .map(str::trim)
            .filter(|opt| !opt.is_empty())
            .map(ToString::to_string)
            .collect();

        Some(Self {
            prompt,
            options,
            secret,
        })
    }

    /// Returns `true` when the user must choose from a fixed list rather than
    /// type a free value.
    #[must_use]
    pub fn is_choice(&self) -> bool {
        !self.options.is_empty()
    }
}

/// Returns `true` if `value` is an ASK directive (visible or secret).
///
/// Cheaper than [`AskSpec::parse`] when only presence matters; note it returns
/// `true` even for a malformed directive with an empty description, which
/// `parse` rejects — use `parse().is_some()` when you need the well-formed
/// distinction.
#[must_use]
pub fn is_ask_directive(value: &str) -> bool {
    value.starts_with(ASK_PREFIX) || value.starts_with(ASK_SECRET_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_free_text_prompt() {
        let spec = AskSpec::parse("@ask:Enter the ticket number").expect("valid directive");
        assert_eq!(spec.prompt, "Enter the ticket number");
        assert!(spec.options.is_empty());
        assert!(!spec.secret);
        assert!(!spec.is_choice());
    }

    #[test]
    fn parses_a_secret_prompt() {
        let spec = AskSpec::parse("@ask?:One-time code").expect("valid secret directive");
        assert_eq!(spec.prompt, "One-time code");
        assert!(spec.secret);
        assert!(!spec.is_choice());
    }

    #[test]
    fn parses_a_choice_list() {
        let spec =
            AskSpec::parse("@ask:Select host|prod.example|staging.example").expect("valid choice");
        assert_eq!(spec.prompt, "Select host");
        assert_eq!(spec.options, vec!["prod.example", "staging.example"]);
        assert!(spec.is_choice());
        assert!(!spec.secret);
    }

    #[test]
    fn trims_whitespace_around_prompt_and_options() {
        let spec = AskSpec::parse("@ask:  Pick one  |  a  |  b  ").expect("valid");
        assert_eq!(spec.prompt, "Pick one");
        assert_eq!(spec.options, vec!["a", "b"]);
    }

    #[test]
    fn drops_empty_option_segments() {
        // A trailing separator or a doubled `||` must not become a blank choice.
        let spec = AskSpec::parse("@ask:Env|dev||prod|").expect("valid");
        assert_eq!(spec.options, vec!["dev", "prod"]);
    }

    #[test]
    fn a_plain_value_is_not_a_directive() {
        assert!(AskSpec::parse("prod.example.com").is_none());
        assert!(AskSpec::parse("").is_none());
        assert!(AskSpec::parse("ask:no-at-sign").is_none());
        assert!(!is_ask_directive("prod.example.com"));
    }

    #[test]
    fn an_empty_description_is_rejected() {
        assert!(AskSpec::parse("@ask:").is_none());
        assert!(AskSpec::parse("@ask?:").is_none());
        assert!(AskSpec::parse("@ask:|a|b").is_none());
        // ...but is_ask_directive still sees the prefix.
        assert!(is_ask_directive("@ask:"));
    }

    #[test]
    fn a_secret_choice_is_possible_even_if_unusual() {
        let spec = AskSpec::parse("@ask?:Token|aaa|bbb").expect("valid");
        assert!(spec.secret);
        assert_eq!(spec.options, vec!["aaa", "bbb"]);
    }
}
