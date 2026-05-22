//! Retention / garbage collection for risk score output directories.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::baserun::is_baserun_id;
use crate::manifest;

/// Output folder with a successfully published run (`status: complete`).
pub fn is_risk_output_dir(dir: &Path) -> bool {
    manifest::is_complete_output(dir)
}

/// List baserun output dirs under `output_root`, oldest → newest.
pub fn list_output_baseruns(output_root: &Path) -> Result<Vec<String>> {
    let mut ids = Vec::new();

    if !output_root.is_dir() {
        return Ok(ids);
    }

    for entry in fs::read_dir(output_root)
        .with_context(|| format!("read_dir {}", output_root.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
    {
        let id = match entry.file_name().into_string() {
            Ok(name) if is_baserun_id(&name) => name,
            _ => continue,
        };
        if is_risk_output_dir(&entry.path()) {
            ids.push(id);
        }
    }

    ids.sort();
    Ok(ids)
}

/// Delete output baserun dirs beyond the newest `keep` entries.
///
/// Returns ids of deleted baseruns (oldest removed first).
pub fn gc_old_outputs(output_root: &Path, keep: usize) -> Result<Vec<String>> {
    if keep == 0 {
        anyhow::bail!("retention must be at least 1");
    }

    let mut ids = list_output_baseruns(output_root)?;
    if ids.len() <= keep {
        return Ok(Vec::new());
    }

    let remove_count = ids.len() - keep;
    let to_remove: Vec<String> = ids.drain(..remove_count).collect();

    for id in &to_remove {
        let dir = output_root.join(id);
        fs::remove_dir_all(&dir)
            .with_context(|| format!("GC remove {}", dir.display()))?;
        eprintln!("GC removed output baserun {} ({})", id, dir.display());
    }

    Ok(to_remove)
}
