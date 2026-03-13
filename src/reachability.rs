use crate::options::Options;
use ruff_python_ast as ast;

/// Inferred truth value of an expression during reachability analysis.
///
/// These values match the constants in mypy.reachability:
/// - ALWAYS_TRUE: Expression is always true
/// - MYPY_TRUE: True in mypy, False at runtime
/// - ALWAYS_FALSE: Expression is always false
/// - MYPY_FALSE: False in mypy, True at runtime
/// - TRUTH_VALUE_UNKNOWN: Truth value cannot be determined
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TruthValue {
    AlwaysTrue = 1,
    MypyTrue = 2,
    AlwaysFalse = 3,
    MypyFalse = 4,
    TruthValueUnknown = 5,
}

/// Represents different forms of sys.version_info access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysVersionInfo {
    /// sys.version_info (whole tuple, same as sys.version_info[:])
    Whole,
    /// sys.version_info[index] (single element access)
    Index(i32),
    /// sys.version_info[begin:end] (slice access)
    Slice(Option<i32>, Option<i32>),
}

/// Represents an integer or tuple of integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntOrTuple {
    /// Single integer value
    Int(i32),
    /// Tuple of integer values
    Tuple(Vec<i32>),
}

impl TruthValue {
    /// Returns the inverted truth value (for handling `not` expressions).
    pub fn invert(self) -> Self {
        match self {
            TruthValue::AlwaysTrue => TruthValue::AlwaysFalse,
            TruthValue::AlwaysFalse => TruthValue::AlwaysTrue,
            TruthValue::MypyTrue => TruthValue::MypyFalse,
            TruthValue::MypyFalse => TruthValue::MypyTrue,
            TruthValue::TruthValueUnknown => TruthValue::TruthValueUnknown,
        }
    }
}

/// Reverse a comparison operator (for swapping operands).
fn reverse_cmp_op(op: ast::CmpOp) -> ast::CmpOp {
    match op {
        ast::CmpOp::Eq => ast::CmpOp::Eq,
        ast::CmpOp::NotEq => ast::CmpOp::NotEq,
        ast::CmpOp::Lt => ast::CmpOp::Gt,
        ast::CmpOp::LtE => ast::CmpOp::GtE,
        ast::CmpOp::Gt => ast::CmpOp::Lt,
        ast::CmpOp::GtE => ast::CmpOp::LtE,
        op => op, // Other operators unchanged
    }
}

/// Perform a fixed comparison between two values with a given operator.
/// Returns AlwaysTrue if the comparison is true, AlwaysFalse if false.
fn fixed_comparison<T: PartialOrd>(left: T, op: ast::CmpOp, right: T) -> TruthValue {
    let result = match op {
        ast::CmpOp::Eq => left == right,
        ast::CmpOp::NotEq => left != right,
        ast::CmpOp::Lt => left < right,
        ast::CmpOp::LtE => left <= right,
        ast::CmpOp::Gt => left > right,
        ast::CmpOp::GtE => left >= right,
        // Other operators don't apply in this context
        _ => return TruthValue::TruthValueUnknown,
    };

    if result {
        TruthValue::AlwaysTrue
    } else {
        TruthValue::AlwaysFalse
    }
}

/// Extract an integer value from a NumberLiteral expression.
fn expr_to_int(expr: &ast::Expr) -> Option<i32> {
    if let ast::Expr::NumberLiteral(num) = expr {
        if let ast::Number::Int(int_val) = &num.value {
            return int_val.as_i32();
        }
    }
    None
}

/// Check if an expression is an integer or tuple of integers.
fn contains_int_or_tuple_of_ints(expr: &ast::Expr) -> Option<IntOrTuple> {
    // Check for single integer
    if let Some(int_val) = expr_to_int(expr) {
        return Some(IntOrTuple::Int(int_val));
    }

    // Check for tuple of integers
    if let ast::Expr::Tuple(tuple) = expr {
        let mut values = Vec::with_capacity(tuple.elts.len());
        for item in &tuple.elts {
            let int_val = expr_to_int(item)?;
            values.push(int_val);
        }
        return Some(IntOrTuple::Tuple(values));
    }

    None
}

/// Check if an expression is an attribute access on 'sys' with the given name.
/// For example, `is_sys_attr(expr, "platform")` returns true for `sys.platform`.
fn is_sys_attr(expr: &ast::Expr, name: &str) -> bool {
    if let ast::Expr::Attribute(attr) = expr {
        if attr.attr.as_str() == name {
            if let ast::Expr::Name(base_name) = &*attr.value {
                return base_name.id.as_str() == "sys";
            }
        }
    }
    false
}

