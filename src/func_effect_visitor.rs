//! Visitor to detect if a function body has externally visible effects.
//!
//! This is used to determine if a function body can be safely omitted during serialization
//! when errors are ignored in a file. A function body must be preserved if it has effects
//! that are externally visible, such as:
//! - Defining attributes on its first parameter (e.g., `self.x = 1`)
//! - Containing yield expressions that affect the return type

use ruff_python_ast::{self as ast, visitor::Visitor};

/// Check if a function body is "trivial" (e.g., pass, ..., raise, or just docstring).
///
/// Trivial bodies may be needed for type checking as they have externally visible impact.
/// This matches the behavior of mypy's trivial body detection.
///
/// A body is considered trivial if it contains:
/// - Just a docstring (string literal as a standalone statement)
/// - A docstring followed by pass
/// - A docstring followed by raise
/// - A docstring followed by `...` (ellipsis literal)
///
/// # Examples
///
/// ```python
/// # Trivial:
/// def foo():
///     pass
///
/// def bar():
///     """Docstring"""
///
/// def baz():
///     """Docstring"""
///     pass
///
/// def qux():
///     raise NotImplementedError
///
/// # Not trivial:
/// def compute():
///     return 42
/// ```
pub(crate) fn is_trivial_body(body: &[ast::Stmt]) -> bool {
    if body.is_empty() {
        return false;
    }

    let mut i = 0;

    // Skip docstring if present (first statement is a string literal expression)
    if let Some(ast::Stmt::Expr(expr_stmt)) = body.first() {
        if matches!(&*expr_stmt.value, ast::Expr::StringLiteral(_)) {
            i += 1;
        }
    }

    // If only a docstring, it's trivial
    if i == body.len() {
        return true;
    }

    // If more than one non-docstring statement, it's not trivial
    if body.len() > i + 1 {
        return false;
    }

    // Check if the single non-docstring statement is pass, raise, or ellipsis
    let stmt = &body[i];
    match stmt {
        ast::Stmt::Pass(_) | ast::Stmt::Raise(_) => true,
        ast::Stmt::Expr(expr_stmt) => {
            // Check if it's an ellipsis literal expression
            matches!(&*expr_stmt.value, ast::Expr::EllipsisLiteral(_))
        }
        _ => false,
    }
}

/// Check if a function body has externally visible effects.
///
/// Returns `true` if the function body contains:
/// - Any assignments to attributes of the first parameter (if `check_attributes` is true):
///   - Direct assignments: `self.x = 1`
///   - Augmented assignments: `self.x += 1`
///   - Annotated assignments: `self.x: int = 1`
///   - Tuple/list unpacking: `self.x, self.y = f()`
///   - Starred expressions: `*self.x = range(10)`
///   - Nested tuple/list unpacking: `(self.x, (self.y, self.z)) = f()`
/// - Yield expressions: `yield x` or `yield from iterable`
///   (these affect the inferred return type, making the function a generator)
///
/// Returns `false` if no externally visible effects are found.
///
/// # Arguments
///
/// * `body` - The function body statements to analyze
/// * `parameters` - The function parameters
/// * `check_attributes` - Whether to check for attribute assignments (true for methods, false for top-level functions)
///
/// # Examples
///
/// ```python
/// # Returns true for (when check_attributes=true):
/// def foo(self):
///     self.x = 1
///
/// # Returns true for (regardless of check_attributes):
/// def bar():
///     yield 1
///
/// # Returns false for:
/// def baz(self):
///     local_var = 1
/// ```
pub(crate) fn has_externally_visible_effect(
    body: &[ast::Stmt],
    parameters: &ast::Parameters,
    check_attributes: bool,
) -> bool {
    // Trivial bodies always have externally visible effects (needed for type checking)
    if is_trivial_body(body) {
        return true;
    }

    // Get the name of the first parameter (if any)
    let first_param_name = if check_attributes {
        parameters
            .posonlyargs
            .first()
            .or(parameters.args.first())
            .map(|p| p.parameter.name.as_str())
            .unwrap_or("")
    } else {
        "" // Don't check attributes if not requested
    };

    let mut visitor = EffectDetector {
        first_param_name,
        defines_attributes: false,
        contains_yield: false,
    };

    visitor.visit_body(body);
    visitor.defines_attributes || visitor.contains_yield
}

