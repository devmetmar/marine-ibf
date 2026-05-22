//! Discovering and validating OFS baserun directories under `ofs-latest`.
//!
//! Risk scoring only reads `wave.zarr` variables **`hs`**, **`uwnd`**, **`vwnd`**.
//! Grid size is **not fixed** — it follows forecaster/model config in `metadata.json`.
//!
//! Supported `wave_model` values (see `scratch/check_var_input.ipynb`):
//! - **`INACAWO_WAVES`** — e.g. baserun `202605180000`, grid `(241, 1200, 2200)`
//! - **`INAWAVES`** — e.g. baserun `202605191200`, grid `(128, 480, 880)`
//!
//! `flow.zarr` / `atmos.zarr` use other models/grids and are ignored by this worker.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::manifest;

/// Variables required for risk scoring (wave height + wind components).
pub const RISK_INPUT_VARS: &[&str] = &["hs", "uwnd", "vwnd"];

/// Coordinate arrays copied to output and expected on input wave store.
pub const RISK_INPUT_COORDS: &[&str] = &["lat", "lon", "time"];

/// Wave models whose `wave.zarr` layout this worker supports.
pub const SUPPORTED_WAVE_MODELS: &[&str] = &["INACAWO_WAVES", "INAWAVES"];

/// Written to `conversion.log` when the OFS zarr updater finishes a baserun.
const CONVERSION_DONE_MARKER: &str = "--DONE-- : Zarr update process completed successfully";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaserunScan {
    pub id: String,
    pub path: PathBuf,
    pub wave_model: String,
    pub grid: WaveGridShape,
}

/// Parsed fields from `<baserun>/metadata.json`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BaserunMetadata {
    pub baserun: Option<String>,
    pub wave_model: Option<String>,
    pub flow_model: Option<String>,
    pub atmos_model: Option<String>,
    pub wave_size_bytes: Option<u64>,
    pub flow_size_bytes: Option<u64>,
    pub atmos_size_bytes: Option<u64>,
}

/// Shape of `hs` / `uwnd` / `vwnd` discovered for one baserun (`[time, lat, lon]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveGridShape {
    pub shape: Vec<u64>,
}

pub fn is_supported_wave_model(model: &str) -> bool {
    SUPPORTED_WAVE_MODELS.contains(&model)
}

