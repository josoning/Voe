//! Unified colored I/O helpers for the Voe shell framework.
//!
//! Wraps [`owo_colors`] with:
//! - A fixed [`ColorScheme`] describing how different message categories
//!   (success, warning, error, info, ...) should be rendered.
//! - Convenience functions for printing to stdout/stderr.
//! - Macros that mirror `print!` / `println!` / `eprintln!` but apply
//!   the correct scheme automatically.
//!
//! The layer is intentionally small so that individual commands never
//! need to reach for raw ANSI escape codes or [`owo_colors::OwoColorize`]
//! directly — they call `shell_info!(...)` or [`println_ok`] and the
//! framework decides whether to apply colors (TTY) or emit plain text
//! (pipe, CI, `NO_COLOR=1`).

use owo_colors::OwoColorize;
use std::env;
use std::io::{self, IsTerminal, Write};

/// Semantic color categories used throughout the shell.
///
/// A [`ColorScheme`] maps each category to concrete styling rules.
/// Adding a new semantic role here automatically makes it available to
/// every command — individual command code never has to know *what*
/// color is rendered, only *why*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRole {
    /// Informational output (neutral).
    Info,
    /// A successful operation.
    Success,
    /// A warning or recoverable problem.
    Warning,
    /// An unrecoverable error.
    Error,
    /// Decorative heading or section title.
    Heading,
    /// User-facing prompt / label.
    Prompt,
    /// Dimmed or secondary text.
    Dim,
}

/// A concrete mapping from [`ColorRole`] values to their rendered style.
///
/// Built-in presets are provided via [`ColorScheme::default_preset`] and
/// friends; custom schemes can be constructed with [`ColorScheme::new`]
/// for future theming support.
#[derive(Debug, Clone)]
pub struct ColorScheme {
    pub info: String,
    pub success: String,
    pub warning: String,
    pub error: String,
    pub heading: String,
    pub prompt: String,
    pub dim: String,
}

impl ColorScheme {
    /// Returns the default Voe color scheme: cyan info, green success,
    /// yellow warning, red error, bold headings.
    pub fn default_preset() -> Self {
        Self {
            info: String::new(),
            success: String::new().green().bold().to_string(),
            warning: String::new().yellow().bold().to_string(),
            error: String::new().red().bold().to_string(),
            heading: String::new().cyan().bold().to_string(),
            prompt: String::new().cyan().to_string(),
            dim: String::new().dimmed().to_string(),
        }
    }

    /// Build a fully custom scheme.  Each string is the ANSI escape
    /// sequence (or empty string for no styling) applied to that role.
    ///
    /// # Note
    /// Prefer the default preset unless you have a strong reason — the
    /// visual consistency is part of the framework contract.
    pub fn new(
        info: impl Into<String>,
        success: impl Into<String>,
        warning: impl Into<String>,
        error: impl Into<String>,
        heading: impl Into<String>,
        prompt: impl Into<String>,
        dim: impl Into<String>,
    ) -> Self {
        Self {
            info: info.into(),
            success: success.into(),
            warning: warning.into(),
            error: error.into(),
            heading: heading.into(),
            prompt: prompt.into(),
            dim: dim.into(),
        }
    }

    /// Apply the scheme's role-specific styling to a string.
    ///
    /// When colors are disabled (via `NO_COLOR`, `CLICOLOR=0`, or a
    /// non-TTY stdout), this simply returns the input unchanged.
    pub fn apply(&self, role: ColorRole, text: &str) -> String {
        if !colors_enabled() {
            return text.to_string();
        }
        let style = match role {
            ColorRole::Info => &self.info,
            ColorRole::Success => &self.success,
            ColorRole::Warning => &self.warning,
            ColorRole::Error => &self.error,
            ColorRole::Heading => &self.heading,
            ColorRole::Prompt => &self.prompt,
            ColorRole::Dim => &self.dim,
        };
        // If the style string is empty (user chose no styling for this
        // role), skip wrapping entirely so we don't emit stray "\x1b[0m".
        if style.is_empty() {
            text.to_string()
        } else {
            // owo-colors already produces the full escape sequence; we
            // wrap with reset to guarantee style does not bleed into
            // the next uncolored chunk.
            format!("{}{}\x1b[0m", style, text)
        }
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::default_preset()
    }
}

/// Returns `true` if colored output is appropriate for the current
/// environment.
///
/// Respects the [NO_COLOR](https://no-color.org/) convention, the
/// `CLICOLOR=0` override, and forces colors on when `FORCE_COLOR` is
/// set to any non-empty value.  When none of these env vars is present
/// we check whether stdout is a TTY — if not (pipe, CI redirects),
/// colors are disabled.
pub fn colors_enabled() -> bool {
    if let Ok(v) = env::var("NO_COLOR") {
        if !v.is_empty() {
            return false;
        }
    }
    if let Ok(v) = env::var("FORCE_COLOR") {
        if !v.is_empty() {
            return true;
        }
    }
    if let Ok(v) = env::var("CLICOLOR") {
        if v == "0" {
            return false;
        }
    }
    // Final fallback: only emit colors when stdout is a real terminal.
    std::io::stdout().is_terminal()
}

// ---------------------------------------------------------------------------
// Convenience printing functions (used inside `impl` blocks where macros
// cannot be used or when the caller wants programmatic control).

/// Print an informational line (stdout, default scheme).
pub fn println_info(msg: &str) {
    let scheme = ColorScheme::default();
    let _ = writeln!(io::stdout(), "{}", scheme.apply(ColorRole::Info, msg));
}

/// Print a success line (stdout, green-bold).
pub fn println_ok(msg: &str) {
    let scheme = ColorScheme::default();
    let _ = writeln!(io::stdout(), "{}", scheme.apply(ColorRole::Success, msg));
}

/// Print a warning line (stderr, yellow-bold).
pub fn println_warn(msg: &str) {
    let scheme = ColorScheme::default();
    let _ = writeln!(io::stderr(), "{}", scheme.apply(ColorRole::Warning, msg));
}

/// Print an error line (stderr, red-bold).
pub fn println_err(msg: &str) {
    let scheme = ColorScheme::default();
    let _ = writeln!(io::stderr(), "{}", scheme.apply(ColorRole::Error, msg));
}

/// Print a heading (stdout, cyan-bold).
pub fn println_heading(msg: &str) {
    let scheme = ColorScheme::default();
    let _ = writeln!(io::stdout(), "{}", scheme.apply(ColorRole::Heading, msg));
}

// ---------------------------------------------------------------------------
// Macros.  These are the preferred way for shell commands to produce
// output — they look just like `println!` and hide all styling logic.

/// Print a line to stdout using the "info" role (default text).
#[macro_export]
macro_rules! shell_info {
    ($($arg:tt)*) => {
        $crate::io::println_info(&format!($($arg)*))
    };
}

/// Print a success line to stdout (green-bold).
#[macro_export]
macro_rules! shell_ok {
    ($($arg:tt)*) => {
        $crate::io::println_ok(&format!($($arg)*))
    };
}

/// Print a warning line to stderr (yellow-bold).
#[macro_export]
macro_rules! shell_warn {
    ($($arg:tt)*) => {
        $crate::io::println_warn(&format!($($arg)*))
    };
}

/// Print an error line to stderr (red-bold).
#[macro_export]
macro_rules! shell_err {
    ($($arg:tt)*) => {
        $crate::io::println_err(&format!($($arg)*))
    };
}

/// Print a heading to stdout (cyan-bold).
#[macro_export]
macro_rules! shell_heading {
    ($($arg:tt)*) => {
        $crate::io::println_heading(&format!($($arg)*))
    };
}
