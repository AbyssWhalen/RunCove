pub mod cli;
pub mod cli_app;
pub mod model;
pub mod process;
pub mod render;
pub mod scanner;

pub use cli_app::run_cli;

use cli::Cli;
use model::{ConnectionState, PortEntry};

/// Apply all filters to a list of port entries.
pub fn apply_filters(
    entries: Vec<PortEntry>,
    show_all: bool,
    process_filter: Option<&str>,
    port_filter: Option<u16>,
    range_filter: Option<&str>,
) -> Vec<PortEntry> {
    // Parse range if provided
    let range = range_filter.and_then(|r| match Cli::parse_range(r) {
        Ok((start, end)) => Some((start, end)),
        Err(e) => {
            eprintln!("Warning: {}", e);
            None
        }
    });

    // Hoist to_lowercase out of the filter closure
    let filter_lower = process_filter.map(|name| name.to_lowercase());

    entries
        .into_iter()
        .filter(|e| {
            // State filter: only show LISTEN by default
            if !show_all && e.state != ConnectionState::Listen {
                return false;
            }

            // Port filter
            if let Some(p) = port_filter {
                if e.port != p {
                    return false;
                }
            }

            // Range filter
            if let Some((start, end)) = range {
                if e.port < start || e.port > end {
                    return false;
                }
            }

            // Process name filter (case-insensitive substring match)
            if let Some(ref lower) = filter_lower {
                if let Some(ref proc_name) = e.process_name {
                    if !proc_name.to_lowercase().contains(lower.as_str()) {
                        return false;
                    }
                } else {
                    return false;
                }
            }

            true
        })
        .collect()
}