/// Check if an expression contains a sys.version_info access pattern.
/// Returns the type of access (whole, index, or slice) if found.
fn contains_sys_version_info(expr: &ast::Expr) -> Option<SysVersionInfo> {
    // Check for bare sys.version_info
    if is_sys_attr(expr, "version_info") {
        return Some(SysVersionInfo::Whole);
    }

    // Check for sys.version_info[...] subscript
    if let ast::Expr::Subscript(subscript) = expr {
        if !is_sys_attr(&subscript.value, "version_info") {
            return None;
        }

        match &*subscript.slice {
            // sys.version_info[index] - single integer index
            ast::Expr::NumberLiteral(_) => {
                let index = expr_to_int(&subscript.slice)?;
                return Some(SysVersionInfo::Index(index));
            }
            // sys.version_info[begin:end] - slice
            ast::Expr::Slice(slice) => {
                // Check stride is None or 1
                if let Some(stride) = &slice.step {
                    if expr_to_int(stride)? != 1 {
                        return None;
                    }
                }

                // Extract begin and end values
                let begin = if let Some(lower) = &slice.lower {
                    Some(expr_to_int(lower)?)
                } else {
                    None
                };
                let end = if let Some(upper) = &slice.upper {
                    Some(expr_to_int(upper)?)
                } else {
                    None
                };

                return Some(SysVersionInfo::Slice(begin, end));
            }
            _ => {}
        }
    }

    None
}

/// Check if a name corresponds to a special constant with known truth value.
fn check_name_truth_value(name: &str, options: &Options) -> TruthValue {
    match name {
        "MYPY" | "TYPE_CHECKING" => TruthValue::MypyTrue,
        "PY2" => TruthValue::AlwaysFalse,
        "PY3" => TruthValue::AlwaysTrue,
        _ => {
            // Check user-provided always_true/always_false lists
            if options.always_true().iter().any(|s| s == name) {
                TruthValue::AlwaysTrue
            } else if options.always_false().iter().any(|s| s == name) {
                TruthValue::AlwaysFalse
            } else {
                TruthValue::TruthValueUnknown
            }
        }
    }
}

/// Combine truth values using a boolean operator (and/or).
fn combine_bool_op(op: ast::BoolOp, values: &[TruthValue]) -> TruthValue {
    // Track what truth values we've seen (efficient single-pass)
    let mut has_always_true = false;
    let mut has_mypy_true = false;
    let mut has_always_false = false;
    let mut has_mypy_false = false;
    let mut has_unknown = false;

    for &val in values {
        match val {
            TruthValue::AlwaysTrue => has_always_true = true,
            TruthValue::MypyTrue => has_mypy_true = true,
            TruthValue::AlwaysFalse => has_always_false = true,
            TruthValue::MypyFalse => has_mypy_false = true,
            TruthValue::TruthValueUnknown => has_unknown = true,
        }
    }

    match op {
        ast::BoolOp::Or => {
            if has_always_true {
                TruthValue::AlwaysTrue
            } else if has_mypy_true {
                TruthValue::MypyTrue
            } else if !has_always_false && !has_unknown && has_mypy_false {
                // All values are MYPY_FALSE
                TruthValue::MypyFalse
            } else if !has_unknown && !has_always_true && !has_mypy_true {
                // All values are ALWAYS_FALSE or MYPY_FALSE
                TruthValue::AlwaysFalse
            } else {
                TruthValue::TruthValueUnknown
            }
        }
        ast::BoolOp::And => {
            if has_always_false {
                TruthValue::AlwaysFalse
            } else if has_mypy_false {
                TruthValue::MypyFalse
            } else if !has_mypy_true && !has_unknown && has_always_true {
                // All values are ALWAYS_TRUE
                TruthValue::AlwaysTrue
            } else if !has_unknown && !has_always_false && !has_mypy_false {
                // All values are ALWAYS_TRUE or MYPY_TRUE
                TruthValue::MypyTrue
            } else {
                TruthValue::TruthValueUnknown
            }
        }
    }
}

