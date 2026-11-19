//! Python bindings for `gamelib/`: generated from the signatures in `port.rs` by rust-anything.
//! Regenerate with `rust-anything refresh` after changing them.

use pyo3::prelude::*;

pub mod js;
pub mod port;
pub mod service_config;
pub mod pyconv;
pub mod pynum;

#[allow(unused_imports)]
use pynum::Num;

fn error(message: String) -> PyErr {
    use pyo3::exceptions as e;
    let (kind, text) = message.split_once(": ").unwrap_or(("", message.as_str()));
    let text = text.to_string();
    match kind {
        "TypeError" => e::PyTypeError::new_err(text),
        "KeyError" => e::PyKeyError::new_err(text),
        "IndexError" => e::PyIndexError::new_err(text),
        "OverflowError" => e::PyOverflowError::new_err(text),
        "ZeroDivisionError" => e::PyZeroDivisionError::new_err(text),
        "AttributeError" => e::PyAttributeError::new_err(text),
        "RuntimeError" => e::PyRuntimeError::new_err(text),
        "NotImplementedError" => e::PyNotImplementedError::new_err(text),
        "LookupError" => e::PyLookupError::new_err(text),
        "ArithmeticError" => e::PyArithmeticError::new_err(text),
        "UnicodeError" => e::PyUnicodeError::new_err(text),
        "ValueError" => e::PyValueError::new_err(text),
        // HAND-PATCH (re-apply after `rust-anything refresh`): `struct.pack('<H', ...)`
        // raises the real struct.error, whose type name is `error` — a ValueError would
        // change the exception type across the cutover.
        "struct.error" => Python::attach(|py| {
            let ty: pyo3::Bound<'_, pyo3::types::PyType> = py
                .import("struct")
                .and_then(|m| m.getattr("error"))
                .expect("INFALLIBLE: struct.error exists")
                .cast_into::<pyo3::types::PyType>()
                .expect("INFALLIBLE: struct.error is a type");
            PyErr::from_type(ty, text)
        }),
        _ => e::PyValueError::new_err(message),
    }
}

fn get_flag_inner(
    team_id: i64,
    service_id: i64,
    tick: i64,
    payload: i64,
) -> Result<String, String> {
    let out = port::get_flag(team_id, service_id, tick, payload)?;
    Ok(out)
}

/// `get_flag`.
#[pyfunction]
#[pyo3(name = "get_flag", signature = (team_id, service_id, tick, payload=0))]
fn get_flag(team_id: i64, service_id: i64, tick: i64, payload: i64) -> PyResult<String> {
    get_flag_inner(team_id, service_id, tick, payload).map_err(error)
}

fn get_flag_with_key_inner(
    key: Bound<'_, PyAny>,
    team_id: i64,
    service_id: i64,
    tick: i64,
    payload: i64,
) -> Result<String, String> {
    let key = crate::pyconv::arg(&key)?;
    let out = port::get_flag_with_key(key, team_id, service_id, tick, payload)?;
    Ok(out)
}

/// `getFlagWithKey`.
#[pyfunction]
#[pyo3(name = "getFlagWithKey", signature = (key, team_id, service_id, tick, payload))]
fn get_flag_with_key(
    key: Bound<'_, PyAny>,
    team_id: i64,
    service_id: i64,
    tick: i64,
    payload: i64,
) -> PyResult<String> {
    get_flag_with_key_inner(key, team_id, service_id, tick, payload).map_err(error)
}

fn get_flag_regex_pattern_inner() -> Result<String, String> {
    let out = port::get_flag_regex_pattern();
    Ok(out)
}

/// `pattern`.
#[pyfunction]
#[pyo3(name = "pattern", signature = ())]
fn get_flag_regex_pattern() -> PyResult<String> {
    get_flag_regex_pattern_inner().map_err(error)
}

fn generate_flag_id_inner(
    flag_id_type: String,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
) -> Result<String, String> {
    let out = port::generate_flag_id(flag_id_type, service_id, team_id, tick, index)?;
    Ok(out)
}

