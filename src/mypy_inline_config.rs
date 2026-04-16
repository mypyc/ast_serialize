/// Lightweight parsing of `# mypy: ...` inline configuration comments.
///
/// This only extracts the options that affect AST parsing/serialization:
/// - `always_true` / `always-true`: names treated as always true in reachability
/// - `always_false` / `always-false`: names treated as always false in reachability
/// - `ignore_errors` / `ignore-errors`: skip function bodies
///
/// The full comment text is still returned to Python for complete config processing.

/// Overrides extracted from a single `# mypy:` comment that affect parsing.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct InlineConfigOverrides {
    pub always_true: Vec<String>,
    pub always_false: Vec<String>,
    pub ignore_errors: Option<bool>,
}

impl InlineConfigOverrides {
    /// Merge another set of overrides into this one (later comments win for
    /// `ignore_errors`; list values are extended).
    pub(crate) fn merge(&mut self, other: InlineConfigOverrides) {
        self.always_true.extend(other.always_true);
        self.always_false.extend(other.always_false);
        if other.ignore_errors.is_some() {
            self.ignore_errors = other.ignore_errors;
        }
    }
}

/// Process all `# mypy:` comments and return combined overrides.
pub(crate) fn resolve_overrides(comments: &[(usize, String)]) -> InlineConfigOverrides {
    let mut result = InlineConfigOverrides::default();
    for (_, text) in comments {
        result.merge(parse_single_mypy_comment(text));
    }
    result
}

/// Parse a single `# mypy:` comment text (the part after `# mypy:`) and extract
/// overrides for options that affect parsing.
///
/// The format follows mypy's `split_directive` + `mypy_comments_to_config_map`:
/// - Comma-separated entries
/// - Each entry is `key=value` or bare `key` (bare key means `True`)
/// - Quoted values preserve commas: `always-true="FOO, BAR"`
/// - Dashes in keys are normalized to underscores
///
/// Only `always_true`, `always_false`, and `ignore_errors` are extracted;
/// all other options are silently ignored.
pub(crate) fn parse_single_mypy_comment(text: &str) -> InlineConfigOverrides {
    let mut result = InlineConfigOverrides::default();

    for (key, value) in split_directive(text) {
        let normalized = key.replace('-', "_");
        match normalized.as_str() {
            "always_true" => {
                for name in split_commas(&value) {
                    if !name.is_empty() {
                        result.always_true.push(name);
                    }
                }
            }
            "always_false" => {
                for name in split_commas(&value) {
                    if !name.is_empty() {
                        result.always_false.push(name);
                    }
                }
            }
            "ignore_errors" => {
                if let Some(b) = parse_bool(&value) {
                    result.ignore_errors = Some(b);
                }
            }
            _ => {}
        }
    }

    result
}

/// Split a directive string on commas, respecting double-quoted sections.
/// Returns `(key, value)` pairs where bare keys get value `"True"`.
fn split_directive(s: &str) -> Vec<(String, String)> {
    let parts = split_respecting_quotes(s);
    parts
        .into_iter()
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some((key, value)) = trimmed.split_once('=') {
                Some((key.trim().to_string(), value.trim().to_string()))
            } else {
                Some((trimmed.to_string(), "True".to_string()))
            }
        })
        .collect()
}

