use clap::Parser;

use crate::apply_filters;
use crate::cli::{Cli, Command};
use crate::process;
use crate::render::json::JsonRenderer;
use crate::render::table::TableRenderer;
use crate::render::Renderer;

/// Run the shared RunCove CLI entrypoint.
pub fn run_cli() {
    let cli = Cli::parse();

    if let Some(command) = &cli.command {
        match command {
            Command::Kill { port, force } => {
                let scanner = crate::scanner::create_scanner();
                match scanner.scan() {
                    Ok(entries) => {
                        if let Err(error) =
                            process::kill_on_port(*port, *force, &entries, cli.no_color)
                        {
                            eprintln!("Error: {error}");
                            std::process::exit(1);
                        }
                    }
                    Err(error) => {
                        eprintln!("Failed to scan ports: {error}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            Command::Open { port } => {
                if let Err(error) = process::open_port(*port) {
                    eprintln!("Error: {error}");
                    std::process::exit(1);
                }
                return;
            }
        }
    }

    let scanner = crate::scanner::create_scanner();

    if cli.watch {
        if let Err(error) = crate::render::watch::run_watch_mode(
            scanner.as_ref(),
            cli.interval,
            cli.no_color,
            cli.all,
            cli.process.as_deref(),
            cli.port,
            cli.range.as_deref(),
        ) {
            eprintln!("Watch mode error: {error}");
            std::process::exit(1);
        }
        return;
    }

    let entries = match scanner.scan() {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Failed to scan ports: {error}");
            std::process::exit(1);
        }
    };

    let filtered = apply_filters(
        entries,
        cli.all,
        cli.process.as_deref(),
        cli.port,
        cli.range.as_deref(),
    );

    let output = if cli.json {
        JsonRenderer.render(&filtered, cli.no_color)
    } else {
        TableRenderer.render(&filtered, cli.no_color)
    };

    println!("{output}");
}
