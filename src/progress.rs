//! Progress reporting on stderr: a live bar on terminals, plain lines otherwise.
//!
//! stdout is reserved for reports and the cd path, so progress never touches it.

use std::io::IsTerminal;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// How progress is rendered for this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// Animated spinner/bar redrawn in place (stderr is a terminal).
    Bar,
    /// One line per phase and per item (non-terminal stderr, or `-v` logging).
    Lines,
    /// Nothing (`--quiet`).
    Hidden,
}

impl Display {
    pub fn detect(quiet: bool, verbose: u8) -> Self {
        if quiet {
            Self::Hidden
        } else if verbose == 0 && std::io::stderr().is_terminal() {
            Self::Bar
        } else {
            // Logs interleave cleanly with lines but would garble a redrawn bar.
            Self::Lines
        }
    }
}

const TICK: Duration = Duration::from_millis(100);
const SPINNER_TEMPLATE: &str = "{spinner:.cyan} {wide_msg}";
const BAR_TEMPLATE: &str = "{spinner:.cyan} [{bar:24.cyan/blue}] {pos}/{len} {wide_msg}";

/// Progress for one command. Starts as a spinner for open-ended phases; `items`
/// switches to a counted bar. The bar is cleared on drop, so reports printed
/// afterwards are never interleaved with it.
#[derive(Debug)]
pub struct Progress {
    display: Display,
    /// Hidden draw target in `Lines`/`Hidden` mode; still tracks position/length.
    bar: ProgressBar,
}

impl Progress {
    pub fn new(display: Display) -> Self {
        let bar = match display {
            Display::Bar => {
                let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
                bar.set_style(style(SPINNER_TEMPLATE));
                bar.enable_steady_tick(TICK);
                bar
            }
            Display::Lines | Display::Hidden => ProgressBar::hidden(),
        };
        Self { display, bar }
    }

    /// Announce an open-ended phase (fetching, querying GitHub, ...).
    pub fn phase(&self, msg: impl Into<String>) {
        let msg = msg.into();
        match self.display {
            Display::Bar => self.bar.set_message(msg),
            Display::Lines => eprintln!("{msg}..."),
            Display::Hidden => {}
        }
    }

    /// Switch to counting `total` items.
    pub fn items(&self, total: usize) {
        let total = u64::try_from(total).unwrap_or(u64::MAX);
        self.bar.set_length(total);
        self.bar.set_position(0);
        if self.display == Display::Bar {
            self.bar.set_style(style(BAR_TEMPLATE));
        }
    }

    /// Start the next item; pair with `item_done`.
    pub fn item(&self, label: impl Into<String>) {
        let label = label.into();
        match self.display {
            Display::Bar => self.bar.set_message(label),
            Display::Lines => eprintln!(
                "[{}/{}] {label}",
                self.bar.position() + 1,
                self.bar.length().unwrap_or(0)
            ),
            Display::Hidden => {}
        }
    }

    /// What the current item is doing right now. Only drawn on the live bar: as
    /// lines it would be too chatty (use `-v` for that level of detail).
    pub fn detail(&self, msg: impl Into<String>) {
        if self.display == Display::Bar {
            self.bar.set_message(msg.into());
        }
    }

    pub fn item_done(&self) {
        self.bar.inc(1);
    }

    /// Print a line to stderr without corrupting the bar.
    pub fn println(&self, msg: impl AsRef<str>) {
        match self.display {
            Display::Bar => self.bar.suspend(|| eprintln!("{}", msg.as_ref())),
            Display::Lines | Display::Hidden => eprintln!("{}", msg.as_ref()),
        }
    }

    /// Remove the bar before printing final output.
    pub fn finish(&self) {
        self.bar.finish_and_clear();
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.bar.finish_and_clear();
    }
}

fn style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .progress_chars("=> ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_wins() {
        assert_eq!(Display::detect(true, 0), Display::Hidden);
        assert_eq!(Display::detect(true, 2), Display::Hidden);
    }

    #[test]
    fn verbose_never_draws_a_bar() {
        assert_eq!(Display::detect(false, 1), Display::Lines);
    }

    #[test]
    fn hidden_progress_still_counts() {
        let p = Progress::new(Display::Hidden);
        p.items(3);
        p.item("a");
        p.item_done();
        p.item("b");
        p.item_done();
        assert_eq!(p.bar.position(), 2);
        assert_eq!(p.bar.length(), Some(3));
    }

    #[test]
    fn templates_parse() {
        assert!(ProgressStyle::with_template(SPINNER_TEMPLATE).is_ok());
        assert!(ProgressStyle::with_template(BAR_TEMPLATE).is_ok());
    }
}