/// Consider whether expr is a comparison involving sys.version_info.
pub(crate) fn consider_sys_version_info(expr: &ast::Expr, options: &Options) -> TruthValue {
    let python_version = options.python_version();
    let ast::Expr::Compare(compare) = expr else {
        return TruthValue::TruthValueUnknown;
    };

    // Don't support chained comparisons
    if compare.ops.len() > 1 {
        return TruthValue::TruthValueUnknown;
    }

    let mut op = compare.ops[0];
    // Only support standard comparison operators
    if !matches!(
        op,
        ast::CmpOp::Eq
            | ast::CmpOp::NotEq
            | ast::CmpOp::Lt
            | ast::CmpOp::LtE
            | ast::CmpOp::Gt
            | ast::CmpOp::GtE
    ) {
        return TruthValue::TruthValueUnknown;
    }

    // Try to extract sys.version_info pattern from left and int/tuple from right
    let mut index = contains_sys_version_info(&compare.left);
    let mut thing = contains_int_or_tuple_of_ints(&compare.comparators[0]);

    // If that didn't work, try the reverse
    if index.is_none() || thing.is_none() {
        index = contains_sys_version_info(&compare.comparators[0]);
        thing = contains_int_or_tuple_of_ints(&compare.left);
        op = reverse_cmp_op(op);
    }

    let Some(index) = index else {
        return TruthValue::TruthValueUnknown;
    };
    let Some(thing) = thing else {
        return TruthValue::TruthValueUnknown;
    };

    match (index, thing) {
        // Handle sys.version_info[i] <compare_op> k
        (SysVersionInfo::Index(idx), IntOrTuple::Int(value)) => {
            if idx >= 0 && idx <= 1 {
                let version_component = if idx == 0 {
                    python_version.0 as i32
                } else {
                    python_version.1 as i32
                };
                return fixed_comparison(version_component, op, value);
            } else {
                return TruthValue::TruthValueUnknown;
            }
        }

        // Handle sys.version_info[lo:hi] <compare_op> tuple or sys.version_info <compare_op> tuple
        (index, IntOrTuple::Tuple(target_tuple)) => {
            let (lo, hi) = match index {
                SysVersionInfo::Slice(begin, end) => {
                    let lo = begin.unwrap_or(0);
                    let hi = end.unwrap_or(2);
                    (lo, hi)
                }
                SysVersionInfo::Whole => (0, 2), // sys.version_info is same as [0:2]
                _ => return TruthValue::TruthValueUnknown,
            };

            // Validate bounds: 0 <= lo < hi <= 2
            if lo < 0 || hi > 2 || lo >= hi {
                return TruthValue::TruthValueUnknown;
            }

            // Extract version slice
            let version_slice: Vec<i32> = match (lo, hi) {
                (0, 1) => vec![python_version.0 as i32],
                (0, 2) => vec![python_version.0 as i32, python_version.1 as i32],
                (1, 2) => vec![python_version.1 as i32],
                _ => return TruthValue::TruthValueUnknown,
            };

            // Check length compatibility
            let version_len = version_slice.len();
            let target_len = target_tuple.len();

            // Allow comparison if lengths match, or if version is longer and op is not == or !=
            if version_len == target_len
                || (version_len > target_len && !matches!(op, ast::CmpOp::Eq | ast::CmpOp::NotEq))
            {
                // Compare tuples lexicographically
                return fixed_comparison(version_slice.as_slice(), op, target_tuple.as_slice());
            }

            TruthValue::TruthValueUnknown
        }

        // Other combinations not supported
        _ => TruthValue::TruthValueUnknown,
    }
}

/// Consider whether expr is a comparison involving sys.platform.
pub(crate) fn consider_sys_platform(expr: &ast::Expr, options: &Options) -> TruthValue {
    let platform = options.platform();
    match expr {
        ast::Expr::Compare(compare) => {
            // Don't support chained comparisons
            if compare.ops.len() > 1 {
                return TruthValue::TruthValueUnknown;
            }

            let op = compare.ops[0];
            // Only support == and !=
            if !matches!(op, ast::CmpOp::Eq | ast::CmpOp::NotEq) {
                return TruthValue::TruthValueUnknown;
            }

            // Check if left operand is sys.platform
            if !is_sys_attr(&compare.left, "platform") {
                return TruthValue::TruthValueUnknown;
            }

            // Check if right operand is a string literal
            if let ast::Expr::StringLiteral(string_lit) = &compare.comparators[0] {
                return fixed_comparison(platform, op, string_lit.value.to_str());
            }

            TruthValue::TruthValueUnknown
        }
        ast::Expr::Call(call) => {
            // Check if callee is an attribute expression
            let ast::Expr::Attribute(attr) = &*call.func else {
                return TruthValue::TruthValueUnknown;
            };

            // Check that there's exactly one argument
            if call.arguments.args.len() != 1 {
                return TruthValue::TruthValueUnknown;
            }

            // Check if the attribute base is sys.platform
            if !is_sys_attr(&attr.value, "platform") {
                return TruthValue::TruthValueUnknown;
            }

            // Check if the method is "startswith"
            if attr.attr.as_str() != "startswith" {
                return TruthValue::TruthValueUnknown;
            }

            // Check if the argument is a string literal
            if let ast::Expr::StringLiteral(string_lit) = &call.arguments.args[0] {
                if platform.starts_with(string_lit.value.to_str()) {
                    return TruthValue::AlwaysTrue;
                } else {
                    return TruthValue::AlwaysFalse;
                }
            }

            TruthValue::TruthValueUnknown
        }
        _ => TruthValue::TruthValueUnknown,
    }
}