/// Split `s` on commas, but preserve commas inside double quotes.
fn split_respecting_quotes(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        match c {
            ',' => {
                parts.push(current);
                current = String::new();
            }
            '"' => {
                // Consume until closing quote
                for c2 in chars.by_ref() {
                    if c2 == '"' {
                        break;
                    }
                    current.push(c2);
                }
            }
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}

/// Split a value string on commas (for list-type options like `always_true`).
fn split_commas(s: &str) -> Vec<String> {
    s.split(',').map(|part| part.trim().to_string()).collect()
}

/// Parse a boolean value the way configparser does.
/// Returns None for values not recognized by configparser.
fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Some(true),
        "0" | "no" | "false" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_comment() {
        let result = parse_single_mypy_comment("");
        assert_eq!(result, InlineConfigOverrides::default());
    }

    #[test]
    fn test_irrelevant_options() {
        let result = parse_single_mypy_comment("disallow-untyped-defs, warn-return-any");
        assert_eq!(result, InlineConfigOverrides::default());
    }

    #[test]
    fn test_always_true_single() {
        let result = parse_single_mypy_comment("always-true=MYPY");
        assert_eq!(result.always_true, vec!["MYPY"]);
        assert!(result.always_false.is_empty());
        assert_eq!(result.ignore_errors, None);
    }

    #[test]
    fn test_always_false_single() {
        let result = parse_single_mypy_comment("always-false=SLOW_PATH");
        assert!(result.always_true.is_empty());
        assert_eq!(result.always_false, vec!["SLOW_PATH"]);
    }

    #[test]
    fn test_always_true_multiple_quoted() {
        let result = parse_single_mypy_comment("always-true=\"FOO, BAR\"");
        assert_eq!(result.always_true, vec!["FOO", "BAR"]);
    }

    #[test]
    fn test_always_true_and_false() {
        let result = parse_single_mypy_comment("always-true=FAST, always-false=SLOW");
        assert_eq!(result.always_true, vec!["FAST"]);
        assert_eq!(result.always_false, vec!["SLOW"]);
    }

    #[test]
    fn test_ignore_errors_bare() {
        let result = parse_single_mypy_comment("ignore-errors");
        assert_eq!(result.ignore_errors, Some(true));
    }

    #[test]
    fn test_ignore_errors_explicit_true() {
        let result = parse_single_mypy_comment("ignore-errors=True");
        assert_eq!(result.ignore_errors, Some(true));
    }

    #[test]
    fn test_ignore_errors_explicit_false() {
        let result = parse_single_mypy_comment("ignore-errors=False");
        assert_eq!(result.ignore_errors, Some(false));
    }

    #[test]
    fn test_underscore_variant() {
        let result = parse_single_mypy_comment("always_true=FOO");
        assert_eq!(result.always_true, vec!["FOO"]);
    }

    #[test]
    fn test_mixed_relevant_and_irrelevant() {
        let result =
            parse_single_mypy_comment("disallow-untyped-defs, always-true=FLAG, warn-return-any");
        assert_eq!(result.always_true, vec!["FLAG"]);
        assert!(result.always_false.is_empty());
        assert_eq!(result.ignore_errors, None);
    }

    #[test]
    fn test_extra_whitespace() {
        let result = parse_single_mypy_comment("  always-true = FOO ,  always-false = BAR  ");
        assert_eq!(result.always_true, vec!["FOO"]);
        assert_eq!(result.always_false, vec!["BAR"]);
    }

    #[test]
    fn test_merge_overrides() {
        let mut first = parse_single_mypy_comment("always-true=A");
        let second = parse_single_mypy_comment("always-true=B, ignore-errors");
        first.merge(second);
        assert_eq!(first.always_true, vec!["A", "B"]);
        assert_eq!(first.ignore_errors, Some(true));
    }

    #[test]
    fn test_merge_ignore_errors_last_wins() {
        let mut first = parse_single_mypy_comment("ignore-errors=True");
        let second = parse_single_mypy_comment("ignore-errors=False");
        first.merge(second);
        assert_eq!(first.ignore_errors, Some(false));
    }

    #[test]
    fn test_bool_parsing_variants() {
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=yes").ignore_errors,
            Some(true)
        );
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=1").ignore_errors,
            Some(true)
        );
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=on").ignore_errors,
            Some(true)
        );
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=no").ignore_errors,
            Some(false)
        );
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=0").ignore_errors,
            Some(false)
        );
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=off").ignore_errors,
            Some(false)
        );
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=false").ignore_errors,
            Some(false)
        );
        // Unrecognized values are ignored (None), matching configparser
        // which would raise ValueError — Python side handles the error.
        assert_eq!(
            parse_single_mypy_comment("ignore-errors=maybe").ignore_errors,
            None
        );
    }

    #[test]
    fn test_quoted_value_with_spaces() {
        let result = parse_single_mypy_comment("always-true=\"FOO , BAR , BAZ\"");
        assert_eq!(result.always_true, vec!["FOO", "BAR", "BAZ"]);
    }

    #[test]
    fn test_split_respecting_quotes_basic() {
        let parts = split_respecting_quotes("a, b, c");
        assert_eq!(parts, vec!["a", " b", " c"]);
    }

    #[test]
    fn test_split_respecting_quotes_with_quotes() {
        let parts = split_respecting_quotes("x=\"a, b\", y");
        assert_eq!(parts, vec!["x=a, b", " y"]);
    }
}