/// `generate_flag_id`.
#[pyfunction]
#[pyo3(name = "generate_flag_id", signature = (flag_id_type, service_id, team_id, tick, index))]
fn generate_flag_id(
    flag_id_type: String,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
) -> PyResult<String> {
    generate_flag_id_inner(flag_id_type, service_id, team_id, tick, index).map_err(error)
}

fn generate_flag_id_with_key_inner(
    key: Bound<'_, PyAny>,
    flag_id_type: String,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
) -> Result<String, String> {
    let key = crate::pyconv::arg(&key)?;
    let out = port::generate_flag_id_with_key(key, flag_id_type, service_id, team_id, tick, index)?;
    Ok(out)
}

/// `generateFlagIdWithKey`.
#[pyfunction]
#[pyo3(name = "generateFlagIdWithKey", signature = (key, flag_id_type, service_id, team_id, tick, index))]
fn generate_flag_id_with_key(
    key: Bound<'_, PyAny>,
    flag_id_type: String,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
) -> PyResult<String> {
    generate_flag_id_with_key_inner(key, flag_id_type, service_id, team_id, tick, index)
        .map_err(error)
}

fn generate_flag_id_rnd_inner(
    flag_id_type: String,
    seed: Bound<'_, PyAny>,
) -> Result<String, String> {
    let seed = crate::pyconv::arg(&seed)?;
    let out = port::generate_flag_id_rnd(flag_id_type, seed)?;
    Ok(out)
}

/// `generateFlagIdRnd`.
#[pyfunction]
#[pyo3(name = "generateFlagIdRnd", signature = (flag_id_type, seed))]
fn generate_flag_id_rnd(flag_id_type: String, seed: Bound<'_, PyAny>) -> PyResult<String> {
    generate_flag_id_rnd_inner(flag_id_type, seed).map_err(error)
}

fn ucfirst_inner(s: String) -> Result<String, String> {
    let out = port::ucfirst(s)?;
    Ok(out)
}

/// `ucfirst`.
#[pyfunction]
#[pyo3(name = "ucfirst", signature = (s))]
fn ucfirst(s: String) -> PyResult<String> {
    ucfirst_inner(s).map_err(error)
}

fn is_generatable_flag_id_inner(flag_id_type: String) -> Result<bool, String> {
    let out = port::is_generatable_flag_id(flag_id_type)?;
    Ok(out)
}

/// `is_generatable_flag_id`.
#[pyfunction]
#[pyo3(name = "is_generatable_flag_id", signature = (flag_id_type))]
fn is_generatable_flag_id(flag_id_type: String) -> PyResult<bool> {
    is_generatable_flag_id_inner(flag_id_type).map_err(error)
}

fn is_valid_flag_id_inner(flag_id_type: String) -> Result<bool, String> {
    let out = port::is_valid_flag_id(flag_id_type)?;
    Ok(out)
}

/// `is_valid_flag_id`.
#[pyfunction]
#[pyo3(name = "is_valid_flag_id", signature = (flag_id_type))]
fn is_valid_flag_id(flag_id_type: String) -> PyResult<bool> {
    is_valid_flag_id_inner(flag_id_type).map_err(error)
}

#[pymodule]
#[pyo3(name = "_native")]
fn native_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(get_flag, m)?)?;
    m.add_function(wrap_pyfunction!(get_flag_with_key, m)?)?;
    m.add_function(wrap_pyfunction!(get_flag_regex_pattern, m)?)?;
    m.add_function(wrap_pyfunction!(generate_flag_id, m)?)?;
    m.add_function(wrap_pyfunction!(generate_flag_id_with_key, m)?)?;
    m.add_function(wrap_pyfunction!(generate_flag_id_rnd, m)?)?;
    m.add_function(wrap_pyfunction!(ucfirst, m)?)?;
    m.add_function(wrap_pyfunction!(is_generatable_flag_id, m)?)?;
    m.add_function(wrap_pyfunction!(is_valid_flag_id, m)?)?;
    Ok(())
}
