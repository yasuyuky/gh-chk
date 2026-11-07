use crossterm::{
    cursor::Show,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::{self, Write};

pub(super) fn run(
    run_app: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()>,
) -> io::Result<()> {
    enable_raw_mode()?;
    with_restore(
        || {
            let mut stdout = io::stdout();
            execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
            let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
            run_app(&mut terminal)
        },
        || restore_terminal(&mut io::stdout(), disable_raw_mode),
    )
}

fn restore_terminal(
    writer: &mut impl Write,
    disable_raw: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    // Attempt every restoration step even if an earlier one fails.
    let raw = disable_raw();
    let screen = execute!(writer, LeaveAlternateScreen);
    let mouse = execute!(writer, DisableMouseCapture);
    let cursor = execute!(writer, Show);
    raw.and(screen).and(mouse).and(cursor)
}

fn with_restore(
    operation: impl FnOnce() -> io::Result<()>,
    restore: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let mut guard = RestoreOnDrop(Some(restore));
    let result = operation();
    let restored = guard.restore();
    // Preserve the initialization or application error if cleanup also fails.
    result.and(restored)
}

struct RestoreOnDrop<F: FnOnce() -> io::Result<()>>(Option<F>);

impl<F: FnOnce() -> io::Result<()>> RestoreOnDrop<F> {
    fn restore(&mut self) -> io::Result<()> {
        match self.0.take() {
            Some(restore) => restore(),
            None => Ok(()),
        }
    }
}

impl<F: FnOnce() -> io::Result<()>> Drop for RestoreOnDrop<F> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn restores_after_success_and_errors() {
        for operation_fails in [false, true] {
            for restore_fails in [false, true] {
                let restores = Cell::new(0);
                let result = with_restore(
                    || {
                        if operation_fails {
                            Err(io::Error::other("operation failed"))
                        } else {
                            Ok(())
                        }
                    },
                    || {
                        restores.set(restores.get() + 1);
                        if restore_fails {
                            Err(io::Error::other("restore failed"))
                        } else {
                            Ok(())
                        }
                    },
                );
                assert_eq!(restores.get(), 1);
                match (operation_fails, restore_fails) {
                    (true, _) => assert_eq!(result.unwrap_err().to_string(), "operation failed"),
                    (false, true) => assert_eq!(result.unwrap_err().to_string(), "restore failed"),
                    (false, false) => result.unwrap(),
                }
            }
        }
    }

    #[test]
    fn restores_during_panic_unwinding() {
        let restored = Cell::new(false);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = with_restore(
                || panic!("drawing panicked"),
                || {
                    restored.set(true);
                    Ok(())
                },
            );
        }));
        assert!(result.is_err());
        assert!(restored.get());
    }

    #[cfg(unix)]
    #[derive(Default)]
    struct FailingWriter {
        bytes: Vec<u8>,
        flushes: usize,
        fail_on_flush: usize,
    }

    #[cfg(unix)]
    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.flushes == self.fail_on_flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn attempts_all_restoration_steps_despite_errors() {
        for raw_fails in [false, true] {
            for fail_on_flush in 0..=3 {
                let mut writer = FailingWriter {
                    fail_on_flush,
                    ..Default::default()
                };
                let raw_restored = Cell::new(false);
                let result = restore_terminal(&mut writer, || {
                    raw_restored.set(true);
                    if raw_fails {
                        Err(io::Error::other("raw mode restore failed"))
                    } else {
                        Ok(())
                    }
                });
                assert!(raw_restored.get());
                let output = String::from_utf8(writer.bytes).unwrap();
                assert!(output.contains("\x1b[?1049l"), "leave alternate screen");
                assert!(output.contains("\x1b[?1000l"), "disable mouse capture");
                assert!(output.ends_with("\x1b[?25h"), "show cursor");
                if raw_fails {
                    assert_eq!(result.unwrap_err().to_string(), "raw mode restore failed");
                } else if fail_on_flush > 0 {
                    assert_eq!(result.unwrap_err().to_string(), "flush failed");
                } else {
                    result.unwrap();
                }
            }
        }
    }
}
