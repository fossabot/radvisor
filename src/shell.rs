use crate::cli::{Opts, ParseFailure};
use std::fmt;
use std::io::{self, Write};
use std::sync::Mutex;

use atty;
use termcolor::{self, Color, ColorSpec, StandardStream, WriteColor};

/// Inspiration/partial implementations taken from the Cargo source at
/// [cargo/core/shell.rs](https://github.com/rust-lang/cargo/blob/53094e32b11c57a917f3ec3a48f29f388583ca3b/src/cargo/core/shell.rs)

/// Maximum length of status string when being justified
const JUSTIFY_STATUS_LEN: usize = 12usize;

/// The requested verbosity of the program output
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verbosity {
    Verbose,
    Normal,
    Quiet,
}

impl Verbosity {
    /// Determines the appropriate verbosity setting for the specified CLI
    /// options
    fn from_opts(opts: &Opts) -> Self {
        match opts.quiet {
            true => Verbosity::Quiet,
            false => match opts.verbose {
                true => Verbosity::Verbose,
                false => Verbosity::Normal,
            },
        }
    }
}

/// Mode of the color output of the process, controllable via a CLI flag
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl std::str::FromStr for ColorMode {
    type Err = ParseFailure;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(ColorMode::Auto),
            "always" => Ok(ColorMode::Always),
            "never" => Ok(ColorMode::Never),
            _ => Err(ParseFailure::new(String::from("color mode"), s.to_owned())),
        }
    }
}

impl ColorMode {
    fn into_termcolor(self, stream: atty::Stream) -> termcolor::ColorChoice {
        match self {
            ColorMode::Always => termcolor::ColorChoice::Always,
            ColorMode::Never => termcolor::ColorChoice::Never,
            ColorMode::Auto => {
                if atty::is(stream) {
                    termcolor::ColorChoice::Auto
                } else {
                    termcolor::ColorChoice::Never
                }
            },
        }
    }
}

/// Thread-safe handle to formatted stderr/stdout output
pub struct Shell {
    verbosity: Verbosity,
    out:       Mutex<OutSink>,
    err:       Mutex<OutSink>,
}

#[allow(dead_code)]
impl Shell {
    /// Creates a new instance of the Shell handle, initializing all fields from
    /// the CLI options as necessary. Should only be called once per process.
    pub fn new(opts: &Opts) -> Self {
        Shell {
            verbosity: Verbosity::from_opts(opts),
            out:       Mutex::new(OutSink::Stream {
                color_mode:  opts.color_mode,
                is_tty:      atty::is(atty::Stream::Stdout),
                stream_type: atty::Stream::Stdout,
                stream:      StandardStream::stdout(
                    opts.color_mode.into_termcolor(atty::Stream::Stdout),
                ),
            }),
            err:       Mutex::new(OutSink::Stream {
                color_mode:  opts.color_mode,
                is_tty:      atty::is(atty::Stream::Stderr),
                stream_type: atty::Stream::Stderr,
                stream:      StandardStream::stderr(
                    opts.color_mode.into_termcolor(atty::Stream::Stderr),
                ),
            }),
        }
    }

    /// Creates a shell from plain writable objects, with no color, and max
    /// verbosity.
    pub fn from_write(stdout: Box<dyn Write + Send>, stderr: Box<dyn Write + Send>) -> Self {
        Shell {
            out:       Mutex::new(OutSink::Write(stdout)),
            err:       Mutex::new(OutSink::Write(stderr)),
            verbosity: Verbosity::Verbose,
        }
    }

    /// Shortcut to right-align and color green a status message.
    pub fn status<T, U>(&mut self, status: T, message: U) -> ()
    where
        T: fmt::Display,
        U: fmt::Display,
    {
        self.print(&status, Some(&message), Color::Green, true);
    }

    pub fn status_header<T>(&mut self, status: T) -> ()
    where
        T: fmt::Display,
    {
        self.print(&status, None, Color::Cyan, true);
    }

    /// Prints a message, where the status will have `color` color, and can be
    /// justified. The messages follows without color.
    fn print(
        &mut self,
        status: &dyn fmt::Display,
        message: Option<&dyn fmt::Display>,
        color: Color,
        justified: bool,
    ) -> () {
        match self.verbosity {
            Verbosity::Quiet => (),
            _ => {
                let mut out = self
                    .out
                    .lock()
                    .expect("Could not unwrap stdout lock: mutex poisoned");
                let _ = out.print(status, message, color, justified);
            },
        }
    }

    /// Prints a red 'error' message.
    pub fn error<T: fmt::Display>(&mut self, message: T) -> () {
        let mut err = self
            .err
            .lock()
            .expect("Could not unwrap stderr lock: mutex poisoned");
        let _ = err.print(&"(error)", Some(&message), Color::Red, true);
    }

