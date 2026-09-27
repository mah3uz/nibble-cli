use anstyle::{AnsiColor, Effects, Style};
use clap::builder::styling::Styles;
use std::fmt::Display;

pub const HEADING: Style = AnsiColor::Magenta.on_default().effects(Effects::BOLD);
pub const NAME: Style = AnsiColor::Cyan.on_default();
pub const STRONG: Style = Style::new().effects(Effects::BOLD);
pub const DIM: Style = Style::new().effects(Effects::DIMMED);
pub const GOOD: Style = AnsiColor::Green.on_default().effects(Effects::BOLD);
pub const BAD: Style = AnsiColor::Red.on_default().effects(Effects::BOLD);
pub const CHANGE: Style = AnsiColor::Yellow.on_default().effects(Effects::BOLD);
pub const LINK: Style = AnsiColor::Cyan.on_default().effects(Effects::UNDERLINE);

pub const HELP: Styles =
    Styles::styled().header(HEADING).usage(HEADING).literal(NAME).placeholder(DIM).error(BAD).valid(GOOD).invalid(CHANGE);

// Pad before painting: escape codes would otherwise count towards the width.
pub fn paint(style: Style, text: impl Display) -> String {
    format!("{style}{text}{style:#}")
}

pub fn done(message: impl Display) {
    anstream::eprintln!("{} {message}", paint(GOOD, "✓"));
}

pub fn colour_on_stdout() -> bool {
    anstream::AutoStream::choice(&std::io::stdout()) != anstream::ColorChoice::Never
}
