//! Parse type comments from Python source code

use ruff_python_ast::token::TokenKind;
use ruff_python_parser;
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
use ruff_text_size::Ranged;

/// Individual type comment found in a comment line
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeComment {
    /// A type: ignore with optional error codes
    TypeIgnore(Vec<String>),
    /// A mypy: ignore with optional error codes
    MypyIgnore(Vec<String>),
    /// A type annotation (e.g., `list[int]`)
    TypeAnnotation(String),
}

/// Parse a type comment and extract all parts (type annotation and/or type: ignore).
///
/// # Arguments
///
/// * `comment` - The comment string to parse (should include the leading `#`)
///
/// # Returns
///
/// - `Some(Vec<TypeComment>)` with one or more parts if valid type comment(s) found
/// - `None` if it's not a type comment
///
/// # Examples
///
/// ```
/// use mypy_parser::type_comment::{parse_type_comments, TypeComment};
///
/// // Pure type: ignore
/// let result = parse_type_comments("# type: ignore").unwrap();
/// assert_eq!(result.len(), 1);
///
/// // Type annotation with type: ignore on same line
/// let result = parse_type_comments("# type: int  # type: ignore[arg-type]").unwrap();
/// assert_eq!(result.len(), 2);  // Both annotation and ignore
/// ```
pub fn parse_type_comments(comment: &str) -> Option<Vec<TypeComment>> {
    let mut parts = Vec::new();

    // Remove leading '#' and whitespace
    let trimmed = comment.trim_start_matches('#').trim_start();

    let mypy_comment = trimmed.starts_with("mypy:");
    // Check if it starts with "type:"
    if !trimmed.starts_with("type:") && !mypy_comment {
        return None;
    }

    // Get the part after "type:" or "mypy:"
    let after_type = trimmed["type:".len()..].trim_start();

    // Check if it's a type: ignore comment (without type annotation)
    if after_type.starts_with("ignore") {
        // Check if "ignore" is followed by whitespace, '[', or end of string
        let after_ignore = &after_type["ignore".len()..];
        if after_ignore.is_empty()
            || after_ignore.starts_with(|c: char| c.is_whitespace() || c == '[')
        {
            // Parse as type: ignore
            let error_codes = parse_error_codes(after_ignore);
            if error_codes.is_none() {
                return None;
            }
            if mypy_comment {
                parts.push(TypeComment::MypyIgnore(error_codes.unwrap()));
            } else {
                parts.push(TypeComment::TypeIgnore(error_codes.unwrap()));
            }
            if let Some(hash_pos) = after_type.find('#') {
                // We allow multiple ignore comments per line.
                if let Some(remainder_ignores) = parse_type_comments(&after_type[hash_pos..]) {
                    for part in remainder_ignores {
                        if matches!(
                            part,
                            TypeComment::TypeIgnore(_) | TypeComment::MypyIgnore(_)
                        ) {
                            parts.push(part);
                        }
                    }
                }
            }
            return Some(parts);
        }
    }

    // Anything like `mypy: enable` etc. doesn't have any special meaning in this context.
    if mypy_comment && !after_type.starts_with("ignore") {
        return None;
    }

    // Parse type annotation, stopping at the next '#'
    let (type_annotation, remainder) = if let Some(hash_pos) = after_type.find('#') {
        // There's another comment after the type annotation
        (
            after_type[..hash_pos].trim_end(),
            Some(&after_type[hash_pos..]),
        )
    } else {
        // No trailing comment
        (after_type.trim_end(), None)
    };

    if !type_annotation.is_empty() {
        parts.push(TypeComment::TypeAnnotation(type_annotation.to_string()));
    }

    // Check if there's a "# type: ignore" in the remainder
    if let Some(remainder_str) = remainder {
        // Recursively parse the remainder to check for type: ignore
        if let Some(remainder_parts) = parse_type_comments(remainder_str) {
            // Add any ignore parts found
            for part in remainder_parts {
                if matches!(
                    part,
                    TypeComment::TypeIgnore(_) | TypeComment::MypyIgnore(_)
                ) {
                    parts.push(part);
                }
            }
        }
    }

    if parts.is_empty() { None } else { Some(parts) }
}

fn parse_error_codes(after_ignore: &str) -> Option<Vec<String>> {
    let after_ignore_trimmed = after_ignore.trim_start();

    // Check if there are error codes in brackets
    if after_ignore_trimmed.starts_with('[') {
        // Ensure only whitespace was between 'ignore' and '['
        let whitespace_between = &after_ignore[..after_ignore.len() - after_ignore_trimmed.len()];
        if !whitespace_between.chars().all(char::is_whitespace) {
            return None;
        }

        if let Some(bracket_end) = after_ignore_trimmed.find(']') {
            // Extract the content between brackets
            let codes_str = &after_ignore_trimmed[1..bracket_end];

            // Split by comma and collect error codes
            let error_codes: Vec<String> = codes_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return Some(error_codes);
        }
    }
    // No error codes specified (just "# type: ignore")
    Some(Vec::new())
}

