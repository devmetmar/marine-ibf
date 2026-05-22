//! Risk score worker library: baserun discovery, wave/wind risk model, Zarr I/O.

pub mod baserun;
pub mod cli;
pub mod manifest;
pub mod pipeline;
pub mod retention;
pub mod risk;
pub mod zarr_store;

pub use baserun::{
    baseruns_to_process, discover_wave_grid_shape, is_output_complete, pick_baserun,
    read_baserun_metadata, scan_complete_input_baseruns, scan_latest_complete_input,
    scan_latest_pending_input, scan_pending_input_baseruns, BaserunMetadata, BaserunScan,
    WaveGridShape, RISK_INPUT_COORDS, RISK_INPUT_VARS, SUPPORTED_WAVE_MODELS,
};
pub use pipeline::process_baserun;
pub use retention::gc_old_outputs;
