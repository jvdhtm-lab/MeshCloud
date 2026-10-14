//! Sandboxed WebAssembly execution.
//!
//! Every job runs in a fresh Wasmtime store with:
//! - a hard memory ceiling (`StoreLimits`),
//! - fuel metering so runaway loops are terminated,
//! - epoch-based interruption for wall-clock timeouts,
//! - no WASI, no preopened directories, and no network access.
//!
//! Modules export `memory`, `alloc(len: i32) -> ptr: i32`, and
//! `run(ptr: i32, len: i32) -> out_ptr: i32`, where the output is a
//! NUL-terminated UTF-8 string in linear memory.
//!
//! For defense in depth [`run_isolated`] performs the same execution inside a
//! separate process by re-executing the host binary in sandbox mode.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};

/// Resource limits applied to a single job execution.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    /// Maximum linear-memory size in bytes.
    pub max_memory_bytes: usize,
    /// Fuel budget. Each Wasm instruction consumes fuel.
    pub fuel: u64,
    /// Wall-clock timeout.
    pub timeout: Duration,
}

impl RuntimeLimits {
    /// Build limits from megabytes, a fuel budget, and a timeout.
    pub fn from_megabytes(max_memory_mb: u64, fuel: u64, timeout: Duration) -> Self {
        Self {
            max_memory_bytes: (max_memory_mb as usize).saturating_mul(1024 * 1024),
            fuel,
            timeout,
        }
    }
}

/// The result of executing a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    /// Raw output bytes returned by the module.
    pub output: Vec<u8>,
    /// Fuel consumed by the execution.
    pub fuel_consumed: u64,
}

/// Errors raised during sandboxed execution.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The module could not be compiled.
    #[error("failed to compile module: {0}")]
    Compile(String),
    /// The module could not be instantiated.
    #[error("failed to instantiate module: {0}")]
    Instantiate(String),
    /// The module does not expose the expected ABI.
    #[error("invalid module ABI: {0}")]
    InvalidAbi(String),
    /// The module exhausted its fuel budget.
    #[error("module exhausted its fuel budget")]
    FuelExhausted,
    /// The module exceeded the wall-clock timeout.
    #[error("module exceeded the execution timeout")]
    Timeout,
    /// The module exceeded its memory limit.
    #[error("module exceeded its memory limit")]
    MemoryLimit,
    /// The module trapped for another reason.
    #[error("module trapped: {0}")]
    Trap(String),
    /// An I/O error occurred (isolated mode).
    #[error("sandbox I/O error: {0}")]
    Io(String),
    /// The isolated sandbox process failed.
    #[error("sandbox process failed: {0}")]
    External(String),
}

/// Execute `wasm` (binary or WAT) with `input` under `limits`.
pub fn run(wasm: &[u8], input: &[u8], limits: &RuntimeLimits) -> Result<RunOutput, RuntimeError> {
    let compiled = wat::parse_bytes(wasm)
        .map_err(|e| RuntimeError::Compile(e.to_string()))?
        .into_owned();

    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    let engine = Engine::new(&config).map_err(|e| RuntimeError::Instantiate(e.to_string()))?;

    let module =
        Module::new(&engine, &compiled).map_err(|e| RuntimeError::Compile(e.to_string()))?;

    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.max_memory_bytes)
        .build();
    let mut store = Store::new(&engine, store_limits);
    store.limiter(|s| s as &mut dyn wasmtime::ResourceLimiter);
    store
        .set_fuel(limits.fuel)
        .map_err(|e| RuntimeError::Instantiate(e.to_string()))?;
    store.set_epoch_deadline(1);

    // A watchdog thread interrupts execution if the timeout elapses first.
    let engine_handle = engine.clone();
    let timeout = limits.timeout;
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(timeout).is_err() {
            engine_handle.increment_epoch();
        }
    });

    let result = execute(&mut store, &module, input, limits.fuel);
    let _ = done_tx.send(());
    let _ = watchdog.join();
    result
}