/// Infer whether a given statement is an assert that will always fail.
pub(crate) fn assert_will_always_fail(stmt: &ast::Stmt, options: &Options) -> bool {
    if let ast::Stmt::Assert(a) = stmt {
        let truth_value = infer_condition_value(a.test.as_ref(), options);
        return truth_value == TruthValue::AlwaysFalse || truth_value == TruthValue::MypyFalse;
    }
    false
}

/// Infer whether the given condition is always true/false.
pub(crate) fn infer_condition_value(expr: &ast::Expr, options: &Options) -> TruthValue {
    match expr {
        // Handle unary "not" expressions
        ast::Expr::UnaryOp(unary) if matches!(unary.op, ast::UnaryOp::Not) => {
            let positive = infer_condition_value(&unary.operand, options);
            positive.invert()
        }

        // Handle name expressions (e.g., PY3, MYPY, TYPE_CHECKING)
        ast::Expr::Name(name) => check_name_truth_value(name.id.as_str(), options),

        // Handle attribute expressions (e.g., typing.TYPE_CHECKING, sys.platform)
        ast::Expr::Attribute(attr) => check_name_truth_value(attr.attr.as_str(), options),

        // Handle boolean operations (and/or)
        ast::Expr::BoolOp(bool_op) => {
            // Infer truth values for all operands
            let mut inferred_values = Vec::with_capacity(bool_op.values.len());
            for value in &bool_op.values {
                inferred_values.push(infer_condition_value(value, options));
            }

            combine_bool_op(bool_op.op, &inferred_values)
        }

        // Fallback: try sys.version_info and sys.platform checks
        _ => {
            let result = consider_sys_version_info(expr, options);
            if result != TruthValue::TruthValueUnknown {
                return result;
            }
            consider_sys_platform(expr, options)
        }
    }
}

