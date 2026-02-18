//! Serialize the AST for a given Python file as a mypy AST

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use ruff_python_ast::{self as ast};
use ruff_python_ast::{Number, PySourceType};
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::func_effect_visitor;
use crate::options::Options;
use crate::reachability::TruthValue;
use crate::type_comment;

/// Syntax error information with location details
#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

// Fixed tags for primitive types (must match mypy/cache.py)
const TAG_LITERAL_FALSE: u8 = 0;
const TAG_LITERAL_TRUE: u8 = 1;
const TAG_LITERAL_NONE: u8 = 2;
const TAG_LITERAL_INT: u8 = 3;
const TAG_LITERAL_STR: u8 = 4;
const TAG_LITERAL_BYTES: u8 = 5;
const TAG_LITERAL_FLOAT: u8 = 6;
const TAG_LITERAL_COMPLEX: u8 = 7;

// Fixed tags for collections (must match mypy/cache.py)
const TAG_LIST_GEN: u8 = 20;
const TAG_LIST_INT: u8 = 21;
const TAG_LIST_STR: u8 = 22;
const TAG_LIST_BYTES: u8 = 23;
const TAG_DICT_STR_GEN: u8 = 30;

const TAG_DECORATOR: u8 = 53;
const TAG_CLASS_DEF: u8 = 60;

// End tag for composite objects
const TAG_END: u8 = 255;

const TAG_LOCATION: u8 = 152;
const TAG_EXPR_STMT: u8 = 160;
const TAG_CALL_EXPR: u8 = 161;
const TAG_NAME_EXPR: u8 = 162;
const TAG_STR_EXPR: u8 = 163;
const TAG_IMPORT: u8 = 164;
const TAG_MEMBER_EXPR: u8 = 165;
const TAG_OP_EXPR: u8 = 166;
const TAG_INT_EXPR: u8 = 167;
const TAG_IF: u8 = 168;
const TAG_ASSIGN: u8 = 169;
const TAG_TUPLE_EXPR: u8 = 170;
const TAG_BLOCK: u8 = 171;
const TAG_INDEX: u8 = 172;
const TAG_LIST_EXPR: u8 = 173;
const TAG_SET_EXPR: u8 = 174;
const TAG_RETURN: u8 = 175;
const TAG_WHILE: u8 = 176;
const TAG_COMPARISON_EXPR: u8 = 177;
const TAG_BOOL_OP_EXPR: u8 = 178;
const TAG_FUNC_DEF: u8 = 179;
const TAG_PASS_STMT: u8 = 180;
const TAG_FLOAT_EXPR: u8 = 181;
const TAG_UNARY_EXPR: u8 = 182;
const TAG_DICT_EXPR: u8 = 183;
const TAG_COMPLEX_EXPR: u8 = 184;
const TAG_SLICE_EXPR: u8 = 185;
const TAG_TEMP_NODE: u8 = 186;
const TAG_RAISE_STMT: u8 = 187;
const TAG_BREAK_STMT: u8 = 188;
const TAG_CONTINUE_STMT: u8 = 189;
const TAG_GENERATOR_EXPR: u8 = 190;
const TAG_YIELD_EXPR: u8 = 191;
const TAG_YIELD_FROM_EXPR: u8 = 192;
const TAG_LIST_COMPREHENSION: u8 = 193;
const TAG_SET_COMPREHENSION: u8 = 194;
const TAG_DICT_COMPREHENSION: u8 = 195;
const TAG_IMPORT_FROM: u8 = 196;
const TAG_ASSERT_STMT: u8 = 197;
const TAG_FOR_STMT: u8 = 198;
const TAG_WITH_STMT: u8 = 199;
const TAG_OPERATOR_ASSIGNMENT_STMT: u8 = 200;
const TAG_TRY_STMT: u8 = 201;
const TAG_ELLIPSIS_EXPR: u8 = 202;
const TAG_CONDITIONAL_EXPR: u8 = 203;
const TAG_DEL_STMT: u8 = 204;
const TAG_FSTRING_EXPR: u8 = 205;
const TAG_FSTRING_INTERPOLATION: u8 = 206;
const TAG_LAMBDA_EXPR: u8 = 207;
const TAG_NAMED_EXPR: u8 = 208;
const TAG_STAR_EXPR: u8 = 209;
const TAG_BYTES_EXPR: u8 = 210;
const TAG_GLOBAL_DECL: u8 = 211;
const TAG_NONLOCAL_DECL: u8 = 212;
const TAG_AWAIT_EXPR: u8 = 213;
const TAG_BIG_INT_EXPR: u8 = 214;
const TAG_IMPORT_ALL: u8 = 215;
const TAG_MATCH_STMT: u8 = 216;
const TAG_AS_PATTERN: u8 = 217;
const TAG_OR_PATTERN: u8 = 218;
const TAG_VALUE_PATTERN: u8 = 219;
const TAG_SINGLETON_PATTERN: u8 = 220;
const TAG_SEQUENCE_PATTERN: u8 = 221;
const TAG_STARRED_PATTERN: u8 = 222;
const TAG_MAPPING_PATTERN: u8 = 223;
const TAG_CLASS_PATTERN: u8 = 224;
const TAG_TYPE_ALIAS_STMT: u8 = 225;
const TAG_IMPORT_METADATA: u8 = 226;
const TAG_IMPORTFROM_METADATA: u8 = 227;
const TAG_IMPORTALL_METADATA: u8 = 228;
const TAG_UNBOUND_TYPE: u8 = 104;
const TAG_TUPLE_TYPE: u8 = 112;
const TAG_UNION_TYPE: u8 = 115;
const TAG_LIST_TYPE: u8 = 118;
const TAG_ELLIPSIS_TYPE: u8 = 119;
const TAG_RAW_EXPRESSION_TYPE: u8 = 120;
const TAG_CALL_TYPE: u8 = 121;
const TAG_UNPACK_TYPE: u8 = 105;

// Argument kinds (must match mypy/nodes.py)
const ARG_POS: i64 = 0; // Positional argument
const ARG_OPT: i64 = 1; // Positional argument with default
const ARG_STAR: i64 = 2; // *args
const ARG_NAMED: i64 = 3; // Keyword-only argument
const ARG_STAR2: i64 = 4; // **kwargs
const ARG_NAMED_OPT: i64 = 5; // Keyword-only argument with default

// TypeParam kinds (must match mypy/nodes.py)
const TYPE_VAR_KIND: i64 = 0; // TypeVar
const PARAM_SPEC_KIND: i64 = 1; // ParamSpec
const TYPE_VAR_TUPLE_KIND: i64 = 2; // TypeVarTuple

const MIN_SHORT_INT: i64 = -10;
const MIN_TWO_BYTES_INT: i64 = -100;
const MAX_TWO_BYTES_INT: i64 = 16283; // 2 ** (8 + 6) - 1 - 100
const MIN_FOUR_BYTES_INT: i64 = -10000;
const MAX_FOUR_BYTES_INT: i64 = 536860911; // 2 ** (3 * 8 + 5) - 1 - 10000

const TWO_BYTES_INT_BIT: i64 = 1;
const FOUR_BYTES_INT_TRAILER: i64 = 3;
const LONG_INT_TRAILER: u8 = 15;

/// Serialize a Python file to mypy AST format.
///
/// # Arguments
///
/// * `file_path` - Path to the Python file to parse and serialize
/// * `skip_function_bodies` - If true, omit function bodies unless they have externally visible effects
///   (for methods in classes only; module-level functions always have bodies omitted when this is true)
/// * `options` - Reachability analysis options (python version, platform and always-true/-false names)
///
/// # Returns
///
/// A tuple containing:
/// - A `Vec<u8>` with the serialized AST in mypy's binary format (may be partial if there are syntax errors)
/// - A `Vec<SyntaxError>` containing any syntax errors with line/column information
/// - A `Vec<(usize, Vec<String>)>` containing tuples of (line_number, error_codes) for all type: ignore comments
///
/// # Errors
///
/// Returns an error if the file cannot be read (but not for syntax errors, which are returned in the tuple)
pub(crate) fn serialize_python_file(
    file_path: &Path,
    skip_function_bodies: bool,
    options: Options,
) -> Result<(Vec<u8>, Vec<SyntaxError>, Vec<(usize, Vec<String>)>, Vec<u8>, bool)> {
    let source_type = PySourceType::from(file_path);
    let source_text = std::fs::read_to_string(file_path)?;
    let line_index = LineIndex::from_source_text(&source_text);
    let is_stub_package = match file_path.file_name() {
        Some(file) => file.as_encoded_bytes() == b"__init__.pyi",
        _ => false,
    };

    // Check if file is all ASCII and build per-line non-ASCII flags if needed
    let is_all_ascii = source_text.is_ascii();
    let lines_with_non_ascii = if is_all_ascii {
        Vec::new() // No need to track per-line if whole file is ASCII
    } else {
        // Build a Vec<bool> indicating which lines have non-ASCII characters
        source_text.lines().map(|line| !line.is_ascii()).collect()
    };

    // Parse the file - this always returns a result, even with syntax errors
    let parsed = parse_unchecked(&source_text, ParseOptions::from(source_type));

    // Extract syntax errors with location information
    let syntax_errors: Vec<SyntaxError> = parsed
        .errors()
        .iter()
        .map(|error| {
            let location = line_index.line_column(error.location.start(), &source_text);
            SyntaxError {
                line: location.line.get(),
                column: location.column.get(),
                message: error.error.to_string(),
            }
        })
        .collect();

    // Extract both type: ignore comments and type annotation comments in a single pass
    let (type_ignore_lines, type_comments) =
        extract_type_comments_and_ignores(parsed.tokens(), &source_text, &line_index);

    // Serialize the AST (even if partial due to syntax errors)
    let mut ser = Serializer {
        bytes: Vec::new(),
        imports: Vec::new(),
        line_index,
        text: &source_text,
        skip_function_bodies,
        in_class: false,
        in_function: false,
        is_all_ascii,
        lines_with_non_ascii,
        type_comments,
        options,
        current_unreachable: false,
        current_mypy_only: false,
        top_level_getattr: false,
    };
    parsed.syntax().serialize(&mut ser);

    // Serialize the collected imports, reusing the moved state from serializer
    let import_bytes = serialize_imports(
        &ser.imports,
        &source_text,
        Some(ser.line_index),
        Some(is_all_ascii),
        Some(ser.lines_with_non_ascii),
    );

    // Return this directly to caller, so that it can check this without deserialization
    let is_partial_package = is_stub_package && ser.top_level_getattr;

    Ok((ser.bytes, syntax_errors, type_ignore_lines, import_bytes, is_partial_package))
}

// Bit flags for import statement metadata
const IMPORT_FLAG_TOP_LEVEL: u8 = 1 << 0;    // true if import is not within a function
const IMPORT_FLAG_UNREACHABLE: u8 = 1 << 1;  // true if import is in unreachable code
const IMPORT_FLAG_MYPY_ONLY: u8 = 1 << 2;    // true if import is mypy-only (e.g., in TYPE_CHECKING block)

// Used to report which imports are used in a file
enum ImportStatement {
    Import {
        name: String,
        relative: i32,           // Number of dots in relative import 'import ..x'
        as_name: Option<String>, // Set for 'import x as y'
        range: TextRange,        // Source range of the import alias
        flags: u8,               // Bitfield of IMPORT_FLAG_* constants
    },
    ImportFrom {
        module: String, // Module being imported from (empty string for "from . import x")
        relative: i32,  // Number of dots in relative import
        names: Vec<(String, Option<String>)>, // List of (name, as_name) tuples
        range: TextRange, // Source range of the entire import statement
        flags: u8,       // Bitfield of IMPORT_FLAG_* constants
    },
    ImportAll {
        module: String,   // Module being imported from (empty string for "from . import *")
        relative: i32,    // Number of dots in relative import
        range: TextRange, // Source range of the entire import statement
        flags: u8,        // Bitfield of IMPORT_FLAG_* constants
    },
}

struct Serializer<'a> {
    bytes: Vec<u8>,
    imports: Vec<ImportStatement>, // Encountered import statements
    line_index: LineIndex,
    text: &'a str,
    skip_function_bodies: bool, // Whether to omit function bodies without visible effects
    in_class: bool,             // Whether we're currently inside a class definition
    in_function: bool,          // Whether we're currently inside a function definition
    is_all_ascii: bool,         // Whether the entire file contains only ASCII characters
    lines_with_non_ascii: Vec<bool>, // Per-line flags: true if line has non-ASCII (empty if is_all_ascii)
    type_comments: HashMap<usize, ast::Expr>, // Type comments by line number (1-indexed)
    options: Options,           // Reachability analysis options
    current_unreachable: bool,  // Whether we're currently in an unreachable block
    current_mypy_only: bool,    // Whether we're currently in a mypy-only block (e.g., if TYPE_CHECKING)
    top_level_getattr: bool,    // Does module have top-level __getattr__() function
}