fn execute(
    store: &mut Store<StoreLimits>,
    module: &Module,
    input: &[u8],
    fuel_budget: u64,
) -> Result<RunOutput, RuntimeError> {
    let instance = Instance::new(&mut *store, module, &[])
        .map_err(|e| RuntimeError::Instantiate(e.to_string()))?;

    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| RuntimeError::InvalidAbi("module does not export `memory`".into()))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "alloc")
        .map_err(|e| RuntimeError::InvalidAbi(format!("alloc: {e}")))?;
    let run = instance
        .get_typed_func::<(i32, i32), i32>(&mut *store, "run")
        .map_err(|e| RuntimeError::InvalidAbi(format!("run: {e}")))?;

    let in_ptr = alloc
        .call(&mut *store, input.len() as i32)
        .map_err(map_trap)?;
    memory
        .write(&mut *store, in_ptr as usize, input)
        .map_err(|_| RuntimeError::MemoryLimit)?;

    let out_ptr = run
        .call(&mut *store, (in_ptr, input.len() as i32))
        .map_err(map_trap)?;

    let data = memory.data(&*store);
    let start = out_ptr as usize;
    if start >= data.len() {
        return Err(RuntimeError::InvalidAbi(
            "output pointer out of bounds".into(),
        ));
    }
    let mut end = start;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    if end >= data.len() {
        return Err(RuntimeError::InvalidAbi(
            "output is not NUL-terminated".into(),
        ));
    }
    let output = data[start..end].to_vec();

    let remaining = store.get_fuel().unwrap_or(0);
    Ok(RunOutput {
        output,
        fuel_consumed: fuel_budget.saturating_sub(remaining),
    })
}

fn map_trap(error: wasmtime::Error) -> RuntimeError {
    if let Some(trap) = error.downcast_ref::<Trap>() {
        return match trap {
            Trap::OutOfFuel => RuntimeError::FuelExhausted,
            Trap::Interrupt => RuntimeError::Timeout,
            Trap::MemoryOutOfBounds => RuntimeError::MemoryLimit,
            other => RuntimeError::Trap(other.to_string()),
        };
    }
    let message = error.to_string();
    if message.contains("fuel") {
        RuntimeError::FuelExhausted
    } else if message.contains("epoch") {
        RuntimeError::Timeout
    } else if message.contains("memory") {
        RuntimeError::MemoryLimit
    } else {
        RuntimeError::Trap(message)
    }
}

/// Execute a module in a separate process by re-executing `host_executable`
/// with the [`SANDBOX_ARG`] flag. This confines the Wasmtime runtime to a
/// short-lived child process.
pub fn run_isolated(
    host_executable: &Path,
    wasm: &[u8],
    input: &[u8],
    limits: &RuntimeLimits,
) -> Result<RunOutput, RuntimeError> {
    let mut child = Command::new(host_executable)
        .arg(SANDBOX_ARG)
        .arg("--max-memory")
        .arg(limits.max_memory_bytes.to_string())
        .arg("--fuel")
        .arg(limits.fuel.to_string())
        .arg("--timeout-ms")
        .arg(limits.timeout.as_millis().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RuntimeError::Io(e.to_string()))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| RuntimeError::Io("no sandbox stdin".into()))?;
        write_frame(stdin, wasm).map_err(|e| RuntimeError::Io(e.to_string()))?;
        write_frame(stdin, input).map_err(|e| RuntimeError::Io(e.to_string()))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| RuntimeError::Io(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RuntimeError::External(format!(
            "exit {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    let stdout = output.stdout;
    if stdout.len() < 9 {
        return Err(RuntimeError::External("truncated sandbox response".into()));
    }
    let status = stdout[0];
    let mut fuel_bytes = [0u8; 8];
    fuel_bytes.copy_from_slice(&stdout[1..9]);
    let fuel_consumed = u64::from_le_bytes(fuel_bytes);
    let payload = &stdout[9..];
    if status == 0 {
        Ok(RunOutput {
            output: payload.to_vec(),
            fuel_consumed,
        })
    } else {
        Err(RuntimeError::External(
            String::from_utf8_lossy(payload).into_owned(),
        ))
    }
}

/// Argument that puts the host binary into sandbox mode.
pub const SANDBOX_ARG: &str = "__sandbox";

/// Entry point for sandbox mode. Reads the request from stdin and writes the
/// result to stdout.
pub fn sandbox_main() -> Result<(), RuntimeError> {
    let args: Vec<String> = std::env::args().collect();
    let mut max_memory_bytes = 64usize * 1024 * 1024;
    let mut fuel = 1_000_000_000u64;
    let mut timeout_ms = 5_000u64;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--max-memory" => {
                max_memory_bytes = args
                    .get(index + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(max_memory_bytes);
                index += 2;
            }
            "--fuel" => {
                fuel = args
                    .get(index + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(fuel);
                index += 2;
            }
            "--timeout-ms" => {
                timeout_ms = args
                    .get(index + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(timeout_ms);
                index += 2;
            }
            _ => index += 1,
        }
    }

    let mut stdin = std::io::stdin().lock();
    let wasm = read_frame(&mut stdin).map_err(|e| RuntimeError::Io(e.to_string()))?;
    let input = read_frame(&mut stdin).map_err(|e| RuntimeError::Io(e.to_string()))?;

    let limits = RuntimeLimits {
        max_memory_bytes,
        fuel,
        timeout: Duration::from_millis(timeout_ms),
    };

    let mut stdout = std::io::stdout().lock();
    match run(&wasm, &input, &limits) {
        Ok(output) => {
            let _ = stdout.write_all(&[0u8]);
            let _ = stdout.write_all(&output.fuel_consumed.to_le_bytes());
            let _ = stdout.write_all(&output.output);
        }
        Err(err) => {
            let _ = stdout.write_all(&[1u8]);
            let _ = stdout.write_all(&0u64.to_le_bytes());
            let _ = stdout.write_all(err.to_string().as_bytes());
        }
    }
    let _ = stdout.flush();
    Ok(())
}

fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)
}

