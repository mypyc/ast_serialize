use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::path::Path;
use pyo3::exceptions::PyValueError;

mod func_effect_visitor;
mod options;
pub mod reachability;
mod serialize_ast;
pub mod type_comment;

/// Parse a Python file and serialize its AST to mypy's binary format.
///
/// # Arguments
///
/// * `fnam` - Path to the Python file to parse
/// * `skip_function_bodies` - Optional boolean to skip function bodies without externally visible effects (default: false)
/// * `python_version` - Optional tuple (major, minor) for reachability analysis (default: use sys.version_info)
/// * `platform` - Optional platform string for reachability analysis (default: use sys.platform)
/// * `always_true` - Optional list of names that are always considered true (default: empty)
/// * `always_false` - Optional list of names that are always considered false (default: empty)
///
/// # Returns
///
/// A tuple containing:
/// - bytes: The serialized AST in mypy's format (may be partial if there are syntax errors)
/// - list: A list of syntax errors, where each error is a dict with 'line', 'column', and 'message'
/// - list[tuple[int, list[str]]]: A list of tuples (line_number, error_codes) for `type: ignore` comments
/// - bytes: The serialized imports metadata
///
/// # Errors
///
/// Raises ValueError if the file cannot be read (but not for syntax errors)
#[pyfunction]
#[pyo3(signature = (
    fnam,
    source=None,
    skip_function_bodies=false,
    python_version=None,
    platform=None,
    always_true=None,
    always_false=None
))]
fn parse(
    py: Python,
    fnam: String,
    source: Option<String>,
    skip_function_bodies: bool,
    python_version: Option<(u32, u32)>,
    platform: Option<String>,
    always_true: Option<Vec<String>>,
    always_false: Option<Vec<String>>,
) -> PyResult<(Vec<u8>, Vec<Py<PyAny>>, Vec<Py<PyAny>>, Vec<u8>, Py<PyAny>)> {
    // Get defaults from Python if not provided
    let python_version = match python_version {
        Some(v) => v,
        None => get_default_python_version(py)?,
    };

    let platform = match platform {
        Some(p) => p,
        None => get_default_platform(py)?,
    };

    let always_true = always_true.unwrap_or_default();
    let always_false = always_false.unwrap_or_default();

    if source.is_some() {
        return Err(PyErr::new::<PyValueError, _>("Source parsing is not supported yet"));
    }

    let path = Path::new(&fnam);
    let (ast_bytes, syntax_errors, type_ignore_lines, import_bytes, is_partial_package) = py
        .detach(|| {
            serialize_ast::serialize_python_file(
                path,
                skip_function_bodies,
                options::Options::new(python_version, platform, always_true, always_false),
            )
        })
        .map_err(|e| PyErr::new::<PyValueError, _>(e.to_string()))?;

    // Convert syntax errors to Python dicts
    let py_errors: PyResult<Vec<Py<PyAny>>> = syntax_errors
        .iter()
        .map(|error| {
            let dict = PyDict::new(py);
            dict.set_item("line", error.line)?;
            dict.set_item("column", error.column)?;
            dict.set_item("message", error.message.clone())?;
            dict.set_item("blocker", error.blocker)?;
            Ok(dict.into())
        })
        .collect();
    let py_errors = py_errors?;

    // Convert type ignore lines to Python tuples (line, error_codes)
    let py_type_ignores: PyResult<Vec<Py<PyAny>>> = type_ignore_lines
        .iter()
        .map(|(line, error_codes)| {
            let tuple = PyTuple::new(
                py,
                [
                    line.into_pyobject(py)?.into_any(),
                    error_codes.into_pyobject(py)?.into_any(),
                ],
            )?;
            Ok(tuple.into())
        })
        .collect();
    let py_type_ignores = py_type_ignores?;

    // Add various mypy-specific extra information about AST.
    let ast_data = PyDict::new(py);
    ast_data.set_item("is_partial_package", is_partial_package)?;
    let ast_data = ast_data.into();

    Ok((ast_bytes, py_errors, py_type_ignores, import_bytes, ast_data))
}

/// Get the default Python version from sys.version_info
fn get_default_python_version(py: Python) -> PyResult<(u32, u32)> {
    let sys = py.import("sys")?;
    let version_info = sys.getattr("version_info")?;
    let major: u32 = version_info.get_item(0)?.extract()?;
    let minor: u32 = version_info.get_item(1)?.extract()?;
    Ok((major, minor))
}

/// Get the default platform from sys.platform
fn get_default_platform(py: Python) -> PyResult<String> {
    let sys = py.import("sys")?;
    sys.getattr("platform")?.extract()
}

/// A Python module for parsing Python files and serializing to mypy AST format
#[pymodule]
fn ast_serialize(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    Ok(())
}