    /// Prints an amber 'warning' message.
    pub fn warn<T: fmt::Display>(&mut self, message: T) -> () {
        match self.verbosity {
            Verbosity::Quiet => (),
            _ => self.print(&"(warning)", Some(&message), Color::Yellow, true),
        };
    }

    /// Prints a cyan 'note' message.
    pub fn note<T: fmt::Display>(&mut self, message: T) -> () {
        self.print(&"(note)", Some(&message), Color::Cyan, true);
    }

    /// Gets the current color mode.
    ///
    /// If we are not using a color stream, this will always return `Never`,
    /// even if the color mode has been set to something else.
    pub fn color_mode(&self) -> ColorMode {
        let out = self
            .out
            .lock()
            .expect("Could not unwrap stdout lock: mutex poisoned");
        match *out {
            OutSink::Stream { color_mode, .. } => color_mode,
            OutSink::Write(_) => ColorMode::Never,
        }
    }

    /// Whether the shell supports color.
    pub fn supports_color(&self) -> bool {
        let out = self
            .out
            .lock()
            .expect("Could not unwrap stdout lock: mutex poisoned");
        match &*out {
            OutSink::Write(_) => false,
            OutSink::Stream { stream, .. } => stream.supports_color(),
        }
    }
}

enum OutSink {
    Write(Box<dyn Write + Send>),
    Stream {
        color_mode:  ColorMode,
        stream:      StandardStream,
        stream_type: atty::Stream,
        is_tty:      bool,
    },
}

impl OutSink {
    /// Prints out a message with a status. The status comes first, and is bold
    /// plus the given color. The status can be justified, in which case the
    /// max width that will right align is JUSTIFY_STATUS_LEN chars.
    fn print(
        &mut self,
        status: &dyn fmt::Display,
        message: Option<&dyn fmt::Display>,
        color: Color,
        justified: bool,
    ) -> io::Result<()> {
        let width: Option<usize> = self.width();
        match *self {
            OutSink::Stream { ref mut stream, is_tty,.. } => {
                stream.reset()?;
                stream.set_color(ColorSpec::new().set_bold(true).set_fg(Some(color)))?;

                // Calculate the offset based on the line header
                let offset = if justified && is_tty {
                    write!(stream, "{:>width$}", status, width = JUSTIFY_STATUS_LEN)?;
                    JUSTIFY_STATUS_LEN
                } else {
                    let status_str = format!("{}", status);
                    write!(stream, "{}", status_str)?;
                    stream.set_color(ColorSpec::new().set_bold(true))?;
                    write!(stream, ":")?;
                    status_str.len() + 1
                };

                stream.reset()?;
                match message {
                    None => write!(stream, " ")?,
                    Some(message) => {
                        // If width can be found, then wrap/indent
                        match width {
                            None => writeln!(stream, " {}", message)?,
                            Some(width) => {
                                let formatted: String = format!("{}", message);
                                let lines = textwrap::wrap_iter(&formatted, width - (offset + 1));
                                let mut is_first = true;
                                let indent = " ".repeat(offset);
                                for line in lines {
                                    if is_first {
                                        is_first = false;
                                        writeln!(stream, " {}", line)?;
                                    } else {
                                        writeln!(stream, "{} {}", indent, line)?;
                                    }
                                }
                            },
                        }
                    },
                }
            },
            OutSink::Write(ref mut w) => {
                if justified {
                    write!(w, "{:width$}", status, width = JUSTIFY_STATUS_LEN)?;
                } else {
                    write!(w, "{}:", status)?;
                }
                match message {
                    Some(message) => writeln!(w, " {}", message)?,
                    None => write!(w, " ")?,
                }
            },
        }
        Ok(())
    }

    /// Gets width of terminal, if applicable
    fn width(&self) -> Option<usize> {
        match self {
            OutSink::Stream {
                is_tty: true,
                stream_type,
                ..
            } => imp::width(*stream_type),
            _ => None,
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::mem;

    pub fn width(stream: atty::Stream) -> Option<usize> {
        unsafe {
            let mut winsize: libc::winsize = mem::zeroed();

            // Resolve correct fileno for the stream type
            let fileno = match stream {
                atty::Stream::Stdout => libc::STDOUT_FILENO,
                _ => libc::STDERR_FILENO,
            };

            // The .into() here is needed for FreeBSD which defines TIOCGWINSZ
            // as c_uint but ioctl wants c_ulong.
            if libc::ioctl(fileno, libc::TIOCGWINSZ.into(), &mut winsize) < 0 {
                return None;
            }
            if winsize.ws_col > 0 {
                Some(winsize.ws_col as usize)
            } else {
                None
            }
        }
    }
}

// Package is not Windows-compatible
#[cfg(windows)]
mod imp {
    pub fn width(_stream: atty::Stream) -> Option<usize> { None }
}