/// Visitor that detects externally visible effects in function bodies.
struct EffectDetector<'a> {
    /// Name of the first parameter to check (e.g., "self")
    first_param_name: &'a str,
    /// Whether we've found an attribute assignment
    defines_attributes: bool,
    /// Whether we've found a yield expression
    contains_yield: bool,
}

impl<'a> Visitor<'a> for EffectDetector<'a> {
    fn visit_stmt(&mut self, stmt: &'a ast::Stmt) {
        // Early exit if we already found both an attribute definition and yield
        if self.defines_attributes && self.contains_yield {
            return;
        }

        match stmt {
            // Don't recurse into nested functions/classes - they have their own scope
            ast::Stmt::FunctionDef(_) | ast::Stmt::ClassDef(_) => {
                // Simply skip nested definitions entirely
                return;
            }
            // Check various assignment forms
            ast::Stmt::Assign(assign) => {
                // Check all targets (can have multiple in `a = b = 1`)
                for target in &assign.targets {
                    if self.contains_param_attribute(target) {
                        self.defines_attributes = true;
                        return;
                    }
                }
            }
            ast::Stmt::AugAssign(aug_assign) => {
                if self.contains_param_attribute(&aug_assign.target) {
                    self.defines_attributes = true;
                    return;
                }
            }
            ast::Stmt::AnnAssign(ann_assign) => {
                if self.contains_param_attribute(&ann_assign.target) {
                    self.defines_attributes = true;
                    return;
                }
            }
            ast::Stmt::With(with_stmt) => {
                // Check `with expr as target:` - target can be self.x
                for item in &with_stmt.items {
                    if let Some(optional_vars) = &item.optional_vars {
                        if self.contains_param_attribute(optional_vars) {
                            self.defines_attributes = true;
                            return;
                        }
                    }
                }
            }
            ast::Stmt::For(for_stmt) => {
                // Check `for target in iter:` - target can be self.x
                if self.contains_param_attribute(&for_stmt.target) {
                    self.defines_attributes = true;
                    return;
                }
            }
            _ => {}
        }

        // Continue walking the AST
        ast::visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a ast::Expr) {
        // Early exit if we already found both an attribute definition and yield
        if self.defines_attributes && self.contains_yield {
            return;
        }

        // Check for yield expressions
        match expr {
            ast::Expr::Yield(_) | ast::Expr::YieldFrom(_) => {
                self.contains_yield = true;
                // Don't need to walk further into yield expression
                return;
            }
            // Don't recurse into nested functions/lambdas - they have their own scope
            ast::Expr::Lambda(_) => {
                // Skip lambda bodies - yield in a lambda would be a syntax error anyway,
                // but we skip for consistency with nested function handling
                return;
            }
            _ => {}
        }

        // Continue walking the AST
        ast::visitor::walk_expr(self, expr);
    }
}

