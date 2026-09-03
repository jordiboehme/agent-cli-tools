//! Shared building blocks for the agent-cli-tools commands.

pub mod duration;
pub mod signals;

/// The `--version` text every command prints: the GNU shape, naming this
/// project and its home page so a user can find where the binary came from.
pub fn version_text(command: &str) -> String {
    format!(
        "{command} ({}) {}\n\
         Copyright (c) 2026 Jordi Böhme\n\
         License MIT <https://opensource.org/license/mit>.\n\
         This is free software: you are free to change and redistribute it.\n\
         There is NO WARRANTY, to the extent permitted by law.\n\
         \n\
         Home page: <{}>\n",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    )
}