/// Parse a type comment and extract error codes if it contains a type ignore comment.
///
/// # Arguments
///
/// * `comment` - The comment string to parse (should include the leading `#`)
///
/// # Returns
///
/// `Some(Vec<String>)` containing error codes if it contains a type ignore comment,
/// `None` if it doesn't contain a type ignore comment.
/// Error codes are parsed from brackets like `[code1, code2]`.
/// Only whitespace is allowed between 'ignore' and '['.
///
/// # Examples
///
/// ```
/// use mypy_parser::type_comment::parse_type_comment;
///
/// assert_eq!(parse_type_comment("# type: ignore"), Some(vec![]));
/// assert_eq!(parse_type_comment("# type: ignore[arg-type]"), Some(vec!["arg-type".to_string()]));
/// assert_eq!(parse_type_comment("# type: int  # type: ignore[override]"), Some(vec!["override".to_string()]));
/// assert_eq!(parse_type_comment("# regular comment"), None);
/// ```
pub fn parse_type_comment(comment: &str) -> Option<Vec<String>> {
    let parts = parse_type_comments(comment)?;
    // Find the first Ignore part
    for part in parts {
        if let TypeComment::TypeIgnore(codes) = part {
            return Some(codes);
        }
    }
    None
}

/// Parse a function type comments like `# type: (list[int], *str) -> None`.
///
/// # Arguments
///
/// * `comment` - The comment to parse (should *not* include the leading `# type:` part)
///
/// # Returns
///
/// `Some((Vec<String>, <String>))` argument types and return type, if comment looks valid.
/// `None` otherise.
///
/// Note: this function does *not* validate syntactic validity of individual types.
pub fn parse_func_type_comment(comment: &str) -> Option<(Vec<String>, String)> {
    let result = parse_unchecked(comment, ParseOptions::from(Mode::Expression));
    let mut tokens = result.tokens().iter_with_context();

    let mut arg_types = Vec::new();

    // Function type comment must start with `(`.
    let mut token = tokens.next();
    if token?.kind() != TokenKind::Lpar {
        return None;
    }

    // Initialize the argument types loop with empty range.
    // Use peek() heer and below to handle spaces nicely.
    let mut arg_start = tokens.peek()?.start();
    let mut arg_end = arg_start;
    let mut star_stripped = false;
    loop {
        token = tokens.next();
        // If we reach the end before seeing closing `)`, it is invalid type.
        if token.is_none() {
            return None;
        }
        let token = token.unwrap();
        match token.kind() {
            TokenKind::Comma if tokens.nesting() == 1 => {
                arg_types.push(comment[arg_start.to_usize()..arg_end.to_usize()].to_string());
                // Reset the argument types loop after comma.
                arg_start = tokens.peek()?.start();
                arg_end = arg_start;
                star_stripped = false;
            }
            TokenKind::Star | TokenKind::DoubleStar if tokens.nesting() == 1 => {
                // Skip no more than two initial stars.
                if star_stripped {
                    return None;
                }
                if arg_start == token.start() {
                    arg_start = tokens.peek()?.start();
                    arg_end = arg_start;
                    star_stripped = true;
                }
            }
            TokenKind::Rpar if tokens.nesting() == 0 => {
                if arg_end != arg_start {
                    // This logic will allow trailing comma in argument list.
                    arg_types.push(comment[arg_start.to_usize()..arg_end.to_usize()].to_string());
                }
                break;
            }
            _ => {
                // Common case, advance to next token.
                arg_end = token.end();
            }
        }
    }
    token = tokens.next();
    // Token immediately following closing `)` must be `->`.
    if token?.kind() != TokenKind::Rarrow {
        return None;
    }
    // Put the rest in the return type (may be syntactically invalid).
    let ret_type = &comment[tokens.next()?.start().to_usize()..];
    Some((arg_types, ret_type.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_ignore_basic() {
        assert_eq!(parse_type_comment("# type: ignore"), Some(vec![]));
    }

    #[test]
    fn test_type_ignore_with_single_code() {
        assert_eq!(
            parse_type_comment("# type: ignore[arg-type]"),
            Some(vec!["arg-type".to_string()])
        );
        assert_eq!(
            parse_type_comment("# type: ignore[override]"),
            Some(vec!["override".to_string()])
        );
    }

    #[test]
    fn test_type_ignore_with_multiple_codes() {
        assert_eq!(
            parse_type_comment("# type: ignore[arg-type, override]"),
            Some(vec!["arg-type".to_string(), "override".to_string()])
        );
        assert_eq!(
            parse_type_comment("# type: ignore[name-defined,no-untyped-def]"),
            Some(vec![
                "name-defined".to_string(),
                "no-untyped-def".to_string()
            ])
        );
    }

    #[test]
    fn test_type_ignore_with_whitespace() {
        assert_eq!(parse_type_comment("#type: ignore"), Some(vec![]));
        assert_eq!(parse_type_comment("#  type: ignore"), Some(vec![]));
        assert_eq!(parse_type_comment("# type: ignore "), Some(vec![]));

        // Whitespace before bracket is ok
        assert_eq!(
            parse_type_comment("# type: ignore [arg-type]"),
            Some(vec!["arg-type".to_string()])
        );

        // Whitespace around codes
        assert_eq!(
            parse_type_comment("# type: ignore[ arg-type , override ]"),
            Some(vec!["arg-type".to_string(), "override".to_string()])
        );
    }

    #[test]
    fn test_not_type_ignore() {
        assert_eq!(parse_type_comment("# regular comment"), None);
        assert_eq!(parse_type_comment("# TODO: fix this"), None);
        assert_eq!(parse_type_comment("# type: int"), None);

        // Non-whitespace between 'ignore' and '[' should fail
        assert_eq!(parse_type_comment("# type: ignore-[arg-type]"), None);
        assert_eq!(parse_type_comment("# type: ignorefoo[arg-type]"), None);
    }

    #[test]
    fn test_empty_comment() {
        assert_eq!(parse_type_comment("#"), None);
        assert_eq!(parse_type_comment(""), None);
    }

    #[test]
    fn test_empty_error_codes() {
        // Empty brackets should return empty vec
        assert_eq!(parse_type_comment("# type: ignore[]"), Some(vec![]));
    }

    #[test]
    fn test_type_comment_kind_ignore() {
        let result = parse_type_comments("# type: ignore").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeIgnore(codes) if codes.is_empty()));

        let result = parse_type_comments("# type: ignore[arg-type]").unwrap();
        assert_eq!(result.len(), 1);
        assert!(
            matches!(&result[0], TypeComment::TypeIgnore(codes) if codes == &vec!["arg-type".to_string()])
        );
    }

    #[test]
    fn test_type_comment_kind_mypy_ignore() {
        let result = parse_type_comments("# mypy: ignore").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::MypyIgnore(codes) if codes.is_empty()));

        let result = parse_type_comments("# mypy: ignore[arg-type]").unwrap();
        assert_eq!(result.len(), 1);
        assert!(
            matches!(&result[0], TypeComment::MypyIgnore(codes) if codes == &vec!["arg-type".to_string()])
        );

        let result = parse_type_comments("# mypy: ignore[arg-type]  # whatever").unwrap();
        assert_eq!(result.len(), 1);
        assert!(
            matches!(&result[0], TypeComment::MypyIgnore(codes) if codes == &vec!["arg-type".to_string()])
        );
    }

    #[test]
    fn test_type_comment_kind_ignore_mixed() {
        let result = parse_type_comments("# mypy: ignore[foo]  # type: ignore[bar]").unwrap();
        assert_eq!(result.len(), 2);
        assert!(
            matches!(&result[0], TypeComment::MypyIgnore(codes) if codes == &vec!["foo".to_string()])
        );
        assert!(
            matches!(&result[1], TypeComment::TypeIgnore(codes) if codes == &vec!["bar".to_string()])
        );

        let result = parse_type_comments("# type: ignore[foo]  # mypy: ignore[bar]").unwrap();
        assert_eq!(result.len(), 2);
        assert!(
            matches!(&result[0], TypeComment::TypeIgnore(codes) if codes == &vec!["foo".to_string()])
        );
        assert!(
            matches!(&result[1], TypeComment::MypyIgnore(codes) if codes == &vec!["bar".to_string()])
        );
    }

    #[test]
    fn test_type_comment_kind_ignore_mixed_with_suffix() {
        let result =
            parse_type_comments("# mypy: ignore[foo]  # type: ignore[bar]  # whatever").unwrap();
        assert_eq!(result.len(), 2);
        assert!(
            matches!(&result[0], TypeComment::MypyIgnore(codes) if codes == &vec!["foo".to_string()])
        );
        assert!(
            matches!(&result[1], TypeComment::TypeIgnore(codes) if codes == &vec!["bar".to_string()])
        );

        let result =
            parse_type_comments("# type: ignore[foo]  # mypy: ignore[bar]  # whatever").unwrap();
        assert_eq!(result.len(), 2);
        assert!(
            matches!(&result[0], TypeComment::TypeIgnore(codes) if codes == &vec!["foo".to_string()])
        );
        assert!(
            matches!(&result[1], TypeComment::MypyIgnore(codes) if codes == &vec!["bar".to_string()])
        );
    }

    #[test]
    fn test_type_comment_kind_mypy_ignore_with_annotation() {
        // Type annotation followed by mypy: ignore on the same line
        let result = parse_type_comments("# type: str # mypy: ignore").unwrap();
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "str"));
        assert!(matches!(&result[1], TypeComment::MypyIgnore(codes) if codes.is_empty()));

        let result = parse_type_comments("# type: list[int] # mypy: ignore[arg-type]").unwrap();
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "list[int]"));
        assert!(
            matches!(&result[1], TypeComment::MypyIgnore(codes) if codes == &vec!["arg-type".to_string()])
        );
    }

    #[test]
    fn test_type_comment_kind_annotation() {
        let result = parse_type_comments("# type: int").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "int"));

        let result = parse_type_comments("# type: list[int]").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "list[int]"));

        let result = parse_type_comments("# type: Dict[str, int]").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "Dict[str, int]"));
    }

    #[test]
    fn test_type_comment_kind_annotation_with_trailing_comment() {
        let result = parse_type_comments("# type: int  # This is a comment").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "int"));

        let result = parse_type_comments("# type: list[int] # comment").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "list[int]"));
    }

    #[test]
    fn test_type_comment_kind_annotation_with_type_ignore() {
        // Type annotation followed by type: ignore on the same line
        let result = parse_type_comments("# type: str # type: ignore").unwrap();
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "str"));
        assert!(matches!(&result[1], TypeComment::TypeIgnore(codes) if codes.is_empty()));

        let result = parse_type_comments("# type: list[int] # type: ignore[arg-type]").unwrap();
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "list[int]"));
        assert!(
            matches!(&result[1], TypeComment::TypeIgnore(codes) if codes == &vec!["arg-type".to_string()])
        );
    }

    #[test]
    fn test_type_comment_kind_not_type_comment() {
        assert_eq!(parse_type_comments("# regular comment"), None);
        assert_eq!(parse_type_comments("# TODO: fix this"), None);
        assert_eq!(parse_type_comments("# type:"), None); // Empty annotation
    }

    #[test]
    fn test_type_comment_kind_whitespace_handling() {
        let result = parse_type_comments("#type: int").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "int"));

        let result = parse_type_comments("#  type:  int  ").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], TypeComment::TypeAnnotation(s) if s == "int"));
    }

    #[test]
    fn test_function_type_comments_basics() {
        let result = parse_func_type_comment("(dict[str, int], *str) -> None");
        assert_eq!(
            Some((
                vec!["dict[str, int]".to_string(), "str".to_string()],
                "None".to_string()
            )),
            result
        );
    }

    #[test]
    fn test_function_type_comments_spaces() {
        let result = parse_func_type_comment("( dict[str , int], * str ) -> None");
        assert_eq!(
            Some((
                vec!["dict[str , int]".to_string(), "str".to_string()],
                "None".to_string()
            )),
            result
        );
    }

    #[test]
    fn test_function_type_comments_trailing_comma() {
        let result = parse_func_type_comment("(str,) -> None");
        assert_eq!(Some((vec!["str".to_string()], "None".to_string())), result);
    }

    #[test]
    fn test_function_type_comments_empty_ok() {
        let result = parse_func_type_comment("() -> ()");
        assert_eq!(Some((vec![], "()".to_string())), result);
    }

    #[test]
    fn test_function_type_comments_empty_bad() {
        let result = parse_func_type_comment("( , ) -> ");
        assert_eq!(Some((vec!["".to_string()], "".to_string())), result);
    }

    #[test]
    fn test_function_type_comments_triple_star() {
        let result = parse_func_type_comment("(***str) -> None");
        assert_eq!(None, result);
    }

    #[test]
    fn test_function_type_comments_invalid_multiply_kept() {
        let result = parse_func_type_comment("(a * b) -> a * b");
        assert_eq!(
            Some((vec!["a * b".to_string()], "a * b".to_string())),
            result
        );
    }

    #[test]
    fn test_function_type_comments_literal_special() {
        let result = parse_func_type_comment("(Literal[',->']) -> {}");
        assert_eq!(
            Some((vec!["Literal[',->']".to_string()], "{}".to_string())),
            result
        );
    }
}
