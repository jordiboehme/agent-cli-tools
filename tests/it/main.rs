//! One integration test binary for all commands; each module drives one
//! built binary through `CARGO_BIN_EXE_<name>`.

mod nproc;
mod tac;
mod timeout;
