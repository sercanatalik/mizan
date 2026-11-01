//! `mizan-cli`: command-line tools for Mizan imports.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mizan_core::{
    prepare, report, resolve_batch, CreatePolicy, Decisions, PreviewResolver, ReviewItem,
    ReviewReport,
};
use mizan_import::parse_batch;
use mizan_storage::Store;

#[derive(Parser)]
#[command(
    name = "mizan-cli",
    version,
    about = "Command-line tools for Mizan imports"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Work with mizan.import/1 batches.
    #[command(subcommand)]
    Import(ImportCommand),
}

#[derive(Subcommand)]
enum ImportCommand {
    /// Check that a batch file is valid mizan.import/1. Exits non-zero on any error.
    Validate {
        /// Path to the batch JSON.
        batch: PathBuf,
    },
    /// Show the review report for a batch against a database, without committing anything.
    Preview {
        /// Path to the batch JSON.
        batch: PathBuf,
        /// SQLite database. If it does not exist, an empty one is assumed and no file is created.
        #[arg(long)]
        db: PathBuf,
        /// Treat unknown accounts and instruments as new instead of unresolved.
        #[arg(long)]
        create_missing: bool,
        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Import(ImportCommand::Validate { batch }) => validate(&batch),
        Command::Import(ImportCommand::Preview {
            batch,
            db,
            create_missing,
            json,
        }) => preview(&batch, &db, create_missing, json),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

fn read(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn validate(path: &PathBuf) -> Result<ExitCode, String> {
    let parsed = parse_batch(&read(path)?).map_err(|e| e.to_string())?;
    println!(
        "{} records, {} assertions, {} diagnostics",
        parsed.records.len(),
        parsed.assertions.len(),
        parsed.diagnostics.len()
    );
    if parsed.is_clean() {
        println!("OK: valid mizan.import/1 batch");
        return Ok(ExitCode::SUCCESS);
    }
    println!("{} invalid item(s):", parsed.errors.len());
    for e in &parsed.errors {
        println!("  {e}");
    }
    Ok(ExitCode::FAILURE)
}

fn preview(
    path: &PathBuf,
    db: &PathBuf,
    create_missing: bool,
    json: bool,
) -> Result<ExitCode, String> {
    let parsed = parse_batch(&read(path)?).map_err(|e| e.to_string())?;
    let store = if db.exists() {
        Store::open(db)
    } else {
        Store::open_in_memory()
    }
    .map_err(|e| e.to_string())?;
    let policy = CreatePolicy {
        create_accounts: create_missing,
        create_assets: create_missing,
    };
    let resolved = {
        let mut resolver = PreviewResolver::new(&store, policy);
        resolve_batch(parsed, &mut resolver).map_err(|e| e.to_string())?
    };
    let prepared = prepare(&store, resolved).map_err(|e| e.to_string())?;
    let rep = report(&store, &prepared, &Decisions::new()).map_err(|e| e.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&rep).map_err(|e| e.to_string())?
        );
    } else {
        print_report(&rep);
    }
    println!("(preview only: nothing was written)");
    Ok(ExitCode::SUCCESS)
}

fn print_report(r: &ReviewReport) {
    let s = &r.summary;
    println!(
        "New {} · Duplicates {} · Need decision {} · Figure warnings {} · Not imported {} · Invalid {}",
        s.new, s.duplicates, s.pending_confirmations, s.consistency_warnings, s.adapter_diagnostics, s.validation_errors
    );
    let section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            println!("\n{title}");
            for l in lines {
                println!("  {l}");
            }
        }
    };
    section(
        "New records",
        r.new_records
            .iter()
            .map(|n| {
                format!(
                    "{:<9} {} {:<10} {} {}",
                    n.locator,
                    n.date,
                    n.kind,
                    n.instrument.as_deref().unwrap_or(""),
                    n.net_cash.as_deref().unwrap_or("")
                )
            })
            .collect(),
    );
    section(
        "Need your decision",
        r.confirmations
            .iter()
            .map(|c| {
                format!(
                    "{}: same as {} in file {}…",
                    c.record.locator,
                    c.existing_locator,
                    &c.existing_file_sha256[..8]
                )
            })
            .collect(),
    );
    section(
        "Figure warnings",
        r.consistency_warnings
            .iter()
            .map(|w| format!("{}: {}", w.locator, w.message))
            .collect(),
    );
    section(
        "Ordering",
        r.ordering_diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect(),
    );
    section(
        "Not imported (adapter diagnostics)",
        r.adapter_diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.locator, d.message))
            .collect(),
    );
    section("Invalid items", r.validation_errors.clone());
    section(
        "Unresolved",
        r.unresolved
            .iter()
            .map(|u| match u {
                ReviewItem::UnresolvedAccount { hint } => format!("account {}", hint.hint),
                ReviewItem::UnresolvedInstrument { key, .. } => {
                    format!("instrument {}", key.as_deref().unwrap_or("?"))
                }
            })
            .collect(),
    );
    section(
        "Balance checks",
        r.assertion_results
            .iter()
            .flat_map(|a| {
                let head = format!(
                    "{} {} as of {}{}",
                    if a.passed { "PASS" } else { "FAIL" },
                    a.account,
                    a.as_of,
                    a.note
                        .as_deref()
                        .map(|n| format!(" ({n})"))
                        .unwrap_or_default()
                );
                std::iter::once(head).chain(a.lines.iter().map(|l| {
                    format!(
                        "  {} {}: statement {}, computed {}",
                        if l.passed { "ok " } else { "!! " },
                        l.label,
                        l.expected,
                        l.computed
                    )
                }))
            })
            .collect(),
    );
    section(
        "Adjustments (flagged)",
        r.flagged_adjustments
            .iter()
            .map(|a| format!("{}: {}", a.locator, a.memo.as_deref().unwrap_or("")))
            .collect(),
    );
    section(
        "Unknown cost basis",
        r.basis_unknown
            .iter()
            .map(|b| format!("{} {}", b.locator, b.instrument.as_deref().unwrap_or("")))
            .collect(),
    );
}
