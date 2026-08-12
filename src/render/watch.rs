use crate::apply_filters;
use crate::render::table::TableRenderer;
use crate::render::Renderer;
use crate::scanner::PortScanner;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

#[allow(clippy::too_many_arguments)]
/// Run watch mode: continuously scan and display ports, refreshing at the given interval.
/// Press 'q' or Ctrl+C to exit.
pub fn run_watch_mode(
    scanner: &dyn PortScanner,
    interval_secs: u64,
    no_color: bool,
    show_all: bool,
    process_filter: Option<&str>,
    port_filter: Option<u16>,
    range_filter: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let renderer = TableRenderer;
    let refresh = Duration::from_secs(interval_secs);

    // Enter alternate screen
    let mut stdout = stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

    let _guard = TerminalGuard;

    watch_loop(
        &mut stdout,
        scanner,
        &renderer,
        refresh,
        no_color,
        show_all,
        process_filter,
        port_filter,
        range_filter,
    )
}

#[allow(clippy::too_many_arguments)]
fn watch_loop(
    stdout: &mut impl Write,
    scanner: &dyn PortScanner,
    renderer: &TableRenderer,
    refresh: Duration,
    no_color: bool,
    show_all: bool,
    process_filter: Option<&str>,
    port_filter: Option<u16>,
    range_filter: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_scan = Instant::now() - refresh; // force immediate first scan

    loop {
        // Check for user input (non-blocking)
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                match key_event.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }

        // Refresh display if interval has elapsed
        if last_scan.elapsed() >= refresh {
            last_scan = Instant::now();

            let entries = match scanner.scan() {
                Ok(e) => e,
                Err(e) => {
                    execute!(
                        stdout,
                        terminal::Clear(ClearType::All),
                        cursor::MoveTo(0, 0)
                    )?;
                    writeln!(stdout, "Scan error: {}", e)?;
                    continue;
                }
            };

            // Apply filters
            let filtered =
                apply_filters(entries, show_all, process_filter, port_filter, range_filter);

            execute!(
                stdout,
                terminal::Clear(ClearType::All),
                cursor::MoveTo(0, 0)
            )?;

            // Header
            let header = format!(
                " RunCove - watching (refresh: {}s) - press 'q' to quit\n",
                refresh.as_secs()
            );
            if no_color {
                writeln!(stdout, "{}", header)?;
            } else {
                writeln!(stdout, "{}", colored::Colorize::bold(header.as_str()))?;
            }

            // Table
            let output = renderer.render(&filtered, no_color);
            writeln!(stdout, "{}", output)?;

            stdout.flush()?;
        }
    }
}