impl<'a> Serializer<'a> {
    #[inline]
    fn write_tag(&mut self, i: u8) {
        self.bytes.push(i);
    }

    #[inline]
    fn write_end_tag(&mut self) {
        self.write_tag(TAG_END);
    }

    #[inline]
    fn write_tagged_int(&mut self, i: i64) {
        self.write_tag(TAG_LITERAL_INT);
        self.write_int(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.write_int(i as i64);
    }

    fn write_bytes(&mut self, b: &[u8]) {
        self.write_tag(TAG_LITERAL_STR);
        self.write_usize(b.len());
        self.bytes.extend_from_slice(b);
    }

    fn write_bool(&mut self, b: bool) {
        self.bytes.push(if b {
            TAG_LITERAL_TRUE
        } else {
            TAG_LITERAL_FALSE
        });
    }

    fn write_int(&mut self, i: i64) {
        if i >= MIN_SHORT_INT && i < 128 + MIN_SHORT_INT {
            // 1-byte format
            self.bytes.push(((i - MIN_SHORT_INT) << 1) as u8);
        } else if i >= MIN_TWO_BYTES_INT && i <= MAX_TWO_BYTES_INT {
            // 2-byte format
            let x: u16 = (((i - MIN_TWO_BYTES_INT) << 2) | TWO_BYTES_INT_BIT) as u16;
            self.bytes.extend_from_slice(&x.to_le_bytes());
        } else if i >= MIN_FOUR_BYTES_INT && i <= MAX_FOUR_BYTES_INT {
            // 4-byte format
            let x: u32 = (((i - MIN_FOUR_BYTES_INT) << 3) | FOUR_BYTES_INT_TRAILER) as u32;
            self.bytes.extend_from_slice(&x.to_le_bytes());
        } else {
            // Variable-length format
            self.bytes.push(LONG_INT_TRAILER);
            let neg = i < 0;
            let absval = if neg {
                i.wrapping_abs() as u64
            } else {
                i as u64
            };
            let bytes = absval.to_le_bytes();
            let mut n = bytes.len();
            while n > 1 && bytes[n - 1] == 0 {
                n -= 1;
            }
            self.write_int(((n as i64) << 1) | (neg as i64));
            self.bytes.extend_from_slice(&bytes[..n]);
        }
    }

    /// Find the largest byte offset <= target that is a UTF-8 character boundary.
    /// Similar to the unstable str::floor_char_boundary method.
    fn floor_char_boundary(&self, s: &str, mut target: usize) -> usize {
        if target > s.len() {
            target = s.len();
        }
        // Walk backwards until we find a character boundary
        while target > 0 && !s.is_char_boundary(target) {
            target -= 1;
        }
        target
    }

    /// Convert a column number (UTF-8 byte offset) to a Unicode code point offset
    /// if the line contains non-ASCII characters. Returns the column as-is for ASCII lines.
    ///
    /// Note: This should only be called when is_all_ascii is false (checked by caller).
    fn convert_column_to_codepoint(&self, line_number: usize, byte_column: usize) -> usize {
        // Check if this specific line has non-ASCII
        // Note: line_number is 1-indexed, but Vec is 0-indexed
        let line_idx = line_number.saturating_sub(1);
        if line_idx < self.lines_with_non_ascii.len() && self.lines_with_non_ascii[line_idx] {
            // This line has non-ASCII, need to convert
            // Get the byte offset of the start of this line in the full text
            let line_start = self.line_index.line_start(
                ruff_source_file::OneIndexed::from_zero_indexed(line_idx),
                self.text,
            );
            let line_start_byte = line_start.to_usize();

            // Calculate the absolute byte position in the full text
            let target_byte = line_start_byte + byte_column;

            // Make sure we don't go past the end of the text and are at a char boundary
            let safe_target = target_byte.min(self.text.len());

            // Ensure we're at a valid character boundary
            let char_boundary = self.floor_char_boundary(self.text, safe_target);

            // Extract the portion of the line from start to the target column
            let line_prefix = &self.text[line_start_byte..char_boundary];

            // Count code points in this prefix
            line_prefix.chars().count()
        } else {
            // This line is ASCII, no conversion needed
            byte_column
        }
    }

    fn write_location(&mut self, range: TextRange) {
        self.write_tag(TAG_LOCATION);
        let st_loc = self.line_index.line_column(range.start(), self.text);
        let st_line = st_loc.line.get() as i64;
        let st_column_bytes = st_loc.column.get();

        let end_loc = self.line_index.line_column(range.end(), self.text);
        let end_column_bytes = end_loc.column.get();

        // Fast path for all-ASCII files: use byte offsets directly (no conversion needed)
        // Note: Ruff uses 1-based columns, but mypy expects 0-based, so subtract 1
        if self.is_all_ascii {
            self.write_int(st_line);
            self.write_int((st_column_bytes - 1) as i64); // Convert to 0-based
            self.write_int((end_loc.line.get() as i64) - st_line);
            self.write_int((end_column_bytes as i64) - (st_column_bytes as i64));
        } else {
            // Convert byte offset to code point offset for Python compatibility
            // Note: Ruff uses 1-based columns, but mypy expects 0-based, so subtract 1
            let st_column =
                self.convert_column_to_codepoint(st_loc.line.get(), st_column_bytes) as i64;
            let end_column =
                self.convert_column_to_codepoint(end_loc.line.get(), end_column_bytes) as i64;

            self.write_int(st_line);
            self.write_int(st_column - 1); // Convert to 0-based
            self.write_int((end_loc.line.get() as i64) - st_line);
            self.write_int(end_column - st_column);
        }
    }

    fn serialize_block(&mut self, block: &Vec<ast::Stmt>) {
        self.write_tag(TAG_BLOCK);
        self.write_tag(TAG_LIST_GEN);
        self.write_usize(block.len());
        self.write_bool(self.current_unreachable);
        for stmt in block {
            stmt.serialize(self);
        }
        self.write_end_tag();
    }

    fn serialize_empty_block(&mut self, range: TextRange) {
        self.write_tag(TAG_BLOCK);
        self.write_tag(TAG_LIST_GEN);
        self.write_int(0); // Empty list of statements
        self.write_bool(self.current_unreachable);
        self.write_location(range); // Write location after zero-length list
        self.write_end_tag();
    }

}

trait Ser {
    fn serialize(&self, ser: &mut Serializer);
}

impl Ser for Vec<ast::Expr> {
    fn serialize(&self, ser: &mut Serializer) {
        ser.write_tag(TAG_LIST_GEN);
        ser.write_int(self.len() as i64);
        for e in self {
            e.serialize(ser);
        }
    }
}

impl Ser for [ast::Expr] {
    fn serialize(&self, ser: &mut Serializer) {
        ser.write_tag(TAG_LIST_GEN);
        ser.write_int(self.len() as i64);
        for e in self {
            e.serialize(ser);
        }
    }
}

impl Ser for Option<Box<ast::Expr>> {
    fn serialize(&self, ser: &mut Serializer) {
        if let Some(v) = &self {
            ser.write_bool(true);
            v.serialize(ser);
        } else {
            ser.write_bool(false);
        }
    }
}

impl Ser for ast::Mod {
    fn serialize(&self, ser: &mut Serializer) {
        match self {
            ast::Mod::Module(m) => {
                ser.write_tagged_int(m.body.len() as i64);
                for stmt in &m.body {
                    stmt.serialize(ser);
                }
            }
            ast::Mod::Expression(_) => {
                panic!("Expression unsupported");
            }
        }
    }
}

/// Extract type comments (both type: ignore and type annotations) from tokens in a single pass.
///
/// # Arguments
///
/// * `tokens` - Reference to the tokens from parsing
/// * `source` - The source code text
/// * `line_index` - Line index for converting positions to line numbers
///
/// # Returns
///
/// A tuple containing:
/// - A vector of tuples (line_number, error_codes) where `type: ignore` comments appear
/// - A HashMap mapping line numbers (1-indexed) to parsed type annotation AST expressions
///
/// This function combines the functionality of extract_type_ignore_lines and extract_type_comments
/// to avoid two separate passes over the token sequence, improving cache locality.
fn extract_type_comments_and_ignores(
    tokens: &ruff_python_parser::Tokens,
    source: &str,
    line_index: &LineIndex,
) -> (Vec<(usize, Vec<String>)>, HashMap<usize, ast::Expr>) {
    let mut type_ignore_lines = Vec::new();
    let mut type_comments = HashMap::new();

    for token in tokens.iter() {
        if token.kind().is_comment() {
            let comment_text = &source[token.range()];
            let location = line_index.line_column(token.start(), source);
            let line_number = location.line.get();

            if let Some(parts) = type_comment::parse_type_comments(comment_text) {
                for part in parts {
                    match part {
                        type_comment::TypeComment::Ignore(error_codes) => {
                            type_ignore_lines.push((line_number, error_codes));
                        }
                        type_comment::TypeComment::TypeAnnotation(annotation) => {
                            let wrapped = format!("({})", annotation);
                            let parse_result =
                                parse_unchecked(&wrapped, ParseOptions::from(Mode::Expression));

                            if parse_result.errors().is_empty() {
                                if let ast::Mod::Expression(expr_mod) = parse_result.into_syntax() {
                                    type_comments.insert(line_number, *expr_mod.body);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    (type_ignore_lines, type_comments)
}

/// Helper function to serialize bytes literal to escaped string representation
/// Used for BytesExpr in expression contexts where we need a human-readable form
fn serialize_bytes_to_escaped_string(bytes_lit: &ast::ExprBytesLiteral) -> Vec<u8> {
    let mut result = Vec::new();
    for bytes_part in bytes_lit.value.iter() {
        for &byte in bytes_part.value.iter() {
            match byte {
                b'\r' => result.extend_from_slice(b"\\r"),
                b'\n' => result.extend_from_slice(b"\\n"),
                b'\t' => result.extend_from_slice(b"\\t"),
                b'\\' => result.extend_from_slice(b"\\\\"),
                b'\'' => result.extend_from_slice(b"\\'"),
                // Printable ASCII characters (space to ~)
                32..=126 => result.push(byte),
                // Everything else as hex escape
                _ => {
                    result.extend_from_slice(b"\\x");
                    result.push(b"0123456789abcdef"[(byte >> 4) as usize]);
                    result.push(b"0123456789abcdef"[(byte & 0xf) as usize]);
                }
            }
        }
    }
    result
}

/// Helper function to serialize comprehensions (shared by Generator, ListComp, SetComp)
fn serialize_comprehension(
    ser: &mut Serializer,
    elt: &ast::Expr,
    generators: &[ast::Comprehension],
    range: ruff_text_size::TextRange,
) {
    // Serialize element expression
    elt.serialize(ser);
    // Serialize number of generators
    ser.write_tagged_int(generators.len() as i64);
    // Serialize all indices (targets)
    for comp in generators {
        comp.target.serialize(ser);
    }
    // Serialize all sequences (iters)
    for comp in generators {
        comp.iter.serialize(ser);
    }
    // Serialize all condlists (ifs for each generator)
    for comp in generators {
        comp.ifs.serialize(ser);
    }
    // Serialize all is_async flags
    for comp in generators {
        ser.write_bool(comp.is_async);
    }
    ser.write_location(range);
}

/// Check if argument name should be elided (treated as positional-only).
/// Returns true if the name starts with "__" but doesn't end with "__".
/// This matches the behavior of mypy.sharedparse.argument_elide_name.
fn argument_elide_name(name: &str) -> bool {
    name.starts_with("__") && !name.ends_with("__")
}

fn serialize_parameters(ser: &mut Serializer, params: &ast::Parameters) {
    // Count total number of arguments
    let mut arg_count = 0;
    arg_count += params.posonlyargs.len();
    arg_count += params.args.len();
    if params.vararg.is_some() {
        arg_count += 1;
    }
    arg_count += params.kwonlyargs.len();
    if params.kwarg.is_some() {
        arg_count += 1;
    }

    // Write argument list
    ser.write_tag(TAG_LIST_GEN);
    ser.write_int(arg_count as i64);

    // Serialize positional-only arguments
    for param in &params.posonlyargs {
        serialize_argument(
            ser,
            &param.parameter,
            param.default.as_deref(),
            ARG_POS,
            ARG_OPT,
            true,
        );
    }

    // Serialize regular positional arguments
    for param in &params.args {
        let pos_only = argument_elide_name(&param.parameter.name);
        serialize_argument(
            ser,
            &param.parameter,
            param.default.as_deref(),
            ARG_POS,
            ARG_OPT,
            pos_only,
        );
    }

    // Serialize *args
    if let Some(vararg) = &params.vararg {
        serialize_argument(ser, vararg, None, ARG_STAR, ARG_STAR, false);
    }

    // Serialize keyword-only arguments
    for param in &params.kwonlyargs {
        serialize_argument(
            ser,
            &param.parameter,
            param.default.as_deref(),
            ARG_NAMED,
            ARG_NAMED_OPT,
            false,
        );
    }

    // Serialize **kwargs
    if let Some(kwarg) = &params.kwarg {
        serialize_argument(ser, kwarg, None, ARG_STAR2, ARG_STAR2, false);
    }
}

fn serialize_argument(
    ser: &mut Serializer,
    param: &ast::Parameter,
    default_expr: Option<&ast::Expr>,
    kind_no_default: i64,
    kind_with_default: i64,
    pos_only: bool,
) {
    // Argument name
    ser.write_bytes(param.name.as_bytes());

    // Argument kind
    let kind = if default_expr.is_some() {
        kind_with_default
    } else {
        kind_no_default
    };
    ser.write_tagged_int(kind);

    if let Some(ann) = &param.annotation {
        ser.write_bool(true);
        serialize_type(ser, ann);
    } else {
        ser.write_bool(false);
    }

    // Default value
    if let Some(expr) = default_expr {
        ser.write_bool(true);
        expr.serialize(ser);
    } else {
        ser.write_bool(false);
    }

    // pos_only flag
    ser.write_bool(pos_only);

    ser.write_location(param.range());
}

fn serialize_type_params(ser: &mut Serializer, type_params: &ast::TypeParams) {
    // Serialize each type parameter
    for type_param in &type_params.type_params {
        match type_param {
            ast::TypeParam::TypeVar(tv) => {
                // Type param kind
                ser.write_tagged_int(TYPE_VAR_KIND);

                // Name
                ser.write_bytes(tv.name.as_bytes());

                // Check if bound is a tuple (constrained TypeVar)
                let (has_upper_bound, values) = if let Some(bound) = &tv.bound {
                    if let ast::Expr::Tuple(tuple_expr) = bound.as_ref() {
                        // Constrained TypeVar: T: (int, str)
                        // Values come from the tuple elements
                        (false, tuple_expr.elts.as_slice())
                    } else {
                        // Regular bounded TypeVar: T: str
                        (true, &[] as &[ast::Expr])
                    }
                } else {
                    (false, &[] as &[ast::Expr])
                };

                // Upper bound
                if has_upper_bound {
                    ser.write_bool(true);
                    serialize_type(ser, tv.bound.as_ref().unwrap());
                } else {
                    ser.write_bool(false);
                }

                // Values (for constrained TypeVar)
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(values.len() as i64);
                for value in values {
                    serialize_type(ser, value);
                }

                // Default
                if let Some(default) = &tv.default {
                    ser.write_bool(true);
                    serialize_type(ser, default);
                } else {
                    ser.write_bool(false);
                }
            }
            ast::TypeParam::ParamSpec(ps) => {
                // Type param kind
                ser.write_tagged_int(PARAM_SPEC_KIND);

                // Name
                ser.write_bytes(ps.name.as_bytes());

                // Upper bound (None for ParamSpec)
                ser.write_bool(false);

                // Values (empty for ParamSpec)
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(0);

                // Default
                if let Some(default) = &ps.default {
                    ser.write_bool(true);
                    serialize_type(ser, default);
                } else {
                    ser.write_bool(false);
                }
            }
            ast::TypeParam::TypeVarTuple(tvt) => {
                // Type param kind
                ser.write_tagged_int(TYPE_VAR_TUPLE_KIND);

                // Name
                ser.write_bytes(tvt.name.as_bytes());

                // Upper bound (None for TypeVarTuple)
                ser.write_bool(false);

                // Values (empty for TypeVarTuple)
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(0);

                // Default
                if let Some(default) = &tvt.default {
                    ser.write_bool(true);
                    serialize_type(ser, default);
                } else {
                    ser.write_bool(false);
                }
            }
        }
    }
}

impl Ser for ast::Stmt {
    fn serialize(&self, ser: &mut Serializer) {
        match self {
            ast::Stmt::FunctionDef(f) => {
                if !f.decorator_list.is_empty() {
                    ser.write_tag(TAG_DECORATOR);
                    // Serialize decorators
                    ser.write_tag(TAG_LIST_GEN);
                    ser.write_usize(f.decorator_list.len());
                    for dec in &f.decorator_list {
                        dec.expression.serialize(ser);
                    }
                    // Serialize start location of the decorator. End is same as the func def.
                    // Note: Decorator column intentionally uses 1-based (legacy parser behavior)
                    let start_loc = ser
                        .line_index
                        .line_column(f.decorator_list.first().unwrap().range().start(), ser.text);
                    ser.write_tagged_int(start_loc.line.get() as i64);
                    ser.write_tagged_int(start_loc.column.get() as i64);
                }

                ser.write_tag(TAG_FUNC_DEF);

                // Function name
                ser.write_bytes(f.name.as_bytes());
                // Parameters
                serialize_parameters(ser, &f.parameters);

                // Body - may be omitted if skip_function_bodies is enabled
                let should_serialize_body = if ser.skip_function_bodies {
                    // Check for externally visible effects
                    // For methods (in_class), check both attributes and yield
                    // For top-level functions, check only yield
                    func_effect_visitor::has_externally_visible_effect(
                        &f.body,
                        &f.parameters,
                        ser.in_class, // Only check attributes for methods in classes
                    )
                } else {
                    true
                };

                if !ser.in_class && !ser.in_function && f.name.as_str() == "__getattr__" {
                    ser.top_level_getattr = true;
                };

                // Body - mark that we're inside a function
                let was_in_function = ser.in_function;
                ser.in_function = true;

                if should_serialize_body {
                    if f.body.is_empty() {
                        // Empty body due to syntax error - use serialize_empty_block
                        // to ensure location is written (required by deserializer)
                        ser.serialize_empty_block(f.range());
                    } else {
                        ser.serialize_block(&f.body);
                    }
                } else {
                    // Use the range covering the entire body (start of first stmt to end of last stmt)
                    let body_range = if !f.body.is_empty() {
                        TextRange::new(f.body[0].start(), f.body[f.body.len() - 1].end())
                    } else {
                        f.range()
                    };
                    ser.serialize_empty_block(body_range);
                }

                ser.in_function = was_in_function;

                ser.write_bool(f.is_async);

                // Type parameters
                if let Some(type_params) = &f.type_params {
                    ser.write_bool(true);
                    ser.write_int(type_params.type_params.len() as i64);
                    serialize_type_params(ser, type_params);
                } else {
                    ser.write_bool(false);
                }

                // Return type annotation
                if let Some(ret) = &f.returns {
                    ser.write_bool(true); // No return annotation
                    serialize_type(ser, ret);
                } else {
                    ser.write_bool(false); // No return annotation
                }

                // Write location
                if !f.decorator_list.is_empty() {
                    // For decorated functions, compute def keyword position from function name
                    // Assuming single space between tokens: "def " or "async def "
                    ser.write_tag(TAG_LOCATION);

                    let name_loc = ser.line_index.line_column(f.name.range.start(), ser.text);
                    let end_loc = ser.line_index.line_column(f.range().end(), ser.text);

                    // Compute def keyword column by subtracting offset from name position
                    // "def " = 4 characters, "async def " = 10 characters
                    // Note: Ruff uses 1-based columns, convert to 0-based for mypy
                    let def_offset = if f.is_async { 10 } else { 4 };
                    let def_column = (name_loc.column.get() - 1) as i64 - def_offset;

                    let st_line = name_loc.line.get() as i64;

                    ser.write_int(st_line);
                    ser.write_int(def_column);

                    // End deltas (relative to start)
                    ser.write_int((end_loc.line.get() as i64) - st_line);
                    ser.write_int((end_loc.column.get() - 1) as i64 - def_column);
                } else {
                    // No decorators: use the full range (already starts at async/def)
                    ser.write_location(f.range());
                }

                if !f.decorator_list.is_empty() {
                    // Extra end tag for the Decorator wrapper in mypy AST
                    ser.write_end_tag();
                }
            }
            ast::Stmt::Expr(e) => {
                ser.write_tag(TAG_EXPR_STMT);
                e.value.serialize(ser);
            }
            ast::Stmt::Assign(a) => {
                ser.write_tag(TAG_ASSIGN);
                a.targets.serialize(ser);
                a.value.serialize(ser);

                // Check if there's a type comment on the same line as this assignment
                let location = ser.line_index.line_column(a.start(), ser.text);
                let line_number = location.line.get();

                // Clone the type expression to avoid borrow checker issues
                let type_expr = ser.type_comments.get(&line_number).cloned();

                if let Some(type_expr) = type_expr {
                    // Has type annotation from type comment
                    ser.write_bool(true);
                    serialize_type(ser, &type_expr);
                } else {
                    // No type annotation
                    ser.write_bool(false);
                }

                // new_syntax = false (not using PEP 526 syntax, using type comment)
                ser.write_bool(false);
                ser.write_location(a.range());
            }
            ast::Stmt::AnnAssign(a) => {
                ser.write_tag(TAG_ASSIGN);
                // Serialize target as a single-element list
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(1);
                a.target.serialize(ser);
                // Serialize value (or TempNode if annotation-only)
                if let Some(value) = &a.value {
                    value.serialize(ser);
                } else {
                    // For annotation-only (x: int), serialize as TempNode
                    ser.write_tag(TAG_TEMP_NODE);
                    ser.write_end_tag();
                }
                // has_type = true
                ser.write_bool(true);
                // Serialize type annotation
                serialize_type(ser, &a.annotation);
                // new_syntax = true (using PEP 526 syntax)
                ser.write_bool(true);
                ser.write_location(a.range());
            }
            ast::Stmt::AugAssign(a) => {
                ser.write_tag(TAG_OPERATOR_ASSIGNMENT_STMT);
                // Serialize operator as string
                ser.write_bytes(a.op.as_str().as_bytes());
                // Serialize lvalue (target)
                a.target.serialize(ser);
                // Serialize rvalue (value)
                a.value.serialize(ser);
                ser.write_location(a.range());
            }
            ast::Stmt::Import(i) => {
                ser.write_tag(TAG_IMPORT);
                // Write number of imports
                ser.write_tagged_int(i.names.len() as i64);
                for name in &i.names {
                    // Write import name
                    ser.write_bytes(name.name.as_bytes());
                    // Write as_name (optional)
                    if let Some(asname) = &name.asname {
                        ser.write_bool(true);
                        ser.write_bytes(asname.as_bytes());
                    } else {
                        ser.write_bool(false);
                    }
                    ser.imports.push(ImportStatement::Import {
                        name: name.name.to_string(),
                        relative: 0, // Not a relative import
                        as_name: name.asname.as_ref().map(|n| n.to_string()),
                        range: name.range,
                        flags: make_import_flags(ser),
                    });
                }
                ser.write_location(i.range());
            }
            ast::Stmt::ImportFrom(ifrom) => {
                // Check if this is a wildcard import (from m import *)
                if ifrom.names.len() == 1 && ifrom.names[0].name.as_str() == "*" {
                    // Serialize as ImportAll
                    ser.write_tag(TAG_IMPORT_ALL);

                    // Write module name (empty string for "from . import *")
                    ser.write_bytes(ifrom.module.as_ref().map_or(b"", |m| m.as_bytes()));

                    // Write relative import level (number of dots)
                    ser.write_tagged_int(ifrom.level as i64);

                    ser.write_location(ifrom.range());

                    // Track in imports list for dependency tracking
                    ser.imports.push(ImportStatement::ImportAll {
                        module: ifrom
                            .module
                            .as_ref()
                            .map_or(String::new(), |m| m.to_string()),
                        relative: ifrom.level as i32,
                        range: ifrom.range(),
                        flags: make_import_flags(ser),
                    });
                } else {
                    // Regular from...import statement
                    ser.write_tag(TAG_IMPORT_FROM);

                    // Write relative import level (number of dots)
                    ser.write_tagged_int(ifrom.level as i64);

                    // Write module name (empty string for "from . import x")
                    ser.write_bytes(ifrom.module.as_ref().map_or(b"", |m| m.as_bytes()));

                    // Write number of imported names
                    ser.write_tagged_int(ifrom.names.len() as i64);

                    // Collect names for dependency tracking
                    let mut names = Vec::new();

                    // Write each name and optional alias
                    for alias in &ifrom.names {
                        ser.write_bytes(alias.name.as_bytes());
                        if let Some(asname) = &alias.asname {
                            ser.write_bool(true);
                            ser.write_bytes(asname.as_bytes());
                        } else {
                            ser.write_bool(false);
                        }

                        // Collect for dependency tracking
                        names.push((
                            alias.name.to_string(),
                            alias.asname.as_ref().map(|n| n.to_string()),
                        ));
                    }

                    // Track in imports list for dependency tracking
                    ser.imports.push(ImportStatement::ImportFrom {
                        module: ifrom
                            .module
                            .as_ref()
                            .map_or(String::new(), |m| m.to_string()),
                        relative: ifrom.level as i32,
                        names,
                        range: ifrom.range(),
                        flags: make_import_flags(ser),
                    });

                    ser.write_location(ifrom.range());
                }
            }
            ast::Stmt::Return(s) => {
                ser.write_tag(TAG_RETURN);
                s.value.serialize(ser);
                ser.write_location(s.range());
            }
            ast::Stmt::Raise(r) => {
                ser.write_tag(TAG_RAISE_STMT);
                // Serialize exception expression (optional)
                r.exc.serialize(ser);
                // Serialize from expression (optional)
                r.cause.serialize(ser);
                ser.write_location(r.range());
            }
            ast::Stmt::Assert(a) => {
                ser.write_tag(TAG_ASSERT_STMT);
                // Serialize test expression
                a.test.serialize(ser);
                // Serialize optional message expression
                a.msg.serialize(ser);
                ser.write_location(a.range());
            }
            ast::Stmt::If(s) => serialize_if_stmt(ser, s),
            ast::Stmt::While(s) => {
                ser.write_tag(TAG_WHILE);
                s.test.serialize(ser);
                ser.serialize_block(&s.body);
                ser.serialize_block(&s.orelse);
                ser.write_location(s.range());
            }
            ast::Stmt::For(f) => {
                ser.write_tag(TAG_FOR_STMT);
                // Serialize index (target)
                f.target.serialize(ser);
                // Serialize iterator expression
                f.iter.serialize(ser);
                // Serialize body
                ser.serialize_block(&f.body);
                // Serialize else clause
                ser.serialize_block(&f.orelse);
                // Serialize is_async flag
                ser.write_bool(f.is_async);
                ser.write_location(f.range());
            }
            ast::Stmt::With(w) => {
                ser.write_tag(TAG_WITH_STMT);
                // Write number of items
                ser.write_tagged_int(w.items.len() as i64);
                // Serialize each item
                for item in &w.items {
                    // Serialize context expression
                    item.context_expr.serialize(ser);
                    // Serialize optional target
                    item.optional_vars.serialize(ser);
                }
                // Serialize body
                ser.serialize_block(&w.body);
                // Serialize is_async flag
                ser.write_bool(w.is_async);
                ser.write_location(w.range());
            }
            ast::Stmt::Pass(s) => {
                ser.write_tag(TAG_PASS_STMT);
                ser.write_location(s.range());
            }
            ast::Stmt::Break(s) => {
                ser.write_tag(TAG_BREAK_STMT);
                ser.write_location(s.range());
            }
            ast::Stmt::Continue(s) => {
                ser.write_tag(TAG_CONTINUE_STMT);
                ser.write_location(s.range());
            }
            ast::Stmt::ClassDef(c) => {
                ser.write_tag(TAG_CLASS_DEF);

                // Class name
                ser.write_bytes(c.name.as_bytes());

                // Body - mark that we're inside a class
                let was_in_class = ser.in_class;
                ser.in_class = true;
                if c.body.is_empty() {
                    // Empty body due to syntax error - use serialize_empty_block
                    // to ensure location is written (required by deserializer)
                    ser.serialize_empty_block(c.range());
                } else {
                    ser.serialize_block(&c.body);
                }
                ser.in_class = was_in_class;

                // Base classes (positional arguments in class definition)
                ser.write_tag(TAG_LIST_GEN);
                if let Some(args) = &c.arguments {
                    ser.write_int(args.args.len() as i64);
                    for base in &args.args {
                        base.serialize(ser);
                    }
                } else {
                    ser.write_int(0); // No base classes
                }

                // Decorators
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(c.decorator_list.len() as i64);
                for dec in &c.decorator_list {
                    dec.expression.serialize(ser);
                }

                // Type parameters
                if let Some(type_params) = &c.type_params {
                    ser.write_bool(true);
                    ser.write_int(type_params.type_params.len() as i64);
                    serialize_type_params(ser, type_params);
                } else {
                    ser.write_bool(false);
                }

                // Keywords (all keyword arguments including metaclass)
                ser.write_tag(TAG_DICT_STR_GEN);
                if let Some(args) = &c.arguments {
                    // Count keywords with argument names (exclude **kwargs which have no arg)
                    let named_keywords: Vec<_> =
                        args.keywords.iter().filter(|kw| kw.arg.is_some()).collect();
                    ser.write_int(named_keywords.len() as i64);

                    // Serialize each keyword: name then value
                    for kw in named_keywords {
                        if let Some(arg_name) = &kw.arg {
                            ser.write_bytes(arg_name.as_bytes());
                            kw.value.serialize(ser);
                        }
                    }
                } else {
                    ser.write_int(0); // No keywords
                }

                // Write location
                if !c.decorator_list.is_empty() {
                    // Custom location: use class name's line with last decorator's column
                    ser.write_tag(TAG_LOCATION);

                    let last_decorator = c.decorator_list.last().unwrap();
                    let decorator_loc = ser
                        .line_index
                        .line_column(last_decorator.range().start(), ser.text);
                    let name_loc = ser.line_index.line_column(c.name.range.start(), ser.text);
                    let end_loc = ser.line_index.line_column(c.range().end(), ser.text);

                    // Start: name's line, decorator's column
                    // Note: Ruff uses 1-based columns, convert to 0-based for mypy
                    let st_line = name_loc.line.get() as i64;
                    let st_column = (decorator_loc.column.get() - 1) as i64;

                    ser.write_int(st_line);
                    ser.write_int(st_column);

                    // End deltas (relative to start)
                    ser.write_int((end_loc.line.get() as i64) - st_line);
                    ser.write_int((end_loc.column.get() - 1) as i64 - st_column);
                } else {
                    // No decorators: use the full range
                    ser.write_location(c.range());
                }
            }
            ast::Stmt::Try(t) => {
                ser.write_tag(TAG_TRY_STMT);

                // Serialize try body
                ser.serialize_block(&t.body);

                // Serialize number of except handlers
                ser.write_tagged_int(t.handlers.len() as i64);

                // Serialize exception types for each handler
                for handler in &t.handlers {
                    match handler {
                        ast::ExceptHandler::ExceptHandler(h) => {
                            if let Some(type_expr) = &h.type_ {
                                ser.write_bool(true);
                                type_expr.serialize(ser);
                            } else {
                                ser.write_bool(false);
                            }
                        }
                    }
                }

                // Serialize variable names for each handler
                for handler in &t.handlers {
                    match handler {
                        ast::ExceptHandler::ExceptHandler(h) => {
                            if let Some(name) = &h.name {
                                ser.write_bool(true);
                                ser.write_bytes(name.as_bytes());
                            } else {
                                ser.write_bool(false);
                            }
                        }
                    }
                }

                // Serialize handler bodies
                for handler in &t.handlers {
                    match handler {
                        ast::ExceptHandler::ExceptHandler(h) => {
                            ser.serialize_block(&h.body);
                        }
                    }
                }

                // Serialize else body (optional)
                if !t.orelse.is_empty() {
                    ser.write_bool(true);
                    ser.serialize_block(&t.orelse);
                } else {
                    ser.write_bool(false);
                }

                // Serialize finally body (optional)
                if !t.finalbody.is_empty() {
                    ser.write_bool(true);
                    ser.serialize_block(&t.finalbody);
                } else {
                    ser.write_bool(false);
                }

                // Serialize is_star flag (for except* in Python 3.11+)
                ser.write_bool(t.is_star);

                ser.write_location(t.range());
            }
            ast::Stmt::Delete(d) => {
                ser.write_tag(TAG_DEL_STMT);
                // Serialize the target expression
                // If there's only one target, serialize it directly
                // If there are multiple targets, serialize as a tuple
                if d.targets.len() == 1 {
                    d.targets[0].serialize(ser);
                } else {
                    // Serialize as a tuple expression
                    ser.write_tag(TAG_TUPLE_EXPR);
                    d.targets.serialize(ser);
                    ser.write_location(d.range());
                    ser.write_end_tag();
                }
                ser.write_location(d.range());
            }
            ast::Stmt::Global(g) => {
                ser.write_tag(TAG_GLOBAL_DECL);
                // Write number of names
                ser.write_tagged_int(g.names.len() as i64);
                // Write each name
                for name in &g.names {
                    ser.write_bytes(name.as_bytes());
                }
                ser.write_location(g.range());
            }
            ast::Stmt::Nonlocal(n) => {
                ser.write_tag(TAG_NONLOCAL_DECL);
                // Write number of names
                ser.write_tagged_int(n.names.len() as i64);
                // Write each name
                for name in &n.names {
                    ser.write_bytes(name.as_bytes());
                }
                ser.write_location(n.range());
            }
            ast::Stmt::Match(m) => {
                ser.write_tag(TAG_MATCH_STMT);
                // Serialize subject expression
                m.subject.serialize(ser);
                // Write number of cases
                ser.write_tagged_int(m.cases.len() as i64);
                // Serialize each case
                for case in &m.cases {
                    // Serialize pattern
                    case.pattern.serialize(ser);
                    // Serialize optional guard
                    case.guard.serialize(ser);
                    // Serialize body
                    ser.serialize_block(&case.body);
                }
                ser.write_location(m.range());
            }
            ast::Stmt::TypeAlias(ta) => {
                ser.write_tag(TAG_TYPE_ALIAS_STMT);

                // Name (as NameExpr)
                ta.name.serialize(ser);

                // Type parameters
                if let Some(type_params) = &ta.type_params {
                    ser.write_int(type_params.type_params.len() as i64);
                    serialize_type_params(ser, type_params);
                } else {
                    ser.write_int(0);
                }

                // Value (the RHS type expression - deserialization will wrap it in LambdaExpr)
                ta.value.serialize(ser);

                // TypeAliasStmt location
                ser.write_location(ta.range());
            }
            _ => {
                panic!("unsupported: {self:?}");
            }
        };
        ser.write_end_tag()
    }
}

fn is_always_or_mypy_false(truth: TruthValue) -> bool {
    matches!(truth, TruthValue::AlwaysFalse | TruthValue::MypyFalse)
}

fn is_always_or_mypy_true(truth: TruthValue) -> bool {
    matches!(truth, TruthValue::AlwaysTrue | TruthValue::MypyTrue)
}

/// Stateful reachability analyzer for if/elif/else chains.
///
/// This is intentionally kept separate from serialization so emit logic can
/// eventually call it in sequence:
/// - `condition_flags(expr)` for each if/elif condition
/// - `else_flags()` for the optional else block
#[allow(dead_code)]
struct IfReachabilityAnalyzer<'a> {
    options: &'a Options,
    tail_unreachable: bool,
    seen_mypy_true: bool,
    seen_mypy_false: bool,
}

#[allow(dead_code)]
impl<'a> IfReachabilityAnalyzer<'a> {
    fn new(options: &'a Options) -> Self {
        Self {
            options,
            tail_unreachable: false,
            seen_mypy_true: false,
            seen_mypy_false: false,
        }
    }

    /// Analyze one if/elif condition and advance analyzer state.
    ///
    /// Returns `(unreachable, mypy_only)` for the corresponding block.
    fn condition_flags(&mut self, expr: &ast::Expr) -> (bool, bool) {
        let truth = crate::reachability::infer_condition_value(expr, &self.options);

        let unreachable = self.tail_unreachable || is_always_or_mypy_false(truth);
        let mypy_only =
            !unreachable && !self.seen_mypy_true && truth == TruthValue::MypyTrue;

        self.tail_unreachable = self.tail_unreachable || is_always_or_mypy_true(truth);
        self.seen_mypy_true = self.seen_mypy_true || truth == TruthValue::MypyTrue;
        self.seen_mypy_false = self.seen_mypy_false || truth == TruthValue::MypyFalse;

        (unreachable, mypy_only)
    }

    /// Returns `(unreachable, mypy_only)` for the else block.
    fn else_flags(&self) -> (bool, bool) {
        let unreachable = self.tail_unreachable;
        let mypy_only = !unreachable && self.seen_mypy_false && !self.seen_mypy_true;
        (unreachable, mypy_only)
    }
}

#[inline(always)]
fn with_branch_flags(
    ser: &mut Serializer,
    branch_unreachable: bool,
    branch_mypy_only: bool,
    f: impl FnOnce(&mut Serializer),
) {
    let old_unreachable = ser.current_unreachable;
    let old_mypy_only = ser.current_mypy_only;
    ser.current_unreachable = ser.current_unreachable || branch_unreachable;
    ser.current_mypy_only = ser.current_mypy_only || branch_mypy_only;
    f(ser);
    ser.current_unreachable = old_unreachable;
    ser.current_mypy_only = old_mypy_only;
}

fn serialize_if_stmt(ser: &mut Serializer, stmt: &ast::StmtIf) {
    let has_else = stmt
        .elif_else_clauses
        .last()
        .is_some_and(|clause| clause.test.is_none());

    // First analyze reachability of each block
    let (main_flags, clause_flags, synthetic_else_flags) = {
        let mut analyzer = IfReachabilityAnalyzer::new(&ser.options);
        let main_flags = analyzer.condition_flags(&stmt.test);
        let mut clause_flags = Vec::with_capacity(stmt.elif_else_clauses.len());
        for clause in &stmt.elif_else_clauses {
            let flags = match &clause.test {
                Some(expr) => analyzer.condition_flags(expr),
                None => analyzer.else_flags(),
            };
            clause_flags.push(flags);
        }
        let synthetic_else_flags = if has_else {
            None
        } else {
            Some(analyzer.else_flags())
        };
        (main_flags, clause_flags, synthetic_else_flags)
    };

    ser.write_tag(TAG_IF);
    stmt.test.serialize(ser);

    // Serialize main body with analyzer-provided flags.
    let (main_body_unreachable, main_body_mypy_only) = main_flags;
    with_branch_flags(
        ser,
        main_body_unreachable,
        main_body_mypy_only,
        |ser| ser.serialize_block(&stmt.body),
    );

    let num_elif = stmt.elif_else_clauses.len() - if has_else { 1 } else { 0 };
    ser.write_tagged_int(num_elif as i64);

    // Process elif/else clauses
    for (clause, (branch_unreachable, branch_mypy_only)) in
        stmt.elif_else_clauses.iter().zip(clause_flags.iter().copied())
    {
        match &clause.test {
            Some(expr) => {
                // elif clause
                expr.serialize(ser);
                with_branch_flags(ser, branch_unreachable, branch_mypy_only, |ser| {
                    ser.serialize_block(&clause.body)
                });
            }
            None => {
                // else clause
                ser.write_bool(true);
                with_branch_flags(ser, branch_unreachable, branch_mypy_only, |ser| {
                    ser.serialize_block(&clause.body)
                });
            }
        }
    }

    if !has_else {
        let (else_unreachable, else_mypy_only) =
            synthetic_else_flags.expect("synthetic else flags must exist when there is no else");
        if else_unreachable {
            // Serialize an empty block so that we can pass reachability information
            ser.write_bool(true);
            with_branch_flags(ser, else_unreachable, else_mypy_only, |ser| {
                ser.serialize_empty_block(stmt.range())
            });
        } else {
            ser.write_bool(false);
        }
    }
    ser.write_location(stmt.range());
}

impl Ser for ast::Expr {
    fn serialize(&self, ser: &mut Serializer) {
        match self {
            ast::Expr::Name(n) => {
                ser.write_tag(TAG_NAME_EXPR);
                ser.write_bytes(n.id.as_bytes());
                ser.write_location(n.range());
            }
            ast::Expr::Attribute(a) => {
                ser.write_tag(TAG_MEMBER_EXPR);
                a.value.serialize(ser);
                ser.write_bytes(a.attr.as_bytes());
                ser.write_location(a.range());
            }
            ast::Expr::StringLiteral(s) => {
                ser.write_tag(TAG_STR_EXPR);
                let value = &s.value;
                ser.write_tag(TAG_LITERAL_STR);
                ser.write_usize(value.len());
                for part in value.iter() {
                    ser.bytes.extend_from_slice(part.as_bytes());
                }
                ser.write_location(s.range());
            }
            ast::Expr::Call(c) => {
                ser.write_tag(TAG_CALL_EXPR);
                c.func.serialize(ser);
                let args = &c.arguments;

                // Serialize all arguments (positional + keyword + **kwargs)
                let total_args = args.args.len() + args.keywords.len();
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(total_args as i64);
                for arg in &args.args {
                    // Unwrap starred expressions
                    match arg {
                        ast::Expr::Starred(starred) => starred.value.serialize(ser),
                        _ => arg.serialize(ser),
                    }
                }
                for kwarg in &args.keywords {
                    kwarg.value.serialize(ser);
                }

                // Serialize argument kinds
                ser.write_tag(TAG_LIST_INT);
                ser.write_int(total_args as i64);
                for arg in &args.args {
                    match arg {
                        ast::Expr::Starred(_) => ser.write_int(ARG_STAR),
                        _ => ser.write_int(ARG_POS),
                    }
                }
                for kwarg in &args.keywords {
                    if kwarg.arg.is_none() {
                        ser.write_int(ARG_STAR2); // **kwargs
                    } else {
                        ser.write_int(ARG_NAMED); // keyword arg
                    }
                }

                // Serialize argument names
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(total_args as i64);
                for _ in &args.args {
                    ser.write_tag(TAG_LITERAL_NONE);
                }
                for kwarg in &args.keywords {
                    if let Some(arg_name) = &kwarg.arg {
                        ser.write_bytes(arg_name.as_bytes());
                    } else {
                        ser.write_tag(TAG_LITERAL_NONE);
                    }
                }

                ser.write_location(c.range());
            }
            ast::Expr::BinOp(e) => {
                ser.write_tag(TAG_OP_EXPR);
                ser.write_tagged_int(e.op as i64);
                e.left.serialize(ser);
                e.right.serialize(ser);
            }
            ast::Expr::NumberLiteral(num) => {
                match &num.value {
                    Number::Int(n) => {
                        match n.as_i64() {
                            Some(x) => {
                                ser.write_tag(TAG_INT_EXPR);
                                ser.write_tagged_int(x);
                            }
                            None => {
                                // Use a string representation for big integers. It's not
                                // very efficient, but these are rare.
                                ser.write_tag(TAG_BIG_INT_EXPR);
                                ser.write_bytes(n.to_string().as_bytes());
                            }
                        }
                    }
                    Number::Float(f) => {
                        ser.write_tag(TAG_FLOAT_EXPR);
                        ser.write_tag(TAG_LITERAL_FLOAT);
                        ser.bytes.extend_from_slice(&f.to_le_bytes());
                    }
                    Number::Complex { real, imag } => {
                        ser.write_tag(TAG_COMPLEX_EXPR);
                        // Serialize real part
                        ser.write_tag(TAG_LITERAL_FLOAT);
                        ser.bytes.extend_from_slice(&real.to_le_bytes());
                        // Serialize imaginary part
                        ser.write_tag(TAG_LITERAL_FLOAT);
                        ser.bytes.extend_from_slice(&imag.to_le_bytes());
                    }
                }
                ser.write_location(num.range());
            }
            ast::Expr::Subscript(e) => {
                ser.write_tag(TAG_INDEX);
                e.value.serialize(ser);
                e.slice.serialize(ser);
                ser.write_location(e.range());
            }
            ast::Expr::List(e) => {
                // In mypy, lists in assignment contexts (Store) are converted to tuples
                // e.g., `a, [b, c] = x, [1, 2]` has `[b, c]` as TupleExpr on LHS
                if matches!(e.ctx, ast::ExprContext::Store) {
                    ser.write_tag(TAG_TUPLE_EXPR);
                } else {
                    ser.write_tag(TAG_LIST_EXPR);
                }
                e.elts.serialize(ser);
                ser.write_location(e.range());
            }
            ast::Expr::Tuple(e) => {
                ser.write_tag(TAG_TUPLE_EXPR);
                e.elts.serialize(ser);
                ser.write_location(e.range());
            }
            ast::Expr::Set(e) => {
                ser.write_tag(TAG_SET_EXPR);
                e.elts.serialize(ser);
                ser.write_location(e.range());
            }
            ast::Expr::Generator(g) => {
                ser.write_tag(TAG_GENERATOR_EXPR);
                serialize_comprehension(ser, &g.elt, &g.generators, g.range());
            }
            ast::Expr::ListComp(lc) => {
                ser.write_tag(TAG_LIST_COMPREHENSION);
                serialize_comprehension(ser, &lc.elt, &lc.generators, lc.range());
            }
            ast::Expr::SetComp(sc) => {
                ser.write_tag(TAG_SET_COMPREHENSION);
                serialize_comprehension(ser, &sc.elt, &sc.generators, sc.range());
            }
            ast::Expr::DictComp(dc) => {
                ser.write_tag(TAG_DICT_COMPREHENSION);
                // Serialize key expression
                dc.key.serialize(ser);
                // Serialize value expression
                dc.value.serialize(ser);
                // Serialize number of generators
                ser.write_tagged_int(dc.generators.len() as i64);
                // Serialize all indices (targets)
                for comp in &dc.generators {
                    comp.target.serialize(ser);
                }
                // Serialize all sequences (iters)
                for comp in &dc.generators {
                    comp.iter.serialize(ser);
                }
                // Serialize all condlists (ifs for each generator)
                for comp in &dc.generators {
                    comp.ifs.serialize(ser);
                }
                // Serialize all is_async flags
                for comp in &dc.generators {
                    ser.write_bool(comp.is_async);
                }
                ser.write_location(dc.range());
            }
            ast::Expr::Yield(y) => {
                ser.write_tag(TAG_YIELD_EXPR);
                // Serialize optional value expression
                y.value.serialize(ser);
                ser.write_location(y.range());
            }
            ast::Expr::YieldFrom(y) => {
                ser.write_tag(TAG_YIELD_FROM_EXPR);
                // Serialize value expression (required for yield from)
                y.value.serialize(ser);
                ser.write_location(y.range());
            }
            ast::Expr::BoolOp(e) => {
                ser.write_tag(TAG_BOOL_OP_EXPR);
                ser.write_tagged_int(match e.op {
                    ast::BoolOp::And => 0,
                    ast::BoolOp::Or => 1,
                });
                e.values.serialize(ser);
                ser.write_location(e.range());
            }
            ast::Expr::Compare(e) => {
                ser.write_tag(TAG_COMPARISON_EXPR);
                e.left.serialize(ser);
                // Serialize operators
                ser.write_tag(TAG_LIST_INT);
                ser.write_usize(e.ops.len());
                for op in &e.ops {
                    ser.write_int(match op {
                        ast::CmpOp::Eq => 0,
                        ast::CmpOp::NotEq => 1,
                        ast::CmpOp::Lt => 2,
                        ast::CmpOp::LtE => 3,
                        ast::CmpOp::Gt => 4,
                        ast::CmpOp::GtE => 5,
                        ast::CmpOp::Is => 6,
                        ast::CmpOp::IsNot => 7,
                        ast::CmpOp::In => 8,
                        ast::CmpOp::NotIn => 9,
                    });
                }
                // Serialize comparators
                e.comparators.serialize(ser);
                ser.write_location(e.range());
            }
            ast::Expr::BooleanLiteral(b) => {
                // Serialize as NameExpr with "True" or "False"
                ser.write_tag(TAG_NAME_EXPR);
                ser.write_bytes(if b.value { b"True" } else { b"False" });
                ser.write_location(b.range());
            }
            ast::Expr::NoneLiteral(n) => {
                // Serialize as NameExpr with "None"
                ser.write_tag(TAG_NAME_EXPR);
                ser.write_bytes(b"None");
                ser.write_location(n.range());
            }
            ast::Expr::EllipsisLiteral(e) => {
                ser.write_tag(TAG_ELLIPSIS_EXPR);
                ser.write_location(e.range());
            }
            ast::Expr::If(i) => {
                ser.write_tag(TAG_CONDITIONAL_EXPR);
                // Serialize if_expr (body - value when condition is true)
                i.body.serialize(ser);
                // Serialize cond (test - the condition)
                i.test.serialize(ser);
                // Serialize else_expr (orelse - value when condition is false)
                i.orelse.serialize(ser);
                ser.write_location(i.range());
            }
            ast::Expr::UnaryOp(u) => {
                ser.write_tag(TAG_UNARY_EXPR);
                // Serialize operator as integer
                ser.write_tagged_int(u.op as i64);
                // Serialize operand
                u.operand.serialize(ser);
                ser.write_location(u.range());
            }
            ast::Expr::Dict(d) => {
                ser.write_tag(TAG_DICT_EXPR);
                // Serialize keys
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(d.items.len() as i64);
                for item in &d.items {
                    if let Some(key) = &item.key {
                        ser.write_bool(true);
                        key.serialize(ser);
                    } else {
                        // Dict unpacking: {**other_dict}
                        ser.write_bool(false);
                    }
                }
                // Serialize values
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(d.items.len() as i64);
                for item in &d.items {
                    item.value.serialize(ser);
                }
                ser.write_location(d.range());
            }
            ast::Expr::Slice(s) => {
                ser.write_tag(TAG_SLICE_EXPR);
                // Serialize lower (begin_index in mypy)
                s.lower.serialize(ser);
                // Serialize upper (end_index in mypy)
                s.upper.serialize(ser);
                // Serialize step (stride in mypy)
                s.step.serialize(ser);
                ser.write_location(s.range());
            }
            ast::Expr::FString(fs) => {
                ser.write_tag(TAG_FSTRING_EXPR);
                ser.write_tagged_int(fs.value.iter().len() as i64);
                for part in fs.value.iter() {
                    match part {
                        ast::FStringPart::FString(fstring_part) => {
                            ser.write_bool(true);
                            serialize_fstring_elements(ser, fstring_part.elements.iter().collect());
                        }
                        ast::FStringPart::Literal(lit) => {
                            ser.write_bool(false);
                            ser.write_bytes(lit.value.as_bytes());
                            ser.write_location(lit.range());
                        }
                    }
                }
                ser.write_location(fs.range());
            }
            ast::Expr::Lambda(lambda) => {
                ser.write_tag(TAG_LAMBDA_EXPR);

                // Arguments (parameters)
                if let Some(params) = &lambda.parameters {
                    serialize_parameters(ser, params);
                } else {
                    // No parameters - empty argument list
                    ser.write_tag(TAG_LIST_GEN);
                    ser.write_int(0);
                }

                // Body - lambda body is a single expression, wrap in return statement
                // Serialize as a block containing a single return statement
                ser.write_tag(TAG_BLOCK);
                ser.write_tag(TAG_LIST_GEN);
                ser.write_int(1); // One statement (the return)
                ser.write_bool(ser.current_unreachable); // Write unreachable flag

                ser.write_tag(TAG_RETURN);
                // Return statement has an optional value, we always have a value for lambda
                ser.write_bool(true);
                lambda.body.serialize(ser);
                ser.write_location(lambda.body.range());
                ser.write_end_tag(); // End of return statement

                ser.write_end_tag(); // End of block

                ser.write_location(lambda.range());
            }
            ast::Expr::Named(named) => {
                ser.write_tag(TAG_NAMED_EXPR);
                // Serialize target expression
                named.target.serialize(ser);
                // Serialize value expression
                named.value.serialize(ser);
                ser.write_location(named.range());
            }
            ast::Expr::Starred(starred) => {
                ser.write_tag(TAG_STAR_EXPR);
                // Serialize the wrapped expression
                starred.value.serialize(ser);
                ser.write_location(starred.range());
            }
            ast::Expr::BytesLiteral(bytes_lit) => {
                ser.write_tag(TAG_BYTES_EXPR);
                // Convert bytes to string representation with escape sequences
                let result = serialize_bytes_to_escaped_string(bytes_lit);
                ser.write_bytes(&result);
                ser.write_location(bytes_lit.range());
            }
            ast::Expr::Await(await_expr) => {
                ser.write_tag(TAG_AWAIT_EXPR);
                // Serialize the awaited expression
                await_expr.value.serialize(ser);
                ser.write_location(await_expr.range());
            }
            _ => {
                panic!("unsupported: {self:?}");
            }
        };
        ser.write_end_tag()
    }
}

impl Ser for ast::Pattern {
    fn serialize(&self, ser: &mut Serializer) {
        match self {
            ast::Pattern::MatchAs(p) => {
                ser.write_tag(TAG_AS_PATTERN);
                // Serialize optional pattern
                if let Some(pattern) = &p.pattern {
                    ser.write_bool(true);
                    pattern.serialize(ser);
                } else {
                    ser.write_bool(false);
                }
                // Serialize optional name
                if let Some(name) = &p.name {
                    ser.write_bool(true);
                    ser.write_bytes(name.as_bytes());
                    ser.write_location(name.range);
                } else {
                    ser.write_bool(false);
                }
                ser.write_location(p.range());
            }
            ast::Pattern::MatchOr(p) => {
                ser.write_tag(TAG_OR_PATTERN);
                // Write number of patterns
                ser.write_tagged_int(p.patterns.len() as i64);
                // Serialize each pattern
                for pattern in &p.patterns {
                    pattern.serialize(ser);
                }
                ser.write_location(p.range());
            }
            ast::Pattern::MatchValue(p) => {
                ser.write_tag(TAG_VALUE_PATTERN);
                // Serialize value expression
                p.value.serialize(ser);
                ser.write_location(p.range());
            }
            ast::Pattern::MatchSingleton(p) => {
                ser.write_tag(TAG_SINGLETON_PATTERN);
                // Serialize singleton value (True, False, or None)
                match p.value {
                    ast::Singleton::True => ser.write_bool(true),
                    ast::Singleton::False => ser.write_bool(false),
                    ast::Singleton::None => {
                        // Special marker for None
                        ser.write_tag(TAG_LITERAL_NONE);
                    }
                }
                ser.write_location(p.range());
            }
            ast::Pattern::MatchSequence(p) => {
                ser.write_tag(TAG_SEQUENCE_PATTERN);
                // Write number of patterns
                ser.write_tagged_int(p.patterns.len() as i64);
                // Serialize each pattern
                for pattern in &p.patterns {
                    pattern.serialize(ser);
                }
                ser.write_location(p.range());
            }
            ast::Pattern::MatchStar(p) => {
                ser.write_tag(TAG_STARRED_PATTERN);
                // Serialize optional capture name
                if let Some(name) = &p.name {
                    ser.write_bool(true);
                    ser.write_bytes(name.as_bytes());
                    ser.write_location(name.range);
                } else {
                    ser.write_bool(false);
                }
                ser.write_location(p.range());
            }
            ast::Pattern::MatchMapping(p) => {
                ser.write_tag(TAG_MAPPING_PATTERN);
                // Write number of key-value pairs
                ser.write_tagged_int(p.keys.len() as i64);
                // Serialize keys and patterns
                for (key, pattern) in p.keys.iter().zip(&p.patterns) {
                    key.serialize(ser);
                    pattern.serialize(ser);
                }
                // Serialize optional rest pattern
                if let Some(rest) = &p.rest {
                    ser.write_bool(true);
                    ser.write_bytes(rest.as_bytes());
                    ser.write_location(rest.range);
                } else {
                    ser.write_bool(false);
                }
                ser.write_location(p.range());
            }
            ast::Pattern::MatchClass(p) => {
                ser.write_tag(TAG_CLASS_PATTERN);
                // Serialize class reference
                p.cls.serialize(ser);
                // Write number of positional patterns
                ser.write_tagged_int(p.arguments.patterns.len() as i64);
                // Serialize positional patterns
                for pattern in &p.arguments.patterns {
                    pattern.serialize(ser);
                }
                // Write number of keyword patterns
                ser.write_tagged_int(p.arguments.keywords.len() as i64);
                // Serialize keyword patterns
                for keyword in &p.arguments.keywords {
                    ser.write_bytes(keyword.attr.as_bytes());
                    keyword.pattern.serialize(ser);
                }
                ser.write_location(p.range());
            }
        };
        ser.write_end_tag()
    }
}

fn serialize_fstring_elements(ser: &mut Serializer, elems: Vec<&ast::InterpolatedStringElement>) {
    ser.write_tagged_int(elems.len() as i64);
    for elem in elems {
        match elem {
            ast::InterpolatedStringElement::Literal(lit) => {
                ser.write_bytes(lit.value.as_bytes());
                ser.write_location(lit.range());
            }
            ast::InterpolatedStringElement::Interpolation(interp) => {
                ser.write_tag(TAG_FSTRING_INTERPOLATION);
                interp.expression.serialize(ser);
                match interp.conversion {
                    ast::ConversionFlag::None => {
                        ser.write_bool(false);
                    }
                    ast::ConversionFlag::Str => {
                        // !s conversion: f"{name!s}"
                        ser.write_bool(true);
                        ser.write_bytes(b"!s");
                    }
                    ast::ConversionFlag::Repr => {
                        // !r conversion: f"{name!r}"
                        ser.write_bool(true);
                        ser.write_bytes(b"!r");
                    }
                    ast::ConversionFlag::Ascii => {
                        // !a conversion: f"{name!a}"
                        ser.write_bool(true);
                        ser.write_bytes(b"!a");
                    }
                }
                if let Some(format_spec) = &interp.format_spec {
                    ser.write_bool(true);
                    serialize_fstring_elements(ser, format_spec.elements.iter().collect());
                    ser.write_location(format_spec.range());
                } else {
                    ser.write_bool(false);
                }
                ser.write_end_tag();
            }
        }
    }
}

// ============================================================================
// Type Serialization Functions
// ============================================================================

/// Helper to serialize an invalid type annotation as RawExpressionType with typing.Any.
/// This is used for expressions that are not valid in type contexts (e.g., 3.14, int + str).
fn serialize_invalid_type(ser: &mut Serializer) {
    ser.write_tag(TAG_RAW_EXPRESSION_TYPE);
    ser.write_bytes(b"typing.Any");
    ser.write_tag(TAG_LITERAL_NONE);
}

/// Main entry point for serializing type annotations.
/// Handles all Python type expressions including names, attributes, subscripts,
/// unions, literals, and forward references (string literals).
fn serialize_type(ser: &mut Serializer, t: &ast::Expr) {
    match t {
        ast::Expr::Name(e) => {
            serialize_simple_unbound_type(ser, e.id.as_bytes());
        }
        ast::Expr::Attribute(_e) => {
            serialize_attribute_type(ser, t, None, None);
        }
        ast::Expr::Subscript(e) => {
            serialize_subscript_type(ser, e, None, None);
        }
        ast::Expr::NoneLiteral(_) => {
            serialize_simple_unbound_type(ser, b"None");
        }
        ast::Expr::BooleanLiteral(b) => {
            // Serialize as RawExpressionType with bool value
            ser.write_tag(TAG_RAW_EXPRESSION_TYPE);
            ser.write_bytes(b"builtins.bool");
            ser.write_bool(b.value);
        }
        ast::Expr::NumberLiteral(n) => {
            // Serialize integer literals as RawExpressionType with int value
            if n.value.is_int() {
                if let Some(int_val) = extract_int_literal_value(t) {
                    ser.write_tag(TAG_RAW_EXPRESSION_TYPE);
                    ser.write_bytes(b"builtins.int");
                    ser.write_tagged_int(int_val);
                } else {
                    // Integer too large for i64 - serialize as invalid type
                    serialize_invalid_type(ser);
                }
            } else {
                // Float/complex number literals are not valid in type annotations
                // Serialize as invalid type
                serialize_invalid_type(ser);
            }
        }
        ast::Expr::BinOp(e) => {
            // Handle union types (x | y)
            if matches!(e.op, ast::Operator::BitOr) {
                serialize_union_type(ser, e, t.range(), None, None);
                return;
            } else {
                // Other binary operators are not valid in type annotations
                // Serialize as invalid type
                serialize_invalid_type(ser);
            }
        }
        ast::Expr::List(e) => {
            ser.write_tag(TAG_LIST_TYPE);
            // Serialize items list
            ser.write_tag(TAG_LIST_GEN);
            ser.write_int(e.elts.len() as i64);
            for item in &e.elts {
                serialize_type(ser, item);
            }
        }
        ast::Expr::Tuple(e) => {
            ser.write_tag(TAG_TUPLE_TYPE);
            // Serialize items list
            ser.write_tag(TAG_LIST_GEN);
            ser.write_int(e.elts.len() as i64);
            for item in &e.elts {
                serialize_type(ser, item);
            }
            // Write implicit = True (i.e. from (T, S) syntax)
            ser.write_bool(true);
        }
        ast::Expr::Call(c) => {
            // Handle Call in type context (e.g., Arg(int, 'x'))
            ser.write_tag(TAG_CALL_TYPE);

            // Serialize callee
            serialize_type(ser, &c.func);

            // Serialize positional arguments
            ser.write_tag(TAG_LIST_GEN);
            ser.write_int(c.arguments.args.len() as i64);
            for arg in &c.arguments.args {
                serialize_type(ser, arg);
            }

            // Serialize keyword arguments (name, value pairs)
            ser.write_tag(TAG_LIST_GEN);
            ser.write_int(c.arguments.keywords.len() as i64);
            for keyword in &c.arguments.keywords {
                // Write keyword name (could be None for **kwargs)
                if let Some(name) = &keyword.arg {
                    ser.write_bytes(name.as_bytes());
                } else {
                    ser.write_tag(TAG_LITERAL_NONE);
                }
                // Write keyword value
                serialize_type(ser, &keyword.value);
            }
        }
        ast::Expr::EllipsisLiteral(_) => {
            ser.write_tag(TAG_ELLIPSIS_TYPE);
            // EllipsisType has no attributes
        }
        ast::Expr::Starred(e) => {
            ser.write_tag(TAG_UNPACK_TYPE);
            serialize_type(ser, &e.value);
            // Write from_star_syntax flag (true for * syntax, false for Unpack[...])
            ser.write_bool(true);
        }
        ast::Expr::UnaryOp(e) => {
            // Handle unary operators on integer literals in types
            if matches!(e.op, ast::UnaryOp::USub) {
                // Negative integer literal (e.g., Literal[-1])
                if let Some(int_val) = extract_int_literal_value(t) {
                    ser.write_tag(TAG_RAW_EXPRESSION_TYPE);
                    ser.write_bytes(b"builtins.int");
                    ser.write_tagged_int(int_val);
                    // Return early since we've already written location and end tag below
                    ser.write_location(e.range());
                    ser.write_end_tag();
                    return;
                } else {
                    // Negative number too large - serialize as invalid type
                    serialize_invalid_type(ser);
                }
            } else if matches!(e.op, ast::UnaryOp::UAdd) {
                // Positive unary operator (+) - preserve the underlying value
                // This matches fastparse.py behavior where +5 returns the value unchanged
                if let Some(int_val) = extract_int_literal_value(&e.operand) {
                    ser.write_tag(TAG_RAW_EXPRESSION_TYPE);
                    ser.write_bytes(b"builtins.int");
                    ser.write_tagged_int(int_val);
                    // Return early since we've already written location and end tag below
                    ser.write_location(e.range());
                    ser.write_end_tag();
                    return;
                } else {
                    // Number too large or not an integer - serialize as invalid type
                    serialize_invalid_type(ser);
                }
            } else {
                // Other unary operators (not, ~, etc.) are not valid in type annotations
                // Serialize as invalid type
                serialize_invalid_type(ser);
            }
        }
        ast::Expr::StringLiteral(s) => {
            // String literals in type context (forward references or Literal["x"])
            // Extract the string value (concatenate all parts)
            let mut string_value = String::new();
            for part in &s.value {
                string_value.push_str(part.as_str());
            }

            // Try to parse the string as a type expression
            serialize_string_type(ser, &string_value, s.range());
            return; // serialize_string_type handles location and end tag
        }
        ast::Expr::BytesLiteral(bytes_lit) => {
            // Bytes literals in type context (e.g., Literal[b"foo"])
            // Unlike string literals, bytes literals can't be used for forward references,
            // so they're treated similar to integer literals.
            // We serialize the bytes as an escaped string (similar to BytesExpr)
            ser.write_tag(TAG_RAW_EXPRESSION_TYPE);
            ser.write_bytes(b"builtins.bytes");
            let result = serialize_bytes_to_escaped_string(bytes_lit);
            ser.write_bytes(&result);
        }
        _ => {
            // Unsupported expression type in type annotation
            // Serialize as invalid type
            serialize_invalid_type(ser);
        }
    }
    ser.write_location(t.range());
    ser.write_end_tag();
}

/// Parse and serialize a string literal that appears in a type context.
/// This handles forward references like `x: "int"` and string literals in Literal types.
fn serialize_string_type(ser: &mut Serializer, string_value: &str, range: TextRange) {
    // Try to parse the string as a type expression, similar to fastparse.py's parse_type_string
    // We wrap it in parentheses to parse it as an expression
    let wrapped = format!("({})", string_value);
    let parse_result = parse_unchecked(&wrapped, ParseOptions::from(Mode::Expression));

    // Check if parsing succeeded and we got a valid type expression
    if parse_result.errors().is_empty() {
        // Extract the expression from the parsed module
        if let ast::Mod::Expression(expr_mod) = parse_result.into_syntax() {
            let expr = expr_mod.body;

            // Check if this is a type expression that should have original_str_expr set
            // (UnboundType or UnionType in mypy terms, which are Name/Attribute/Subscript or BinOp with | in AST)
            match expr.as_ref() {
                ast::Expr::Name(e) => {
                    ser.write_tag(TAG_UNBOUND_TYPE);
                    ser.write_bytes(e.id.as_bytes());
                    ser.write_tag(TAG_LIST_GEN);
                    ser.write_int(0);
                    // Write empty_tuple_index
                    ser.write_bool(false);
                    // Write original_str_expr
                    ser.write_bytes(string_value.as_bytes());
                    // Write original_str_fallback
                    ser.write_bytes(b"builtins.str");
                    ser.write_location(range);
                    ser.write_end_tag();
                    return;
                }
                ast::Expr::Attribute(_e) => {
                    serialize_attribute_type(
                        ser,
                        expr.as_ref(),
                        Some(string_value),
                        Some("builtins.str"),
                    );
                    ser.write_location(range);
                    ser.write_end_tag();
                    return;
                }
                ast::Expr::Subscript(e) => {
                    serialize_subscript_type(ser, e, Some(string_value), Some("builtins.str"));
                    ser.write_location(range);
                    ser.write_end_tag();
                    return;
                }
                ast::Expr::BinOp(binop) if matches!(binop.op, ast::Operator::BitOr) => {
                    // Serialize as UnionType with original_str_expr and original_str_fallback
                    serialize_union_type(
                        ser,
                        binop,
                        range,
                        Some(string_value),
                        Some("builtins.str"),
                    );
                    return;
                }
                _ => {
                    // Other expressions - serialize as RawExpressionType
                }
            }
        }
    }

    // If parsing failed or resulted in non-type expression, serialize as RawExpressionType
    ser.write_tag(TAG_RAW_EXPRESSION_TYPE);
    ser.write_bytes(b"builtins.str");
    ser.write_bytes(string_value.as_bytes());
    ser.write_location(range);
    ser.write_end_tag();
}

/// Serialize a union type (BinOp with |) with optional original_str_expr and original_str_fallback.
fn serialize_union_type(
    ser: &mut Serializer,
    binop: &ast::ExprBinOp,
    range: TextRange,
    original_str_expr: Option<&str>,
    original_str_fallback: Option<&str>,
) {
    ser.write_tag(TAG_UNION_TYPE);
    // Serialize items list with exactly two items (left and right)
    ser.write_tag(TAG_LIST_GEN);
    ser.write_int(2);
    serialize_type(ser, &binop.left);
    serialize_type(ser, &binop.right);
    // uses_pep604_syntax = true (using | operator)
    ser.write_bool(true);
    // Write optional original_str_expr
    if let Some(s) = original_str_expr {
        ser.write_bytes(s.as_bytes());
    } else {
        ser.write_tag(TAG_LITERAL_NONE);
    }
    // Write optional original_str_fallback
    if let Some(s) = original_str_fallback {
        ser.write_bytes(s.as_bytes());
    } else {
        ser.write_tag(TAG_LITERAL_NONE);
    }
    ser.write_location(range);
    ser.write_end_tag();
}

/// Serialize an Attribute type (e.g., foo.bar.Baz) with optional original_str_expr.
fn serialize_attribute_type(
    ser: &mut Serializer,
    expr: &ast::Expr,
    original_str_expr: Option<&str>,
    original_str_fallback: Option<&str>,
) {
    let mut v = Vec::new();
    if !get_qualified_type_name(&mut v, expr) {
        // Invalid expression for qualified name - serialize as invalid type
        serialize_invalid_type(ser);
        return;
    }
    ser.write_tag(TAG_UNBOUND_TYPE);
    ser.write_bytes(&v);
    ser.write_tag(TAG_LIST_GEN);
    ser.write_int(0);
    // Write empty_tuple_index
    ser.write_bool(false);
    // Write optional original_str_expr
    if let Some(s) = original_str_expr {
        ser.write_bytes(s.as_bytes());
    } else {
        ser.write_tag(TAG_LITERAL_NONE);
    }
    // Write optional original_str_fallback
    if let Some(s) = original_str_fallback {
        ser.write_bytes(s.as_bytes());
    } else {
        ser.write_tag(TAG_LITERAL_NONE);
    }
}

/// Serialize a Subscript type (e.g., List[int], Dict[str, int]) with optional original_str_expr.
fn serialize_subscript_type(
    ser: &mut Serializer,
    subscript: &ast::ExprSubscript,
    original_str_expr: Option<&str>,
    original_str_fallback: Option<&str>,
) {
    let mut v = Vec::new();
    if !get_qualified_type_name(&mut v, &subscript.value) {
        // Invalid expression for qualified name - serialize as invalid type
        serialize_invalid_type(ser);
        return;
    }
    ser.write_tag(TAG_UNBOUND_TYPE);
    ser.write_bytes(&v);
    ser.write_tag(TAG_LIST_GEN);
    match subscript.slice.as_ref() {
        ast::Expr::Tuple(t) => {
            ser.write_usize(t.len());
            for item in &t.elts {
                serialize_type(ser, item);
            }
            // Write empty_tuple_index.
            ser.write_bool(t.len() == 0);
        }
        _ => {
            ser.write_int(1);
            serialize_type(ser, &subscript.slice);
            ser.write_bool(false);
        }
    }
    // Write optional original_str_expr
    if let Some(s) = original_str_expr {
        ser.write_bytes(s.as_bytes());
    } else {
        ser.write_tag(TAG_LITERAL_NONE);
    }
    // Write optional original_str_fallback
    if let Some(s) = original_str_fallback {
        ser.write_bytes(s.as_bytes());
    } else {
        ser.write_tag(TAG_LITERAL_NONE);
    }
}

/// Serialize a simple unbound type (just a name like `int` or `None`).
fn serialize_simple_unbound_type(ser: &mut Serializer, name: &[u8]) {
    ser.write_tag(TAG_UNBOUND_TYPE);
    ser.write_bytes(name);
    ser.write_tag(TAG_LIST_GEN);
    ser.write_int(0);
    // Write empty_tuple_index
    ser.write_bool(false);
    // Write None for original_str_expr (optional field)
    ser.write_tag(TAG_LITERAL_NONE);
    // Write None for original_str_fallback (optional field)
    ser.write_tag(TAG_LITERAL_NONE);
}

/// Helper to build a qualified type name from nested attributes (e.g., `foo.bar.Baz`).
/// Returns true if successful, false if the expression is not a valid qualified name.
fn get_qualified_type_name(v: &mut Vec<u8>, e: &ast::Expr) -> bool {
    match e {
        ast::Expr::Name(e) => {
            v.extend_from_slice(e.id.as_bytes());
            true
        }
        ast::Expr::Attribute(e) => {
            if get_qualified_type_name(v, &e.value) {
                v.extend_from_slice(b".");
                v.extend_from_slice(e.attr.as_bytes());
                true
            } else {
                false
            }
        }
        _ => {
            // Not a valid qualified name - caller should handle this
            false
        }
    }
}

/// Extract an integer literal value from a type expression, handling both
/// positive literals (NumberLiteral) and negative literals (UnaryOp(USub, NumberLiteral)).
fn extract_int_literal_value(expr: &ast::Expr) -> Option<i64> {
    match expr {
        ast::Expr::NumberLiteral(n) => {
            if let Number::Int(int_val) = &n.value {
                int_val.as_i64()
            } else {
                None
            }
        }
        ast::Expr::UnaryOp(e) if matches!(e.op, ast::UnaryOp::USub) => {
            // Recursively extract value from operand and negate it
            extract_int_literal_value(&e.operand).map(|v| -v)
        }
        _ => None,
    }
}

/// Build import flags from current serializer state
fn make_import_flags(ser: &Serializer) -> u8 {
    (if !ser.in_function { IMPORT_FLAG_TOP_LEVEL } else { 0 })
        | (if ser.current_unreachable { IMPORT_FLAG_UNREACHABLE } else { 0 })
        | (if ser.current_mypy_only { IMPORT_FLAG_MYPY_ONLY } else { 0 })
}

/// Serialize a list of import statements to bytes.
///
/// # Arguments
///
/// * `imports` - List of import statements to serialize
/// * `text` - Source text (used for creating LineIndex to serialize ranges)
/// * `line_index` - Optional pre-computed LineIndex (if None, will be computed from text)
/// * `is_all_ascii` - Optional pre-computed ASCII flag (if None, will be computed from text)
/// * `lines_with_non_ascii` - Optional pre-computed per-line non-ASCII flags (if None, will be computed from text)
///
/// # Returns
///
/// Serialized bytes representing the imports
pub fn serialize_imports(
    imports: &[ImportStatement],
    text: &str,
    line_index: Option<LineIndex>,
    is_all_ascii: Option<bool>,
    lines_with_non_ascii: Option<Vec<bool>>,
) -> Vec<u8> {
    let line_index = line_index.unwrap_or_else(|| LineIndex::from_source_text(text));
    let is_all_ascii = is_all_ascii.unwrap_or_else(|| text.is_ascii());
    let lines_with_non_ascii = lines_with_non_ascii.unwrap_or_else(|| {
        if is_all_ascii {
            Vec::new()
        } else {
            text.lines().map(|line| !line.is_ascii()).collect()
        }
    });

    let mut ser = Serializer {
        bytes: Vec::new(),
        imports: Vec::new(),
        line_index,
        text,
        skip_function_bodies: false,
        in_class: false,
        in_function: false,
        is_all_ascii,
        lines_with_non_ascii,
        type_comments: HashMap::new(),
        options: Options::default(),
        current_unreachable: false,
        current_mypy_only: false,
        top_level_getattr: false,
    };

    // Write list of imports
    ser.write_tag(TAG_LIST_GEN);
    ser.write_usize(imports.len());

    for import in imports {
        match import {
            ImportStatement::Import {
                name,
                relative,
                as_name,
                range,
                flags,
            } => {
                ser.write_tag(TAG_IMPORT_METADATA);
                ser.write_bytes(name.as_bytes());
                ser.write_tagged_int(*relative as i64);
                if let Some(asname) = as_name {
                    ser.write_bool(true);
                    ser.write_bytes(asname.as_bytes());
                } else {
                    ser.write_bool(false);
                }
                ser.write_location(*range);
                ser.write_tagged_int(*flags as i64);
            }
            ImportStatement::ImportFrom {
                module,
                relative,
                names,
                range,
                flags,
            } => {
                ser.write_tag(TAG_IMPORTFROM_METADATA);
                ser.write_bytes(module.as_bytes());
                ser.write_tagged_int(*relative as i64);

                // Write list of (name, as_name) tuples
                ser.write_tag(TAG_LIST_GEN);
                ser.write_usize(names.len());
                for (name, as_name) in names {
                    ser.write_bytes(name.as_bytes());
                    if let Some(asname) = as_name {
                        ser.write_bool(true);
                        ser.write_bytes(asname.as_bytes());
                    } else {
                        ser.write_bool(false);
                    }
                }

                ser.write_location(*range);
                ser.write_tagged_int(*flags as i64);
            }
            ImportStatement::ImportAll {
                module,
                relative,
                range,
                flags,
            } => {
                ser.write_tag(TAG_IMPORTALL_METADATA);
                ser.write_bytes(module.as_bytes());
                ser.write_tagged_int(*relative as i64);
                ser.write_location(*range);
                ser.write_tagged_int(*flags as i64);
            }
        }
    }

    ser.bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_val(x: i64) -> u8 {
        return ((x - MIN_SHORT_INT) << 1) as u8;
    }

    fn parse_expr_for_test(expr: &str) -> ast::Expr {
        let parsed = parse_unchecked(expr, ParseOptions::from(Mode::Expression));
        assert!(
            parsed.errors().is_empty(),
            "failed to parse test expression: {expr}"
        );
        let ast::Mod::Expression(expr_mod) = parsed.into_syntax() else {
            panic!("expected expression AST for test input: {expr}");
        };
        *expr_mod.body
    }

    fn analyze_if_chain(condition_exprs: &[&str]) -> (Vec<(bool, bool)>, (bool, bool)) {
        let options = Options::default();
        let mut analyzer = IfReachabilityAnalyzer::new(&options);
        let mut condition_flags = Vec::with_capacity(condition_exprs.len());

        for expr in condition_exprs {
            let parsed = parse_expr_for_test(expr);
            condition_flags.push(analyzer.condition_flags(&parsed));
        }

        let else_flags = analyzer.else_flags();
        (condition_flags, else_flags)
    }

    fn make_ser<'a>(text: &'a str) -> Serializer<'a> {
        let index = LineIndex::from_source_text(text);
        let is_all_ascii = text.is_ascii();
        let lines_with_non_ascii = if is_all_ascii {
            Vec::new()
        } else {
            text.lines().map(|line| !line.is_ascii()).collect()
        };
        Serializer {
            bytes: Vec::new(),
            imports: Vec::new(),
            line_index: index,
            text,
            skip_function_bodies: false,
            in_class: false,
            in_function: false,
            is_all_ascii,
            lines_with_non_ascii,
            type_comments: HashMap::new(),
            options: Options::default(),
            current_unreachable: false,
            current_mypy_only: false,
            top_level_getattr: false,
        }
    }

    #[test]
    fn test_if_reachability_analyzer_flags() {
        struct Case {
            name: &'static str,
            conditions: &'static [&'static str],
            expected_condition_flags: &'static [(bool, bool)],
            expected_else_flags: (bool, bool),
        }

        let cases = [
            Case {
                name: "all_unknown",
                conditions: &["x", "y"],
                expected_condition_flags: &[(false, false), (false, false)],
                expected_else_flags: (false, false),
            },
            Case {
                name: "always_true_makes_tail_unreachable",
                conditions: &["x", "PY3", "z"],
                expected_condition_flags: &[(false, false), (false, false), (true, false)],
                expected_else_flags: (true, false),
            },
            Case {
                name: "mypy_true_is_mypy_only_then_closes_tail",
                conditions: &["MYPY", "z"],
                expected_condition_flags: &[(false, true), (true, false)],
                expected_else_flags: (true, false),
            },
            Case {
                name: "mypy_false_makes_else_mypy_only",
                conditions: &["x", "not MYPY"],
                expected_condition_flags: &[(false, false), (true, false)],
                expected_else_flags: (false, true),
            },
        ];

        for case in cases {
            let (condition_flags, else_flags) = analyze_if_chain(case.conditions);
            assert_eq!(
                condition_flags.as_slice(),
                case.expected_condition_flags,
                "condition flags mismatch for {}",
                case.name
            );
            assert_eq!(
                else_flags, case.expected_else_flags,
                "else flags mismatch for {}",
                case.name
            );
        }
    }

    #[test]
    fn test_write_short_int() {
        for x in [-10, -1, 0, 1, 117] {
            let mut ser = make_ser("");
            ser.write_int(x);
            assert_eq!(ser.bytes, &[((x - MIN_SHORT_INT) << 1) as u8]);
        }
    }

    #[test]
    fn test_write_2_byte_int() {
        let mut ser = make_ser("");
        ser.write_int(118);
        assert_eq!(ser.bytes, &[105, 3]);

        let mut ser = make_ser("");
        ser.write_int(-11);
        assert_eq!(ser.bytes, &[101, 1]);

        let mut ser = make_ser("");
        ser.write_int(-100);
        assert_eq!(ser.bytes, &[1, 0]);

        let mut ser = make_ser("");
        ser.write_int(16283);
        assert_eq!(ser.bytes, &[253, 255]);
    }

    #[test]
    fn test_write_4_byte_int() {
        let mut ser = make_ser("");
        ser.write_int(-101);
        assert_eq!(ser.bytes, &[91, 53, 1, 0]);

        let mut ser = make_ser("");
        ser.write_int(16284);
        assert_eq!(ser.bytes, &[99, 53, 3, 0]);

        let mut ser = make_ser("");
        ser.write_int(-10000);
        assert_eq!(ser.bytes, &[3, 0, 0, 0]);

        let mut ser = make_ser("");
        ser.write_int(536860911);
        assert_eq!(ser.bytes, &[251, 255, 255, 255]);
    }

    #[test]
    fn test_write_long_int() {
        let mut ser = make_ser("");
        ser.write_int(-10001);
        assert_eq!(ser.bytes, &[15, 30, 17, 39]);

        let mut ser = make_ser("");
        ser.write_int(536860912);
        assert_eq!(ser.bytes, &[15, 36, 240, 216, 255, 31]);
    }

    #[test]
    fn test_unicode_support() {
        // Test that we can parse and serialize files with Unicode characters
        let text = "# Comment with 中文\ndef привет():\n    x = \"🎉\"\n";
        let opt = ParseOptions::from(PySourceType::Python);
        let ast = parse_unchecked(text, opt).into_syntax();
        let mut ser = make_ser(text);

        // Should not panic
        ast.serialize(&mut ser);

        // Verify that is_all_ascii is correctly set
        assert!(!ser.is_all_ascii);

        // Verify that lines_with_non_ascii is correctly populated
        assert_eq!(ser.lines_with_non_ascii.len(), 3);
        assert!(ser.lines_with_non_ascii[0]); // Line with Chinese
        assert!(ser.lines_with_non_ascii[1]); // Line with Cyrillic
        assert!(ser.lines_with_non_ascii[2]); // Line with emoji
    }

    #[test]
    fn test_mixed_ascii_unicode() {
        // Test file with some ASCII and some Unicode lines
        let text = "# ASCII comment\ndef hello():\n    x = \"мир\"\n    y = 42\n";
        let ser = make_ser(text);

        assert!(!ser.is_all_ascii);
        assert_eq!(ser.lines_with_non_ascii.len(), 4);
        assert!(!ser.lines_with_non_ascii[0]); // ASCII line
        assert!(!ser.lines_with_non_ascii[1]); // ASCII line
        assert!(ser.lines_with_non_ascii[2]); // Unicode line
        assert!(!ser.lines_with_non_ascii[3]); // ASCII line
    }

    #[test]
    fn test_all_ascii_optimization() {
        // Test that all-ASCII files use the optimized path
        let text = "def hello():\n    return 42\n";
        let ser = make_ser(text);

        assert!(ser.is_all_ascii);
        assert!(ser.lines_with_non_ascii.is_empty()); // No per-line tracking
    }

    #[test]
    fn test_unicode_with_crlf_line_endings() {
        // Test that Unicode handling works correctly with Windows (CRLF) line endings
        let text = "# Comment with 中文\r\ndef привет():\r\n    x = \"🎉\"\r\n";
        let opt = ParseOptions::from(PySourceType::Python);
        let ast = parse_unchecked(text, opt).into_syntax();
        let mut ser = make_ser(text);

        // Should not panic with CRLF line endings
        ast.serialize(&mut ser);

        // Verify that is_all_ascii is correctly set
        assert!(!ser.is_all_ascii);

        // Verify that lines_with_non_ascii is correctly populated
        assert_eq!(ser.lines_with_non_ascii.len(), 3);
        assert!(ser.lines_with_non_ascii[0]); // Line with Chinese
        assert!(ser.lines_with_non_ascii[1]); // Line with Cyrillic
        assert!(ser.lines_with_non_ascii[2]); // Line with emoji
    }

    #[test]
    fn test_mixed_crlf_and_lf() {
        // Test files with mixed line endings (both CRLF and LF)
        let text = "# ASCII with CRLF\r\ndef hello():\n    x = \"мир\"\r\n    y = 42\n";
        let ser = make_ser(text);

        assert!(!ser.is_all_ascii);
        assert_eq!(ser.lines_with_non_ascii.len(), 4);
        assert!(!ser.lines_with_non_ascii[0]); // ASCII line with CRLF
        assert!(!ser.lines_with_non_ascii[1]); // ASCII line with LF
        assert!(ser.lines_with_non_ascii[2]); // Unicode line with CRLF
        assert!(!ser.lines_with_non_ascii[3]); // ASCII line with LF
    }

    #[test]
    fn print_hello() {
        let opt = ParseOptions::from(PySourceType::Python);
        let text = "print('hello')";
        let ast = parse_unchecked(text, opt).into_syntax();
        let mut ser = make_ser(text);
        ast.serialize(&mut ser);
        let _ = ser; // TODO: drop when not needed

        let expected = &[
            TAG_LITERAL_INT,
            int_val(1),
            TAG_EXPR_STMT,
            TAG_CALL_EXPR,
            TAG_NAME_EXPR,
            TAG_LITERAL_STR,
            int_val(5),
            b'p',
            b'r',
            b'i',
            b'n',
            b't',
            TAG_LOCATION,
            int_val(1),
            int_val(0),
            int_val(0),
            int_val(5),
            TAG_END,
            TAG_LIST_GEN,
            int_val(1),
            TAG_STR_EXPR,
            TAG_LITERAL_STR,
            int_val(5),
            b'h',
            b'e',
            b'l',
            b'l',
            b'o',
            TAG_LOCATION,
            int_val(1),
            int_val(6),
            int_val(0),
            int_val(7),
            TAG_END,
            TAG_LIST_INT,
            int_val(1),
            int_val(0), // ARG_POS
            TAG_LIST_GEN,
            int_val(1),
            TAG_LITERAL_NONE,
            TAG_LOCATION,
            int_val(1),
            int_val(0),
            int_val(0),
            int_val(14),
            TAG_END,
            TAG_END,
        ];

        assert_eq!(ser.bytes, expected);
    }

    #[test]
    fn test_serialize_single_import() {
        // Create a simple import: "import os" at line 1, columns 0-9
        let text = "import os\n";
        let imports = vec![ImportStatement::Import {
            name: "os".to_string(),
            relative: 0,
            as_name: None,
            range: TextRange::new(0.into(), 9.into()),
            flags: IMPORT_FLAG_TOP_LEVEL,
        }];

        let bytes = serialize_imports(&imports, text, None, None, None);

        // Expected byte sequence:
        // TAG_LIST_GEN (20) + length (1)
        // TAG_IMPORT_METADATA (226)
        // name: TAG_LITERAL_STR (4) + length (2) + "os"
        // relative: TAG_LITERAL_INT (3) + int_val(0)
        // as_name: TAG_LITERAL_FALSE (0)
        // range: TAG_LOCATION (152) + line (1) + col (0) + line_diff (0) + col_diff (9)
        // Note: write_location writes start line, start col, line difference, col difference
        // flags: TAG_LITERAL_INT (3) + int_val(1) - IMPORT_FLAG_TOP_LEVEL set
        let expected = vec![
            TAG_LIST_GEN,
            int_val(1), // list length = 1
            TAG_IMPORT_METADATA,
            TAG_LITERAL_STR,
            int_val(2), // "os" length
            b'o',
            b's',
            TAG_LITERAL_INT,
            int_val(0), // relative = 0
            TAG_LITERAL_FALSE, // no as_name
            TAG_LOCATION,
            int_val(1), // start line 1
            int_val(0), // start column 0 (0-based)
            int_val(0), // line difference (same line)
            int_val(9), // column difference (9 chars)
            TAG_LITERAL_INT, // flags tag
            int_val(1), // flags: IMPORT_FLAG_TOP_LEVEL (bit 0 set)
        ];

        assert_eq!(bytes, expected);
    }
}
