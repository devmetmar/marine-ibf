//! CLI entry: delegates to `risk_score_worker` library.

use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use risk_score_worker::cli::Cli;
use risk_score_worker::{
    baseruns_to_process, gc_old_outputs, process_baserun, scan_pending_input_baseruns,
};

fn run_once(cli: &Cli) -> Result<bool> {
    let pending = baseruns_to_process(&cli.input_root, &cli.output_root, cli.baserun.clone())?;

    if pending.is_empty() {
        if cli.baserun.is_none() {
            eprintln!(
                "[risk_score] IDLE — tidak ada baserun input baru yang perlu diproses \
                 (semua baserun lengkap sudah punya output complete di {})",
                cli.output_root.display()
            );
        }
        return Ok(false);
    }

    if cli.baserun.is_none() {
        let all_pending = scan_pending_input_baseruns(&cli.input_root, &cli.output_root)?;
        eprintln!(
            "[risk_score] found {} pending baserun(s): {}",
            all_pending.len(),
            all_pending
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    for baserun in &pending {
        eprintln!(
            "[risk_score] WORKER_START input={} output={} baserun={baserun}",
            cli.input_root.display(),
            cli.output_root.display()
        );

        process_baserun(&cli.input_root, &cli.output_root, baserun)?;

        eprintln!(
            "[risk_score] WORKER_COMPLETE baserun={baserun} output={}",
            cli.output_root.join(baserun).display()
        );
    }

    if !cli.no_gc {
        let removed = gc_old_outputs(&cli.output_root, cli.retention)?;
        if removed.is_empty() {
            eprintln!(
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
        let pending = baseruns_to_process(&cli.input_root, &cli.output_root, cli.baserun.clone())?;
        for b in pending {
            println!("{b}");
        }
        return Ok(());
    }

    if cli.interval_secs == 0 {
        run_once(&cli)?;
        return Ok(());
    }

    eprintln!(
        "[risk_score] WATCH mode: scan setiap {} detik",
        cli.interval_secs
    );

    loop {
        let _ = run_once(&cli);
        thread::sleep(Duration::from_secs(cli.interval_secs));
    }
}