impl<'a> EffectDetector<'a> {
    /// Check if an expression contains an attribute access on the first parameter.
    ///
    /// This handles:
    /// - Direct attribute: `self.x`
    /// - Tuple unpacking: `(self.x, y)`, `self.x, y`
    /// - List unpacking: `[self.x, y]`
    /// - Starred expressions: `*self.x`
    /// - Nested structures: `(self.x, (self.y, z))`
    fn contains_param_attribute(&self, expr: &ast::Expr) -> bool {
        match expr {
            ast::Expr::Attribute(attr) => {
                // Check if this is `param_name.something`
                matches!(&*attr.value, ast::Expr::Name(name)
                    if name.id.as_str() == self.first_param_name)
            }
            ast::Expr::Tuple(tuple) => {
                // Check all elements in tuple: `self.x, self.y = f()`
                tuple
                    .elts
                    .iter()
                    .any(|elt| self.contains_param_attribute(elt))
            }
            ast::Expr::List(list) => {
                // Check all elements in list: `[self.x, self.y] = f()`
                list.elts
                    .iter()
                    .any(|elt| self.contains_param_attribute(elt))
            }
            ast::Expr::Starred(starred) => {
                // Check starred expression: `*self.x = range(10)`
                self.contains_param_attribute(&starred.value)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_python_ast::PySourceType;
    use ruff_python_parser::{ParseOptions, parse_unchecked};

    /// Helper to parse a function and check if it has externally visible effects
    /// Assumes we're checking a method (check_attributes = true)
    fn check_function(code: &str) -> bool {
        let parsed = parse_unchecked(code, ParseOptions::from(PySourceType::Python));
        let ast::Mod::Module(module) = parsed.into_syntax() else {
            panic!("Expected module");
        };

        // Get the first function definition
        for stmt in &module.body {
            if let ast::Stmt::FunctionDef(func) = stmt {
                return has_externally_visible_effect(&func.body, &func.parameters, true);
            }
        }
        panic!("No function found in code");
    }

    /// Helper to check top-level function (check_attributes = false)
    fn check_toplevel_function(code: &str) -> bool {
        let parsed = parse_unchecked(code, ParseOptions::from(PySourceType::Python));
        let ast::Mod::Module(module) = parsed.into_syntax() else {
            panic!("Expected module");
        };

        // Get the first function definition
        for stmt in &module.body {
            if let ast::Stmt::FunctionDef(func) = stmt {
                return has_externally_visible_effect(&func.body, &func.parameters, false);
            }
        }
        panic!("No function found in code");
    }

    #[test]
    fn test_simple_attribute_assignment() {
        let code = r#"
def foo(self):
    self.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_no_attribute_assignment() {
        let code = r#"
def foo(self):
    local_var = 1
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_no_parameters() {
        let code = r#"
def foo():
    x = 1
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_augmented_assignment() {
        let code = r#"
def foo(self):
    self.x += 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_annotated_assignment() {
        let code = r#"
def foo(self):
    self.x: int = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_tuple_unpacking() {
        let code = r#"
def foo(self):
    self.x, y = (1, 2)
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_tuple_unpacking_second_element() {
        let code = r#"
def foo(self):
    x, self.y = (1, 2)
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_tuple_unpacking_no_self() {
        let code = r#"
def foo(self):
    x, y = (1, 2)
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_list_unpacking() {
        let code = r#"
def foo(self):
    [self.x, y] = [1, 2]
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_nested_tuple_unpacking() {
        let code = r#"
def foo(self):
    (self.x, (y, z)) = (1, (2, 3))
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_nested_list_unpacking() {
        let code = r#"
def foo(self):
    [a, [self.y, z]] = [1, [2, 3]]
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_deeply_nested_unpacking() {
        let code = r#"
def foo(self):
    (a, (b, (self.x, c))) = (1, (2, (3, 4)))
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_starred_expression() {
        let code = r#"
def foo(self):
    *self.x, = range(10)
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_starred_in_tuple() {
        let code = r#"
def foo(self):
    a, *self.rest = [1, 2, 3, 4]
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_nested_function_no_leak() {
        // Attribute assignment in nested function should not count
        let code = r#"
def foo(self):
    def inner(self):
        self.x = 1
    return inner
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_nested_class_no_leak() {
        // Attribute assignment in nested class should not count
        let code = r#"
def foo(self):
    class Inner:
        def method(self):
            self.x = 1
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_attribute_in_if_statement() {
        let code = r#"
def foo(self):
    if True:
        self.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_in_while_loop() {
        let code = r#"
def foo(self):
    while True:
        self.x = 1
        break
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_in_for_loop() {
        let code = r#"
def foo(self):
    for i in range(10):
        self.x = i
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_in_try_except() {
        let code = r#"
def foo(self):
    try:
        self.x = 1
    except:
        pass
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_in_except_handler() {
        let code = r#"
def foo(self):
    try:
        pass
    except Exception:
        self.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_in_finally() {
        let code = r#"
def foo(self):
    try:
        pass
    finally:
        self.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_in_with_statement() {
        let code = r#"
def foo(self):
    with open("file") as f:
        self.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_assignment_after_nested_function() {
        let code = r#"
def foo(self):
    def inner():
        pass
    self.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_multiple_assignment_targets() {
        let code = r#"
def foo(self):
    self.x = y = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_different_param_name() {
        let code = r#"
def foo(obj):
    obj.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_attribute_of_different_object() {
        let code = r#"
def foo(self):
    other.x = 1
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_posonly_param() {
        let code = r#"
def foo(self, /):
    self.x = 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_posonly_param_no_assignment() {
        let code = r#"
def foo(self, /):
    y = 1
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_empty_body() {
        // A body with just `pass` is now considered trivial (externally visible)
        let code = r#"
def foo(self):
    pass
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_only_docstring() {
        // A body with just docstring and pass is now considered trivial (externally visible)
        let code = r#"
def foo(self):
    """Docstring"""
    pass
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_mixed_tuple_and_list() {
        let code = r#"
def foo(self):
    (a, [self.x, b]) = (1, [2, 3])
"#;
        assert!(check_function(code));
    }

    // Yield expression tests

    #[test]
    fn test_simple_yield() {
        let code = r#"
def foo(self):
    yield 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_from() {
        let code = r#"
def foo(self):
    yield from range(10)
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_in_loop() {
        let code = r#"
def foo(self):
    for i in range(10):
        yield i
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_in_if() {
        let code = r#"
def foo(self):
    if True:
        yield 1
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_in_expression() {
        let code = r#"
def foo(self):
    x = (yield 1)
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_with_no_value() {
        let code = r#"
def foo(self):
    yield
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_in_comprehension() {
        // Yield in generator expression
        let code = r#"
def foo(self):
    return [(yield i) for i in range(10)]
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_with_attribute_assignment() {
        // Both yield and attribute assignment
        let code = r#"
def foo(self):
    self.x = 1
    yield 2
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_in_nested_function_no_leak() {
        // Yield in nested function should not count
        let code = r#"
def foo(self):
    def inner():
        yield 1
    return inner
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_no_yield_no_attributes() {
        let code = r#"
def foo(self):
    x = 1
    return x
"#;
        assert!(!check_function(code));
    }

    #[test]
    fn test_yield_in_try_except() {
        let code = r#"
def foo(self):
    try:
        yield 1
    except:
        pass
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_in_with() {
        let code = r#"
def foo(self):
    with open("file") as f:
        yield f.read()
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_yield_no_params() {
        // Function with no parameters but with yield should still preserve body
        let code = r#"
def foo():
    yield 1
"#;
        assert!(check_function(code));
    }

    // Tests for top-level functions (check_attributes = false)

    #[test]
    fn test_toplevel_with_yield() {
        let code = r#"
def foo():
    yield 1
"#;
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_toplevel_with_attribute_assignment() {
        // Top-level function with attribute assignment should NOT count
        // (since check_attributes = false)
        let code = r#"
def foo(self):
    self.x = 1
"#;
        assert!(!check_toplevel_function(code));
    }

    #[test]
    fn test_toplevel_plain_function() {
        let code = r#"
def foo(x):
    return x + 1
"#;
        assert!(!check_toplevel_function(code));
    }

    #[test]
    fn test_toplevel_with_yield_in_loop() {
        let code = r#"
def foo(items):
    for item in items:
        yield item
"#;
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_with_statement_target() {
        let code = r#"
def foo(self):
    with y as self.x:
        pass
"#;
        assert!(check_function(code));
    }

    #[test]
    fn test_for_statement_target() {
        let code = r#"
def foo(self):
    for self.x in items:
        pass
"#;
        assert!(check_function(code));
    }

    // Tests for trivial body detection

    #[test]
    fn test_trivial_pass() {
        let code = r#"
def foo():
    pass
"#;
        assert!(check_function(code));
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_trivial_ellipsis() {
        let code = r#"
def foo():
    ...
"#;
        assert!(check_function(code));
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_trivial_raise() {
        let code = r#"
def foo():
    raise NotImplementedError
"#;
        assert!(check_function(code));
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_trivial_docstring_only() {
        let code = r#"
def foo():
    """Docstring"""
"#;
        assert!(check_function(code));
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_trivial_docstring_and_pass() {
        let code = r#"
def foo():
    """Docstring"""
    pass
"#;
        assert!(check_function(code));
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_trivial_docstring_and_ellipsis() {
        let code = r#"
def foo():
    """Docstring"""
    ...
"#;
        assert!(check_function(code));
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_trivial_docstring_and_raise() {
        let code = r#"
def foo():
    """Docstring"""
    raise NotImplementedError
"#;
        assert!(check_function(code));
        assert!(check_toplevel_function(code));
    }

    #[test]
    fn test_not_trivial_return() {
        let code = r#"
def foo():
    return 42
"#;
        assert!(!check_toplevel_function(code));
    }

    #[test]
    fn test_not_trivial_assignment() {
        let code = r#"
def foo():
    x = 1
"#;
        assert!(!check_toplevel_function(code));
    }

    #[test]
    fn test_not_trivial_multiple_statements() {
        let code = r#"
def foo():
    """Docstring"""
    pass
    pass
"#;
        assert!(!check_toplevel_function(code));
    }

    #[test]
    fn test_not_trivial_docstring_and_return() {
        let code = r#"
def foo():
    """Docstring"""
    return 42
"#;
        assert!(!check_toplevel_function(code));
    }
}
