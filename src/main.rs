// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// The code is returned, not passed to process::exit, so the C runtime's exit
// handlers run on Windows too.
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A panic is Python's uncaught exception: its report is already printed, and
    // the interpreter still flushed stdout before exiting 1.
    let code = std::panic::catch_unwind(|| lamp::cli::main(&args)).unwrap_or(1);
    std::process::ExitCode::from(lamp::out::shutdown(code) as u8)
}
