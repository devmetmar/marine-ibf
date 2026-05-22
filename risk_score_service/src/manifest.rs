//! Atomic `risk_manifest.json` and publish helpers for safe concurrent readers.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub const RISK_ZARR: &str = "risk.zarr";
pub const RISK_ZARR_STAGING: &str = "risk.zarr.staging";
pub const MANIFEST: &str = "risk_manifest.json";
const MANIFEST_STAGING: &str = "risk_manifest.json.staging";
const MANIFEST_TMP: &str = "risk_manifest.json.tmp";

pub const STATUS_IN_PROGRESS: &str = "in_progress";
pub const STATUS_COMPLETE: &str = "complete";
pub const STATUS_FAILED: &str = "failed";

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Write JSON atomically: tmp in same directory, then `rename` to final name.
fn atomic_write_json(final_path: &Path, value: &Value) -> Result<()> {
    let parent = final_path
        .parent()
        .with_context(|| format!("manifest parent missing for {}", final_path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create_dir {}", parent.display()))?;

    let tmp = parent.join(MANIFEST_TMP);
    let body = serde_json::to_string_pretty(value)?;
    fs::write(&tmp, &body).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, final_path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), final_path.display()))?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    if path.is_file() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    } else if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

/// Remove leftover staging artifacts from a crashed previous run.
pub fn cleanup_staging(out_dir: &Path) -> Result<()> {
    remove_if_exists(&out_dir.join(RISK_ZARR_STAGING))?;
    remove_if_exists(&out_dir.join(MANIFEST_STAGING))?;
    remove_if_exists(&out_dir.join(MANIFEST_TMP))?;
    Ok(())
}

/// Mark baserun output as rebuilding; previous `risk.zarr` (if any) stays until publish.
pub fn write_in_progress(out_dir: &Path, baserun: &str) -> Result<()> {
    let value = json!({
        "status": STATUS_IN_PROGRESS,
        "baserun": baserun,
        "started_at_epoch": now_epoch_secs(),
        "risk_zarr": RISK_ZARR,
        "message": "risk score computation in progress",
    });
    atomic_write_json(&out_dir.join(MANIFEST), &value)
}

pub fn write_failed(out_dir: &Path, baserun: &str, error: &str) -> Result<()> {
    let value = json!({
        "status": STATUS_FAILED,
        "baserun": baserun,
        "failed_at_epoch": now_epoch_secs(),
        "error": error,
    });
    atomic_write_json(&out_dir.join(MANIFEST), &value)
}

/// Build complete manifest JSON (written to staging before publish).
pub fn build_complete_manifest(
    baserun: &str,
    wave_model: &Option<String>,
    flow_model: &Option<String>,
    atmos_model: &Option<String>,
    wave_grid_shape: &[u64],
    source_wave_zarr: &str,
    risk_input_vars: &[&str],
    elapsed_secs: f64,
) -> Value {
    json!({
        "status": STATUS_COMPLETE,
        "baserun": baserun,
        "completed_at_epoch": now_epoch_secs(),
        "elapsed_secs": elapsed_secs,
        "risk_zarr": RISK_ZARR,
        "wave_model": wave_model,
        "flow_model": flow_model,
        "atmos_model": atmos_model,
        "wave_grid_shape": wave_grid_shape,
        "source_wave_zarr": source_wave_zarr,
        "risk_input_vars": risk_input_vars,
        "wind_units": "knot",
        "worker": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// Write manifest to `.staging`, then rename to final (atomic publish signal).
pub fn publish_manifest(out_dir: &Path, value: &Value) -> Result<PathBuf> {
    let staging = out_dir.join(MANIFEST_STAGING);
    let final_path = out_dir.join(MANIFEST);
    remove_if_exists(&staging)?;
    atomic_write_json(&staging, value)?;
    fs::rename(&staging, &final_path).with_context(|| {
        format!(
            "publish manifest {} -> {}",
            staging.display(),
            final_path.display()
        )
    })?;
    Ok(final_path)
}

/// Replace published `risk.zarr` with fully written staging directory.
pub fn publish_risk_zarr(out_dir: &Path) -> Result<PathBuf> {
    let staging = out_dir.join(RISK_ZARR_STAGING);
    let final_path = out_dir.join(RISK_ZARR);
    if !staging.is_dir() {
        anyhow::bail!("missing staging zarr {}", staging.display());
    }

    let backup = out_dir.join(format!("{RISK_ZARR}.replaced"));
    remove_if_exists(&backup)?;

    if final_path.exists() {
        fs::rename(&final_path, &backup)
            .with_context(|| format!("backup {}", final_path.display()))?;
    }

    match fs::rename(&staging, &final_path) {
        Ok(()) => {
            let _ = remove_if_exists(&backup);
            Ok(final_path)
        }
        Err(e) => {
            if backup.is_dir() && !final_path.exists() {
                let _ = fs::rename(&backup, &final_path);
            }
            Err(e).with_context(|| format!("publish zarr {}", final_path.display()))
        }
    }
}

/// True when output is safe for downstream consumers.
pub fn is_complete_output(dir: &Path) -> bool {
    if !dir.join(RISK_ZARR).is_dir() {
        return false;
    }
    let Ok(text) = fs::read_to_string(dir.join(MANIFEST)) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    v.get("status").and_then(|s| s.as_str()) == Some(STATUS_COMPLETE)
}
