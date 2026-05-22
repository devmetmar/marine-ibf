use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "risk_score_worker")]
pub struct Cli {
    /// Root containing one directory per baserun (e.g. `202605191200`)
    #[arg(long, default_value = "/data/raid0/zarr/ofs-latest")]
    pub input_root: PathBuf,

    /// Output root; results go to `<output_root>/<baserun>/risk.zarr`
    #[arg(long, default_value = "/data/raid0/zarr/risk_score")]
    pub output_root: PathBuf,

    /// Force a baserun (YYYYMMDDHHMM). If omitted, pick newest complete run under input_root.
    #[arg(long)]
    pub baserun: Option<String>,

    /// Dry run: only print chosen baserun
    #[arg(long)]
    pub dry_run: bool,

    /// Keep this many newest baserun outputs; older dirs are removed after a successful run
    #[arg(long, default_value_t = 3)]
    pub retention: usize,

    /// Skip garbage collection of old output baserun folders
    #[arg(long)]
    pub no_gc: bool,

    /// Poll interval in seconds (0 = run once and exit). When > 0, repeatedly scan for new complete input baseruns missing risk output.
    #[arg(long, default_value_t = 0)]
    pub interval_secs: u64,
}