pub fn read_baserun_metadata(baserun_dir: &Path) -> Result<BaserunMetadata> {
    let meta_path = baserun_dir.join("metadata.json");
    let text = fs::read_to_string(&meta_path)
        .with_context(|| format!("read {}", meta_path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", meta_path.display()))
}

pub fn wave_zarr_path(baserun_dir: &Path) -> PathBuf {
    baserun_dir.join("wave.zarr")
}

pub fn is_baserun_id(name: &str) -> bool {
    name.len() == 12 && name.chars().all(|c| c.is_ascii_digit())
}

fn read_zarray_meta(var_dir: &Path) -> Result<(Vec<u64>, Vec<u64>)> {
    let zarray = var_dir.join(".zarray");
    let txt = fs::read_to_string(&zarray)
        .with_context(|| format!("read {}", zarray.display()))?;
    let v: Value = serde_json::from_str(&txt)
        .with_context(|| format!("parse {}", zarray.display()))?;
    let shape = json_u64_array(v.get("shape"), "shape")?;
    let chunks = json_u64_array(v.get("chunks"), "chunks")?;
    Ok((shape, chunks))
}

fn json_u64_array(value: Option<&Value>, field: &str) -> Result<Vec<u64>> {
    let arr = value
        .and_then(|v| v.as_array())
        .with_context(|| format!("missing array field `{field}` in .zarray"))?;
    arr.iter()
        .map(|x| {
            x.as_u64()
                .with_context(|| format!("invalid u64 in `{field}`"))
        })
        .collect()
}

fn expected_chunk_count(shape: &[u64], chunks: &[u64]) -> Result<u64> {
    if shape.len() != chunks.len() {
        bail!("shape/chunks rank mismatch");
    }
    Ok(shape
        .iter()
        .zip(chunks.iter())
        .map(|(s, c)| (*s + *c - 1) / *c)
        .product())
}

fn count_data_chunk_files(var_dir: &Path) -> Result<u64> {
    let mut n = 0u64;
    for entry in fs::read_dir(var_dir)
        .with_context(|| format!("read_dir {}", var_dir.display()))?
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != ".zarray" && name != ".zattrs" {
            n += 1;
        }
    }
    Ok(n)
}

/// All Zarr chunk objects exist for one variable (guards against mid-write conversion).
fn variable_has_all_chunks(var_dir: &Path) -> Result<bool> {
    if !var_dir.join(".zarray").is_file() {
        return Ok(false);
    }
    let (shape, chunks) = read_zarray_meta(var_dir)?;
    let expected = expected_chunk_count(&shape, &chunks)?;
    let actual = count_data_chunk_files(var_dir)?;
    Ok(actual == expected)
}

fn wave_var_dir(wave_root: &Path, name: &str) -> PathBuf {
    wave_root.join(name)
}

fn coord_lengths_match_grid(wave_root: &Path, grid_shape: &[u64]) -> bool {
    if grid_shape.len() < 3 {
        return false;
    }
    let expected = [("time", 0), ("lat", 1), ("lon", 2)];
    for (coord, dim_idx) in expected {
        let dir = wave_var_dir(wave_root, coord);
        let Ok((coord_shape, _)) = read_zarray_meta(&dir) else {
            return false;
        };
        if coord_shape.len() != 1 || coord_shape[0] != grid_shape[dim_idx] {
            return false;
        }
    }
    true
}

/// Read the shared `(time, lat, lon)` shape from `hs` (must match `uwnd`/`vwnd`).
pub fn discover_wave_grid_shape(wave_root: &Path) -> Result<WaveGridShape> {
    let hs_dir = wave_var_dir(wave_root, "hs");
    let (shape, _) = read_zarray_meta(&hs_dir).context("read hs .zarray")?;
    for var in &RISK_INPUT_VARS[1..] {
        let dir = wave_var_dir(wave_root, var);
        let (other, _) = read_zarray_meta(&dir)
            .with_context(|| format!("read {var} .zarray"))?;
        if other != shape {
            bail!(
                "shape mismatch: hs {shape:?} vs {var} {other:?} under {}",
                wave_root.display()
            );
        }
    }
    Ok(WaveGridShape { shape })
}

/// Wave store has `hs`, `uwnd`, `vwnd` with identical shape (any grid size) and all chunks on disk.
pub fn has_complete_wave(wave_root: &Path) -> bool {
    if !wave_root.is_dir() || !wave_root.join(".zgroup").is_file() {
        return false;
    }
    // Consolidated metadata is usually written when OFS conversion finishes (optional signal).
    // Chunk completeness is the main guard against mid-write processing.

    let Ok(grid) = discover_wave_grid_shape(wave_root) else {
        return false;
    };

    for var in RISK_INPUT_VARS {
        let dir = wave_var_dir(wave_root, var);
        match variable_has_all_chunks(&dir) {
            Ok(true) => {}
            _ => return false,
        }
    }

    if !coord_lengths_match_grid(wave_root, &grid.shape) {
        return false;
    }

    true
}

fn conversion_log_complete(baserun_dir: &Path) -> bool {
    let log_path = baserun_dir.join("conversion.log");
    let Ok(text) = fs::read_to_string(&log_path) else {
        return false;
    };
    text.contains(CONVERSION_DONE_MARKER)
}

fn metadata_ready(baserun_dir: &Path, id: &str) -> bool {
    let Ok(parsed) = read_baserun_metadata(baserun_dir) else {
        return false;
    };
    match parsed.baserun.as_deref() {
        Some(b) if b != id => return false,
        None => {}
        Some(_) => {}
    }
    let Some(ref wave_model) = parsed.wave_model else {
        return false;
    };
    if !is_supported_wave_model(wave_model) {
        return false;
    }
    if matches!(parsed.wave_size_bytes, Some(0)) {
        return false;
    }
    true
}

fn build_baserun_scan(path: PathBuf, id: String) -> Result<BaserunScan> {
    let meta = read_baserun_metadata(&path)?;
    let wave_model = meta
        .wave_model
        .filter(|m| is_supported_wave_model(m))
        .with_context(|| format!("unsupported or missing wave_model in {}", path.display()))?;
    let grid = discover_wave_grid_shape(&wave_zarr_path(&path))?;
    Ok(BaserunScan {
        id,
        path,
        wave_model,
        grid,
    })
}

/// True when baserun folder is safe to process (not mid-conversion).
pub fn is_complete_input_baserun(baserun_dir: &Path, id: &str) -> bool {
    if !is_baserun_id(id) || !baserun_dir.is_dir() {
        return false;
    }
    if !conversion_log_complete(baserun_dir) {
        return false;
    }
    if !metadata_ready(baserun_dir, id) {
        return false;
    }
    has_complete_wave(&wave_zarr_path(baserun_dir))
}

/// Scan input root and return complete baseruns, oldest → newest.
pub fn scan_complete_input_baseruns(input_root: &Path) -> Result<Vec<BaserunScan>> {
    let mut found = Vec::new();

    for entry in fs::read_dir(input_root)
        .with_context(|| format!("read_dir {}", input_root.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
    {
        let id = match entry.file_name().into_string() {
            Ok(name) if is_baserun_id(&name) => name,
            _ => continue,
        };
        let path = entry.path();
        if is_complete_input_baserun(&path, &id) {
            match build_baserun_scan(path, id.clone()) {
                Ok(scan) => found.push(scan),
                Err(e) => eprintln!("skip baserun {id}: {e:#}"),
            }
        }
    }

    found.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(found)
}

/// Newest complete baserun under `input_root`, or error if none.
pub fn scan_latest_complete_input(input_root: &Path) -> Result<BaserunScan> {
    scan_complete_input_baseruns(input_root)?
        .pop()
        .with_context(|| {
            format!(
                "no complete baserun under {} (need finished conversion + wave.zarr/{{hs,uwnd,vwnd}} with all chunks)",
                input_root.display()
            )
        })
}

/// True when `<output_root>/<baserun>/` has published `risk.zarr` + manifest `status: complete`.
pub fn is_output_complete(output_root: &Path, baserun: &str) -> bool {
    manifest::is_complete_output(&output_root.join(baserun))
}

/// Complete input baseruns that do not yet have complete risk output (oldest → newest).
pub fn scan_pending_input_baseruns(
    input_root: &Path,
    output_root: &Path,
) -> Result<Vec<BaserunScan>> {
    Ok(scan_complete_input_baseruns(input_root)?
        .into_iter()
        .filter(|s| !is_output_complete(output_root, &s.id))
        .collect())
}

/// Newest input baserun that is complete but still missing risk output.
pub fn scan_latest_pending_input(input_root: &Path, output_root: &Path) -> Result<BaserunScan> {
    scan_pending_input_baseruns(input_root, output_root)?
        .pop()
        .with_context(|| {
            format!(
                "no pending baserun: every complete input under {} already has risk output under {}",
                input_root.display(),
                output_root.display()
            )
        })
}

pub fn read_hs_fill_f32(wave_root: &Path) -> f32 {
    let p = wave_root.join("hs/.zarray");
    let Ok(txt) = fs::read_to_string(&p) else {
        return -999.9;
    };
    let Ok(v) = serde_json::from_str::<Value>(&txt) else {
        return -999.9;
    };
    if let Some(fv) = v.get("fill_value") {
        if let Some(n) = fv.as_f64() {
            return n as f32;
        }
        if let Some(s) = fv.as_str() {
            if s.eq_ignore_ascii_case("nan") || s == "NaN" {
                return f32::NAN;
            }
        }
    }
    -999.9
}

/// Baserun ids to process: all pending (oldest first), or one forced baserun.
pub fn baseruns_to_process(
    input_root: &Path,
    output_root: &Path,
    forced: Option<String>,
) -> Result<Vec<String>> {
    if let Some(b) = forced {
        let p = input_root.join(&b);
        if !p.is_dir() {
            bail!("baserun directory not found: {}", p.display());
        }
        if !is_complete_input_baserun(&p, &b) {
            bail!(
                "baserun {} is incomplete or still being written under {}",
                b,
                p.display()
            );
        }
        return Ok(vec![b]);
    }

    let pending = scan_pending_input_baseruns(input_root, output_root)?;
    Ok(pending.into_iter().map(|s| s.id).collect())
}

/// Newest pending baserun id, or `forced` if set (input must be complete).
pub fn pick_baserun(
    input_root: &Path,
    output_root: &Path,
    forced: Option<String>,
) -> Result<String> {
    Ok(baseruns_to_process(input_root, output_root, forced)?
        .last()
        .cloned()
        .with_context(|| "no baserun selected")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baserun_id_format() {
        assert!(is_baserun_id("202605191200"));
        assert!(!is_baserun_id("2026051912"));
        assert!(!is_baserun_id("not-a-baserun"));
    }

    #[test]
    fn supported_wave_models() {
        assert!(is_supported_wave_model("INACAWO_WAVES"));
        assert!(is_supported_wave_model("INAWAVES"));
        assert!(!is_supported_wave_model("WW3"));
    }

    #[test]
    fn expected_chunk_count_inawaves_grid() {
        // Illustrative only — actual baserun grids vary by forecaster/model config.
        let shape = vec![128, 480, 880];
        let chunks = vec![1, 200, 200];
        assert_eq!(expected_chunk_count(&shape, &chunks).unwrap(), 1920);
    }
}