pub(crate) fn infer_pattern_value(pattern: &ast::Pattern) -> TruthValue {
    match pattern {
        ast::Pattern::MatchAs(pattern) if pattern.pattern.is_none() => TruthValue::AlwaysTrue,
        ast::Pattern::MatchOr(pattern)
            if pattern
                .patterns
                .iter()
                .any(|p| infer_pattern_value(p) == TruthValue::AlwaysTrue) =>
        {
            TruthValue::AlwaysTrue
        }
        _ => TruthValue::TruthValueUnknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enum_values() {
        // Verify the enum values match the Python constants
        assert_eq!(TruthValue::AlwaysTrue as u8, 1);
        assert_eq!(TruthValue::MypyTrue as u8, 2);
        assert_eq!(TruthValue::AlwaysFalse as u8, 3);
        assert_eq!(TruthValue::MypyFalse as u8, 4);
        assert_eq!(TruthValue::TruthValueUnknown as u8, 5);
    }

    #[test]
    fn test_invert() {
        assert_eq!(TruthValue::AlwaysTrue.invert(), TruthValue::AlwaysFalse);
        assert_eq!(TruthValue::AlwaysFalse.invert(), TruthValue::AlwaysTrue);
        assert_eq!(TruthValue::MypyTrue.invert(), TruthValue::MypyFalse);
        assert_eq!(TruthValue::MypyFalse.invert(), TruthValue::MypyTrue);
        assert_eq!(
            TruthValue::TruthValueUnknown.invert(),
            TruthValue::TruthValueUnknown
        );
    }

    /// Helper to parse an expression and infer its truth value
    fn infer_expr(expr_str: &str) -> TruthValue {
        use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};

        let parsed = parse_unchecked(expr_str, ParseOptions::from(Mode::Expression));
        let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
            panic!("Expected expression");
        };

        let options = Options::new((3, 10), String::from("linux"), Vec::new(), Vec::new());
        infer_condition_value(&expr_mod.body, &options)
    }

    #[test]
    fn test_infer_condition_value_placeholder() {
        assert_eq!(infer_expr("foo"), TruthValue::TruthValueUnknown);
    }

    #[test]
    fn test_always_true_always_false() {
        use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};

        let parse_and_infer = |code: &str, always_true: &[String], always_false: &[String]| {
            let parsed = parse_unchecked(code, ParseOptions::from(Mode::Expression));
            let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
                panic!("Expected expression");
            };
            let options = Options::new(
                (3, 10),
                String::from("linux"),
                always_true.to_vec(),
                always_false.to_vec(),
            );
            infer_condition_value(&expr_mod.body, &options)
        };

        // Name in always_true list
        assert_eq!(
            parse_and_infer("DEBUG", &[String::from("DEBUG")], &[]),
            TruthValue::AlwaysTrue
        );

        // Name in always_false list
        assert_eq!(
            parse_and_infer("PRODUCTION", &[], &[String::from("PRODUCTION")]),
            TruthValue::AlwaysFalse
        );

        // Name not in either list
        assert_eq!(
            parse_and_infer(
                "UNKNOWN",
                &[String::from("DEBUG")],
                &[String::from("PRODUCTION")]
            ),
            TruthValue::TruthValueUnknown
        );

        // Built-in names take precedence over always_true/always_false
        assert_eq!(
            parse_and_infer("PY3", &[], &[String::from("PY3")]),
            TruthValue::AlwaysTrue
        );

        // Attribute expressions also check the lists
        assert_eq!(
            parse_and_infer("foo.DEBUG", &[String::from("DEBUG")], &[]),
            TruthValue::AlwaysTrue
        );
    }

    #[test]
    fn test_mypy_and_type_checking() {
        assert_eq!(infer_expr("MYPY"), TruthValue::MypyTrue);
        assert_eq!(infer_expr("TYPE_CHECKING"), TruthValue::MypyTrue);
    }

    #[test]
    fn test_py2_and_py3() {
        assert_eq!(infer_expr("PY2"), TruthValue::AlwaysFalse);
        assert_eq!(infer_expr("PY3"), TruthValue::AlwaysTrue);
    }

    #[test]
    fn test_unary_not() {
        assert_eq!(infer_expr("not MYPY"), TruthValue::MypyFalse);
        assert_eq!(infer_expr("not TYPE_CHECKING"), TruthValue::MypyFalse);
        assert_eq!(infer_expr("not PY2"), TruthValue::AlwaysTrue);
        assert_eq!(infer_expr("not PY3"), TruthValue::AlwaysFalse);
        assert_eq!(infer_expr("not foo"), TruthValue::TruthValueUnknown);
    }

    #[test]
    fn test_attribute_expressions() {
        assert_eq!(infer_expr("typing.TYPE_CHECKING"), TruthValue::MypyTrue);
        assert_eq!(infer_expr("foo.MYPY"), TruthValue::MypyTrue);
        assert_eq!(infer_expr("bar.PY2"), TruthValue::AlwaysFalse);
        assert_eq!(infer_expr("baz.PY3"), TruthValue::AlwaysTrue);
        assert_eq!(infer_expr("sys.platform"), TruthValue::TruthValueUnknown);
    }

    #[test]
    fn test_or_operation() {
        // ALWAYS_TRUE wins
        assert_eq!(infer_expr("PY3 or foo"), TruthValue::AlwaysTrue);
        assert_eq!(infer_expr("foo or PY3"), TruthValue::AlwaysTrue);

        // MYPY_TRUE wins if no ALWAYS_TRUE
        assert_eq!(infer_expr("MYPY or PY2"), TruthValue::MypyTrue);
        assert_eq!(infer_expr("PY2 or MYPY"), TruthValue::MypyTrue);

        // All MYPY_FALSE -> MYPY_FALSE
        assert_eq!(
            infer_expr("(not MYPY) or (not TYPE_CHECKING)"),
            TruthValue::MypyFalse
        );

        // All ALWAYS_FALSE or MYPY_FALSE -> ALWAYS_FALSE
        assert_eq!(infer_expr("PY2 or (not MYPY)"), TruthValue::AlwaysFalse);

        // Mixed with unknown -> unknown
        assert_eq!(infer_expr("foo or bar"), TruthValue::TruthValueUnknown);
    }

    #[test]
    fn test_and_operation() {
        // ALWAYS_FALSE wins
        assert_eq!(infer_expr("PY2 and foo"), TruthValue::AlwaysFalse);
        assert_eq!(infer_expr("foo and PY2"), TruthValue::AlwaysFalse);

        // MYPY_FALSE wins if no ALWAYS_FALSE
        assert_eq!(infer_expr("(not MYPY) and PY3"), TruthValue::MypyFalse);
        assert_eq!(infer_expr("PY3 and (not MYPY)"), TruthValue::MypyFalse);

        // All ALWAYS_TRUE -> ALWAYS_TRUE
        assert_eq!(infer_expr("PY3 and PY3"), TruthValue::AlwaysTrue);

        // All ALWAYS_TRUE or MYPY_TRUE -> MYPY_TRUE
        assert_eq!(infer_expr("PY3 and MYPY"), TruthValue::MypyTrue);
        assert_eq!(infer_expr("MYPY and TYPE_CHECKING"), TruthValue::MypyTrue);

        // Mixed with unknown -> unknown
        assert_eq!(infer_expr("foo and bar"), TruthValue::TruthValueUnknown);
    }

    #[test]
    fn test_is_sys_attr() {
        use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};

        let parse_expr = |code: &str| {
            let parsed = parse_unchecked(code, ParseOptions::from(Mode::Expression));
            let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
                panic!("Expected expression");
            };
            expr_mod.body
        };

        // Positive cases: sys.platform and sys.version_info
        assert!(is_sys_attr(&parse_expr("sys.platform"), "platform"));
        assert!(is_sys_attr(&parse_expr("sys.version_info"), "version_info"));

        // Wrong attribute name
        assert!(!is_sys_attr(&parse_expr("sys.platform"), "version_info"));
        assert!(!is_sys_attr(&parse_expr("sys.version_info"), "platform"));

        // Not sys module
        assert!(!is_sys_attr(&parse_expr("foo.platform"), "platform"));
        assert!(!is_sys_attr(&parse_expr("os.platform"), "platform"));

        // Not an attribute expression
        assert!(!is_sys_attr(&parse_expr("platform"), "platform"));
        assert!(!is_sys_attr(&parse_expr("sys"), "sys"));
    }

    #[test]
    fn test_consider_sys_version_info() {
        use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
        let options = Options::new((3, 10), String::from("linux"), Vec::new(), Vec::new());

        let parse_expr = |code: &str| {
            let parsed = parse_unchecked(code, ParseOptions::from(Mode::Expression));
            let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
                panic!("Expected expression");
            };
            expr_mod.body
        };

        // Edge case: exact equality
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[0] == 3"), &options),
            TruthValue::AlwaysTrue
        );

        // Edge case: one off from target
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[0] == 2"), &options),
            TruthValue::AlwaysFalse
        );

        // Edge case: >= with exact boundary
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[1] >= 10"), &options),
            TruthValue::AlwaysTrue
        );

        // Edge case: < with exact boundary (should be false)
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[0] < 3"), &options),
            TruthValue::AlwaysFalse
        );

        // Edge case: reversed operands with exact equality
        assert_eq!(
            consider_sys_version_info(&parse_expr("3 == sys.version_info[0]"), &options),
            TruthValue::AlwaysTrue
        );

        // Edge case: reversed > with one above boundary
        assert_eq!(
            consider_sys_version_info(&parse_expr("11 > sys.version_info[1]"), &options),
            TruthValue::AlwaysTrue
        );

        // Out of range index (only 0 and 1 supported)
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[2] == 5"), &options),
            TruthValue::TruthValueUnknown
        );

        // Not a comparison
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[0]"), &options),
            TruthValue::TruthValueUnknown
        );

        // Chained comparison
        assert_eq!(
            consider_sys_version_info(&parse_expr("2 < sys.version_info[0] < 4"), &options),
            TruthValue::TruthValueUnknown
        );

        // Tuple comparisons: sys.version_info >= (3, 8)
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info >= (3, 8)"), &options),
            TruthValue::AlwaysTrue
        );

        // Tuple comparisons: sys.version_info >= (3, 10) - exact boundary
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info >= (3, 10)"), &options),
            TruthValue::AlwaysTrue
        );

        // Tuple comparisons: sys.version_info < (3, 10) - exact boundary
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info < (3, 10)"), &options),
            TruthValue::AlwaysFalse
        );

        // Tuple comparisons: sys.version_info[:2] >= (3, 8)
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[:2] >= (3, 8)"), &options),
            TruthValue::AlwaysTrue
        );

        // Tuple comparisons: sys.version_info[0:2] >= (3, 10)
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[0:2] >= (3, 10)"), &options),
            TruthValue::AlwaysTrue
        );

        // Tuple comparisons: reversed operands
        assert_eq!(
            consider_sys_version_info(&parse_expr("(3, 8) <= sys.version_info"), &options),
            TruthValue::AlwaysTrue
        );

        // Tuple comparisons: single element tuple with slice
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info[:1] >= (3,)"), &options),
            TruthValue::AlwaysTrue
        );

        // Tuple comparisons: version longer than target (allowed for ordering)
        assert_eq!(
            consider_sys_version_info(&parse_expr("sys.version_info >= (3,)"), &options),
            TruthValue::AlwaysTrue
        );
    }

    #[test]
    fn test_contains_int_or_tuple_of_ints() {
        use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};

        let parse_expr = |code: &str| {
            let parsed = parse_unchecked(code, ParseOptions::from(Mode::Expression));
            let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
                panic!("Expected expression");
            };
            expr_mod.body
        };

        // Single integer
        assert_eq!(
            contains_int_or_tuple_of_ints(&parse_expr("42")),
            Some(IntOrTuple::Int(42))
        );
        assert_eq!(
            contains_int_or_tuple_of_ints(&parse_expr("0")),
            Some(IntOrTuple::Int(0))
        );

        // Tuple of integers
        assert_eq!(
            contains_int_or_tuple_of_ints(&parse_expr("(3, 10)")),
            Some(IntOrTuple::Tuple(vec![3, 10]))
        );
        assert_eq!(
            contains_int_or_tuple_of_ints(&parse_expr("(3, 8, 2)")),
            Some(IntOrTuple::Tuple(vec![3, 8, 2]))
        );

        // Empty tuple
        assert_eq!(
            contains_int_or_tuple_of_ints(&parse_expr("()")),
            Some(IntOrTuple::Tuple(vec![]))
        );

        // Single element tuple
        assert_eq!(
            contains_int_or_tuple_of_ints(&parse_expr("(5,)")),
            Some(IntOrTuple::Tuple(vec![5]))
        );

        // Not an integer or tuple of integers
        assert_eq!(contains_int_or_tuple_of_ints(&parse_expr("'hello'")), None);
        assert_eq!(contains_int_or_tuple_of_ints(&parse_expr("3.14")), None);
        assert_eq!(contains_int_or_tuple_of_ints(&parse_expr("(1, 'a')")), None);
        assert_eq!(contains_int_or_tuple_of_ints(&parse_expr("(1, 2.5)")), None);
    }

    #[test]
    fn test_contains_sys_version_info() {
        use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};

        let parse_expr = |code: &str| {
            let parsed = parse_unchecked(code, ParseOptions::from(Mode::Expression));
            let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
                panic!("Expected expression");
            };
            expr_mod.body
        };

        // Bare sys.version_info
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info")),
            Some(SysVersionInfo::Whole)
        );

        // Single index access
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[0]")),
            Some(SysVersionInfo::Index(0))
        );
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[1]")),
            Some(SysVersionInfo::Index(1))
        );

        // Slice with both bounds
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[0:2]")),
            Some(SysVersionInfo::Slice(Some(0), Some(2)))
        );

        // Slice with only lower bound
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[1:]")),
            Some(SysVersionInfo::Slice(Some(1), None))
        );

        // Slice with only upper bound
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[:2]")),
            Some(SysVersionInfo::Slice(None, Some(2)))
        );

        // Slice with no bounds (equivalent to [:])
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[:]")),
            Some(SysVersionInfo::Slice(None, None))
        );

        // Slice with stride 1 (allowed)
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[0:2:1]")),
            Some(SysVersionInfo::Slice(Some(0), Some(2)))
        );

        // Slice with stride != 1 (not allowed)
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.version_info[0:2:2]")),
            None
        );

        // Not sys.version_info
        assert_eq!(
            contains_sys_version_info(&parse_expr("foo.version_info[0]")),
            None
        );
        assert_eq!(
            contains_sys_version_info(&parse_expr("sys.platform[0]")),
            None
        );

        // Not a subscript or attribute
        assert_eq!(contains_sys_version_info(&parse_expr("version_info")), None);
    }

    #[test]
    fn test_consider_sys_platform() {
        use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
        let linux_options = Options::new((3, 10), String::from("linux"), Vec::new(), Vec::new());
        let win32_options = Options::new((3, 10), String::from("win32"), Vec::new(), Vec::new());

        let parse_expr = |code: &str| {
            let parsed = parse_unchecked(code, ParseOptions::from(Mode::Expression));
            let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
                panic!("Expected expression");
            };
            expr_mod.body
        };

        // sys.platform == "linux" on linux platform
        assert_eq!(
            consider_sys_platform(&parse_expr("sys.platform == 'linux'"), &linux_options),
            TruthValue::AlwaysTrue
        );

        // sys.platform == "win32" on linux platform
        assert_eq!(
            consider_sys_platform(&parse_expr("sys.platform == 'win32'"), &linux_options),
            TruthValue::AlwaysFalse
        );

        // sys.platform != "win32" on linux platform
        assert_eq!(
            consider_sys_platform(&parse_expr("sys.platform != 'win32'"), &linux_options),
            TruthValue::AlwaysTrue
        );

        // sys.platform != "linux" on linux platform
        assert_eq!(
            consider_sys_platform(&parse_expr("sys.platform != 'linux'"), &linux_options),
            TruthValue::AlwaysFalse
        );

        // Unsupported: other comparison operators
        assert_eq!(
            consider_sys_platform(&parse_expr("sys.platform < 'linux'"), &linux_options),
            TruthValue::TruthValueUnknown
        );

        // Unsupported: not sys.platform
        assert_eq!(
            consider_sys_platform(&parse_expr("foo.platform == 'linux'"), &linux_options),
            TruthValue::TruthValueUnknown
        );

        // Unsupported: chained comparisons
        assert_eq!(
            consider_sys_platform(&parse_expr("'a' < sys.platform < 'z'"), &linux_options),
            TruthValue::TruthValueUnknown
        );

        // startswith: platform starts with prefix
        assert_eq!(
            consider_sys_platform(
                &parse_expr("sys.platform.startswith('lin')"),
                &linux_options
            ),
            TruthValue::AlwaysTrue
        );

        // startswith: platform doesn't start with prefix
        assert_eq!(
            consider_sys_platform(
                &parse_expr("sys.platform.startswith('win')"),
                &linux_options
            ),
            TruthValue::AlwaysFalse
        );

        // startswith: exact match
        assert_eq!(
            consider_sys_platform(
                &parse_expr("sys.platform.startswith('linux')"),
                &linux_options
            ),
            TruthValue::AlwaysTrue
        );

        // startswith on win32 platform
        assert_eq!(
            consider_sys_platform(
                &parse_expr("sys.platform.startswith('win')"),
                &win32_options
            ),
            TruthValue::AlwaysTrue
        );
    }

    #[test]
    fn test_fixed_comparison() {
        // Equality with strings
        assert_eq!(
            fixed_comparison("linux", ast::CmpOp::Eq, "linux"),
            TruthValue::AlwaysTrue
        );
        assert_eq!(
            fixed_comparison("linux", ast::CmpOp::Eq, "win32"),
            TruthValue::AlwaysFalse
        );

        // Inequality with strings
        assert_eq!(
            fixed_comparison("linux", ast::CmpOp::NotEq, "win32"),
            TruthValue::AlwaysTrue
        );
        assert_eq!(
            fixed_comparison("linux", ast::CmpOp::NotEq, "linux"),
            TruthValue::AlwaysFalse
        );

        // Less than with integers
        assert_eq!(
            fixed_comparison(3, ast::CmpOp::Lt, 10),
            TruthValue::AlwaysTrue
        );
        assert_eq!(
            fixed_comparison(10, ast::CmpOp::Lt, 3),
            TruthValue::AlwaysFalse
        );
        assert_eq!(
            fixed_comparison(5, ast::CmpOp::Lt, 5),
            TruthValue::AlwaysFalse
        );

        // Less than or equal with integers
        assert_eq!(
            fixed_comparison(3, ast::CmpOp::LtE, 10),
            TruthValue::AlwaysTrue
        );
        assert_eq!(
            fixed_comparison(5, ast::CmpOp::LtE, 5),
            TruthValue::AlwaysTrue
        );
        assert_eq!(
            fixed_comparison(10, ast::CmpOp::LtE, 3),
            TruthValue::AlwaysFalse
        );

        // Greater than with strings
        assert_eq!(
            fixed_comparison("b", ast::CmpOp::Gt, "a"),
            TruthValue::AlwaysTrue
        );
        assert_eq!(
            fixed_comparison("a", ast::CmpOp::Gt, "b"),
            TruthValue::AlwaysFalse
        );

        // Greater than or equal with integers
        assert_eq!(
            fixed_comparison(10, ast::CmpOp::GtE, 3),
            TruthValue::AlwaysTrue
        );
        assert_eq!(
            fixed_comparison(5, ast::CmpOp::GtE, 5),
            TruthValue::AlwaysTrue
        );

        // Unsupported operators return unknown
        assert_eq!(
            fixed_comparison("a", ast::CmpOp::In, "b"),
            TruthValue::TruthValueUnknown
        );
        assert_eq!(
            fixed_comparison(1, ast::CmpOp::Is, 2),
            TruthValue::TruthValueUnknown
        );
    }
}
