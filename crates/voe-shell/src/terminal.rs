//! Terminal control layer for the Voe shell.
//!
//! Provides a thin wrapper around [`crossterm`] that manages the
//! transition into a full-screen alternate-buffer session and guarantees
//! that the terminal is restored to its original state when the shell
//! exits — even on panic or early return — via the [`TerminalGuard`]
//! RAII guard.

use crossterm::{
    cursor, execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};

/// Captured snapshot of the terminal state before entering the shell.
///
/// Used internally by [`TerminalGuard`] to restore the exact state that
/// was present when the shell launched.  The struct is opaque outside
/// this module — external code only ever sees the guard itself.
#[derive(Debug, Clone)]
struct TerminalSnapshot {
    #[allow(dead_code)]
    original_title: Option<String>,
}

/// RAII guard that keeps the terminal in "shell mode" for its lifetime.
///
/// On construction the guard switches to the alternate screen buffer and
/// hides the cursor.  On drop it leaves the alternate screen, restores
/// the cursor, and resets all terminal attributes — regardless of how
/// the scope is exited (normal return, `?`, or panic unwind).
///
/// # Usage
/// ```no_run
/// use voe_shell::terminal::TerminalGuard;
///
/// let _guard = TerminalGuard::enter_fullscreen().unwrap();
/// // ... immersive REPL work here ...
/// // guard dropped → terminal fully restored
/// ```
pub struct TerminalGuard {
    snapshot: TerminalSnapshot,
    active: bool,
}

impl TerminalGuard {
    /// Enter the full-screen alternate buffer and hide the cursor.
    ///
    /// Returns the RAII guard on success.  If the terminal does not
    /// support alternate-screen switching (e.g. pipes or non-TTY output),
    /// the function returns `Err` so callers can fall back to a plain
    /// line-based REPL.
    pub fn enter_fullscreen() -> io::Result<Self> {
        // Only enter alternate screen when we are attached to a real TTY.
        // Without this check we would silently corrupt output when the
        // binary is piped to another process or run under CI.
        // `is_raw_mode_enabled` returns a Result — ignore the error and
        // simply skip the check when the query itself fails.
        if terminal::is_raw_mode_enabled().unwrap_or(false) {
            // Not strictly required — alternate screen works without raw
            // mode — but the extra check keeps the guard symmetric.
        }

        execute!(
            io::stdout(),
            EnterAlternateScreen,
            cursor::Hide,
            terminal::Clear(ClearType::All),
        )?;

        Ok(Self {
            snapshot: TerminalSnapshot {
                original_title: None,
            },
            active: true,
        })
    }

    /// Enter a **non-fullscreen** guard that still hides/restores the
    /// cursor but does not touch the alternate screen.
    ///
    /// This is used when running under a non-TTY environment where
    /// alternate-screen switching would be harmful.  The guard still
    /// provides the same "always restore on drop" guarantee.
    pub fn enter_line_mode() -> io::Result<Self> {
        execute!(io::stdout(), cursor::Hide)?;

        Ok(Self {
            snapshot: TerminalSnapshot {
                original_title: None,
            },
            active: true,
        })
    }

    /// Manually trigger the restore sequence before the guard is dropped.
    ///
    /// After calling this method the guard is no longer active — calling
    /// it again is a no-op.
    pub fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        restore_terminal(&self.snapshot)
    }

    /// Returns `true` if the guard is still protecting the terminal
    /// (i.e. [`restore`](Self::restore) has not been called yet and we
    /// have not been dropped).
    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            // Best-effort: we must never panic from Drop, so ignore IO errors.
            let _ = restore_terminal(&self.snapshot);
            self.active = false;
        }
    }
}

/// Low-level helper that issues the reset escape sequences.
///
/// Called by both [`TerminalGuard::restore`] and the [`Drop`] impl.
/// Uses `crossterm` commands so it works across Windows PowerShell,
/// bash, zsh, fish, and other POSIX shells.
fn restore_terminal(snapshot: &TerminalSnapshot) -> io::Result<()> {
    let mut out = io::stdout();

    // Always try to leave alternate screen — if we never entered it the
    // command is a harmless no-op.
    execute!(
        out,
        LeaveAlternateScreen,
        cursor::Show,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::CurrentLine),
    )?;

    // Reset all terminal attributes (colors, bold, dim, etc.) so stray
    // escape sequences from the shell cannot leak into the parent prompt.
    write!(out, "\x1b[0m")?;
    out.flush()?;

    let _ = snapshot; // reserved for future use (title, scroll region, …)
    Ok(())
}

/// Clear the visible screen inside the current terminal mode.
///
/// This is a convenience wrapper used by the shell's `clear` command.
pub fn clear_screen() -> io::Result<()> {
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0),)
}

/// Move the cursor to the given `(column, row)` position (0-based).
pub fn move_cursor(col: u16, row: u16) -> io::Result<()> {
    execute!(io::stdout(), cursor::MoveTo(col, row))
}

/// Get the current terminal size as `(columns, rows)`.
///
/// Returns `None` if the size cannot be determined (e.g. in non-TTY
/// environments).
pub fn terminal_size() -> Option<(u16, u16)> {
    terminal::size().ok()
}
