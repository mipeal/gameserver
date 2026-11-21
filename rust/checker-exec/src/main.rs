//! checker-exec: Rust launcher that embeds CPython and runs the checker shim
//! (port of `checker_runner/runners/ctf.py`, ADR-017). The worker spawns
//! `checker-exec <package> <script> <service_id> <team_id> <tick> <name> <flag_ids>`.
//! Output contract: console output, then SEPARATOR + `status|message`, exit 0;
//! an uncaught Python error prints its traceback and exits 1 (the worker
//! maps that to CRASHED, as with the original).

use pyo3::prelude::*;
use std::process::ExitCode;

const SHIM: &str = include_str!("shim.py");

fn main() -> ExitCode {
    Python::attach(|py| {
        // argv[0] is this binary; the shim reads argv[1..] like the original read sys.argv.
        let argv: Vec<String> = std::env::args().collect();
        match (|| -> PyResult<()> {
            let sys = py.import("sys")?;
            let list = pyo3::types::PyList::new(py, argv)?;
            sys.setattr("argv", list)?;
            let shim = PyModule::from_code(
                py,
                std::ffi::CString::new(SHIM)?.as_c_str(),
                c"checker_exec_shim.py",
                c"checker_exec_shim",
            )?;
            shim.getattr("main")?.call0()?;
            Ok(())
        })() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                err.print(py);
                ExitCode::from(1)
            }
        }
    })
}