fn read_frame(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut buffer = vec![0u8; len];
    reader.read_exact(&mut buffer)?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ECHO_WAT: &str = r#"
        (module
          (memory (export "memory") 4)
          (func (export "alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "run") (param $ptr i32) (param $len i32) (result i32)
            (local $out i32)
            (local.set $out (i32.const 65536))
            (memory.copy (local.get $out) (local.get $ptr) (local.get $len))
            (i32.store8 (i32.add (local.get $out) (local.get $len)) (i32.const 0))
            (local.get $out)))
    "#;

    const HELLO_WAT: &str = r#"
        (module
          (memory (export "memory") 1)
          (data (i32.const 16) "hello from meshcloud\00")
          (func (export "alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "run") (param i32 i32) (result i32) (i32.const 16)))
    "#;

    const SPIN_WAT: &str = r#"
        (module
          (memory (export "memory") 1)
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "run") (param i32 i32) (result i32)
            (loop $l (br $l))
            (i32.const 0)))
    "#;

    const GROW_WAT: &str = r#"
        (module
          (memory (export "memory") 1)
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "run") (param i32 i32) (result i32)
            (drop (memory.grow (i32.const 1024)))
            (i32.store (i32.const 100000000) (i32.const 1))
            (i32.const 0)))
    "#;

    fn limits() -> RuntimeLimits {
        RuntimeLimits::from_megabytes(64, 100_000_000, Duration::from_secs(5))
    }

    #[test]
    fn runs_hello_and_is_deterministic() {
        let a = run(HELLO_WAT.as_bytes(), b"", &limits()).unwrap();
        let b = run(HELLO_WAT.as_bytes(), b"", &limits()).unwrap();
        assert_eq!(a.output, b.output);
        assert_eq!(a.output, b"hello from meshcloud");
    }

    #[test]
    fn echoes_input() {
        let out = run(ECHO_WAT.as_bytes(), b"payload-123", &limits()).unwrap();
        assert_eq!(out.output, b"payload-123");
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Wasmtime's longjmp trap unwinding is unsupported on Windows"
    )]
    fn fuel_exhaustion_is_terminated() {
        let limits = RuntimeLimits::from_megabytes(64, 1_000_000, Duration::from_secs(30));
        let err = run(SPIN_WAT.as_bytes(), b"", &limits).unwrap_err();
        assert!(matches!(err, RuntimeError::FuelExhausted), "got {err:?}");
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Wasmtime's longjmp trap unwinding is unsupported on Windows"
    )]
    fn timeout_interrupts_long_execution() {
        let limits = RuntimeLimits {
            max_memory_bytes: 64 * 1024 * 1024,
            fuel: u64::MAX,
            timeout: Duration::from_millis(200),
        };
        let err = run(SPIN_WAT.as_bytes(), b"", &limits).unwrap_err();
        assert!(matches!(err, RuntimeError::Timeout), "got {err:?}");
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Wasmtime's longjmp trap unwinding is unsupported on Windows"
    )]
    fn memory_limit_is_enforced() {
        let limits = RuntimeLimits::from_megabytes(1, 100_000_000, Duration::from_secs(5));
        let err = run(GROW_WAT.as_bytes(), b"", &limits).unwrap_err();
        assert!(
            matches!(err, RuntimeError::MemoryLimit | RuntimeError::Trap(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn host_imports_are_rejected() {
        const IMPORT_WAT: &str = r#"
            (module
              (import "wasi_snapshot_preview1" "fd_read"
                (func (param i32 i32 i32 i32) (result i32)))
              (memory (export "memory") 1)
              (func (export "alloc") (param i32) (result i32) (i32.const 0))
              (func (export "run") (param i32 i32) (result i32) (i32.const 0)))
        "#;
        let err = run(IMPORT_WAT.as_bytes(), b"", &limits()).unwrap_err();
        assert!(matches!(err, RuntimeError::Instantiate(_)), "got {err:?}");
    }
}
