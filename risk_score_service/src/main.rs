//! CLI entry: delegates to `risk_score_worker` library.

use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use risk_score_worker::cli::Cli;
use risk_score_worker::{baseruns_to_process, gc_old_outputs, log, process_baserun};

fn run_once(cli: &Cli) -> Result<bool> {
    let pending = baseruns_to_process(
        &cli.input_root,
        &cli.output_root,
        cli.baserun.clone(),
        cli.retention,
    )?;

    if pending.is_empty() {
        if cli.baserun.is_none() {
            log!(
                "[risk_score] IDLE — tidak ada baserun baru dalam retention={} \
                 (input lengkap tanpa output, atau di luar jendela retention)",
                cli.retention
            );
        }
        return Ok(false);
    }

    let mut processed = Vec::new();

    for baserun in &pending {
        if cli.baserun.is_none() {
            log!("[risk_score] processing newest pending baserun: {baserun}");
        }
        log!(
            "[risk_score] WORKER_START input={} output={} baserun={baserun}",
            cli.input_root.display(),
            cli.output_root.display()
        );

        process_baserun(&cli.input_root, &cli.output_root, baserun)?;

        log!(
            "[risk_score] WORKER_COMPLETE baserun={baserun} output={}",
            cli.output_root.join(baserun).display()
        );
        processed.push(baserun.clone());
    }

    if !cli.no_gc {
        let removed = gc_old_outputs(&cli.output_root, cli.retention, &processed)?;
        if removed.is_empty() {
            log!(
                "[risk_score] GC retention={} — tidak ada output lama yang dihapus",
                cli.retention
            );
        }
    }

    Ok(true)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.dry_run {
        let pending = baseruns_to_process(
            &cli.input_root,
            &cli.output_root,
            cli.baserun.clone(),
            cli.retention,
        )?;
        for b in pending {
            println!("{b}");
        }
        return Ok(());
    }

    if cli.interval_secs == 0 {
        run_once(&cli)?;
        return Ok(());
    }

    log!(
        "[risk_score] WATCH mode: scan setiap {} detik (retention={})",
        cli.interval_secs,
        cli.retention
    );

    loop {
        let _ = run_once(&cli);
        thread::sleep(Duration::from_secs(cli.interval_secs));
    }
}
