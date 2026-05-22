//! End-to-end run: wave.zarr in → risk.zarr out for one baserun.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use zarrs::array::{Array, ArrayBuilder, ArraySubset};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::GroupBuilder;

use crate::baserun::{
    discover_wave_grid_shape, read_baserun_metadata, read_hs_fill_f32, wave_zarr_path,
    BaserunMetadata, RISK_INPUT_COORDS, RISK_INPUT_VARS,
};
use crate::manifest::{
    build_complete_manifest, cleanup_staging, publish_manifest, publish_risk_zarr, write_failed,
    write_in_progress, RISK_ZARR_STAGING,
};
use crate::risk::{compute_chunk_risks, SHIP_MODELS};
use crate::zarr_store::{copy_coord_array, open_fs_store, zpath};

fn log_progress(baserun: &str, pct: u64, done: u64, total: u64) {
    crate::log!(
        "[risk_score] PROGRESS baserun={baserun} {pct}% ({done}/{total} chunks)"
    );
}

fn log_done(
    baserun: &str,
    risk_path: &Path,
    manifest_path: &Path,
    wave_model: &Option<String>,
    elapsed_secs: f64,
) {
    crate::log!("========================================");
    crate::log!("[risk_score] DONE — risk.zarr selesai");
    crate::log!("baserun     : {baserun}");
    crate::log!("risk.zarr   : {}", risk_path.display());
    crate::log!(
        "manifest    : {} (status=complete)",
        manifest_path.display()
    );
    crate::log!("wave_model  : {}", wave_model.as_deref().unwrap_or("?"));
    crate::log!("elapsed     : {elapsed_secs:.1}s");
    crate::log!("========================================");
}

fn on_run_failed(out_dir: &Path, baserun: &str, err: &anyhow::Error) {
    crate::log!("[risk_score] FAILED baserun={baserun}: {err:#}");
    if let Err(clean_err) = cleanup_staging(out_dir) {
        crate::log!(
            "[risk_score] WARN cleanup staging after failure: {clean_err:#}"
        );
    }
    if let Err(m_err) = write_failed(out_dir, baserun, &err.to_string()) {
        crate::log!(
            "[risk_score] WARN could not write failed manifest: {m_err:#}"
        );
    }
}

pub fn process_baserun(input_root: &Path, output_root: &Path, baserun: &str) -> Result<()> {
    let started = Instant::now();
    let baserun_dir = input_root.join(baserun);
    let wave_root = wave_zarr_path(&baserun_dir);
    let out_dir = output_root.join(baserun);

    fs::create_dir_all(&out_dir)
        .with_context(|| format!("create_dir {}", out_dir.display()))?;
    cleanup_staging(&out_dir)?;
    write_in_progress(&out_dir, baserun)?;

    let run_result = process_baserun_inner(
        &started,
        &baserun_dir,
        &wave_root,
        &out_dir,
        baserun,
    );

    if let Err(ref e) = run_result {
        on_run_failed(&out_dir, baserun, e);
    }

    run_result
}

fn process_baserun_inner(
    started: &Instant,
    baserun_dir: &Path,
    wave_root: &Path,
    out_dir: &Path,
    baserun: &str,
) -> Result<()> {
    let store_in = open_fs_store(wave_root)?;

    let hs = Array::open(store_in.clone(), &zpath("hs")).context("open /hs")?;
    let uw = Array::open(store_in.clone(), &zpath("uwnd")).context("open /uwnd")?;
    let vw = Array::open(store_in.clone(), &zpath("vwnd")).context("open /vwnd")?;

    for var in RISK_INPUT_VARS {
        if !wave_root.join(var).join(".zarray").is_file() {
            bail!("missing risk input variable {var} under {}", wave_root.display());
        }
    }

    if hs.shape() != uw.shape() || hs.shape() != vw.shape() {
        bail!(
            "shape mismatch hs {:?} uw {:?} vw {:?}",
            hs.shape(),
            uw.shape(),
            vw.shape()
        );
    }

    let hs_fill = read_hs_fill_f32(wave_root);
    let staging_path = out_dir.join(RISK_ZARR_STAGING);
    if staging_path.exists() {
        fs::remove_dir_all(&staging_path)
            .with_context(|| format!("remove existing {}", staging_path.display()))?;
    }
    let store_out = open_fs_store(&staging_path)?;

    GroupBuilder::new()
        .build(store_out.clone(), "/")
        .context("create root group")?
        .store_metadata()?;

    let mut risk_arrays: HashMap<String, Array<FilesystemStore>> = HashMap::new();
    for (ship, _) in SHIP_MODELS {
        let label = format!("risk_{ship}");
        let path = zpath(&label);
        let mut builder = ArrayBuilder::from_array(&hs);
        builder
            .data_type(zarrs::array::data_type::uint8())
            .fill_value(0u8)
            .dimension_names(Some(["time", "lat", "lon"]))
            .attributes(
                serde_json::json!({
                    "long_name": format!("Risk level for {ship}"),
                    "scores": "0=Nodata, 1=Sangat Rendah, 2=Rendah, 3=Sedang, 4=Tinggi",
                    "source": "InaCAWO / Inawaves Risk Model (rust worker)",
                    "_ARRAY_DIMENSIONS": ["time", "lat", "lon"],
                })
                .as_object()
                .unwrap()
                .clone(),
            );
        let arr = builder
            .build(store_out.clone(), &path)
            .with_context(|| format!("build output array {}", label))?;
        arr.store_metadata()?;
        risk_arrays.insert(label, arr);
    }

    for c in RISK_INPUT_COORDS {
        copy_coord_array(&store_in, &store_out, c)?;
    }

    let chunk_grid_shape = hs.chunk_grid_shape().to_vec();
    let chunk_indices_it = ArraySubset::new_with_shape(chunk_grid_shape.clone()).indices();
    let total_chunks: u64 = chunk_grid_shape.iter().product();
    let grid_shape: Vec<u64> = hs.shape().to_vec();

    crate::log!(
        "[risk_score] START baserun={baserun} grid={grid_shape:?} chunks={total_chunks} staging={}",
        staging_path.display()
    );

    let mut done_chunks = 0u64;
    let mut next_milestone = 25u64;

    for idx in chunk_indices_it {
        let idx_sl: Vec<u64> = idx.as_ref().to_vec();
        let hs_chunk = hs
            .retrieve_chunk_ndarray::<f32>(&idx_sl)
            .with_context(|| format!("read hs chunk {:?}", idx_sl))?;
        let uw_chunk = uw
            .retrieve_chunk_ndarray::<f32>(&idx_sl)
            .with_context(|| format!("read uwnd chunk {:?}", idx_sl))?;
        let vw_chunk = vw
            .retrieve_chunk_ndarray::<f32>(&idx_sl)
            .with_context(|| format!("read vwnd chunk {:?}", idx_sl))?;

        let mut computed: HashMap<String, ndarray::ArrayD<u8>> = HashMap::new();
        compute_chunk_risks(
            &hs_chunk,
            &uw_chunk,
            &vw_chunk,
            hs_fill,
            &mut computed,
        );

        for (name, data) in computed {
            let out_arr = risk_arrays
                .get(&name)
                .with_context(|| format!("missing built array {name}"))?;
            out_arr.store_chunk_ndarray(&idx_sl, &data)?;
        }

        done_chunks += 1;
        if total_chunks > 0 {
            let pct = done_chunks * 100 / total_chunks;
            while pct >= next_milestone && next_milestone <= 100 {
                log_progress(baserun, next_milestone, done_chunks, total_chunks);
                next_milestone += 25;
            }
        }
    }

    let meta = read_baserun_metadata(baserun_dir).unwrap_or(BaserunMetadata {
        baserun: Some(baserun.to_string()),
        wave_model: None,
        flow_model: None,
        atmos_model: None,
        wave_size_bytes: None,
        flow_size_bytes: None,
        atmos_size_bytes: None,
    });
    let grid = discover_wave_grid_shape(wave_root)?;
    let elapsed = started.elapsed().as_secs_f64();

    let manifest_value = build_complete_manifest(
        baserun,
        &meta.wave_model,
        &meta.flow_model,
        &meta.atmos_model,
        &grid.shape,
        &wave_root.display().to_string(),
        &RISK_INPUT_VARS,
        elapsed,
    );

    crate::log!("[risk_score] PUBLISH baserun={baserun} swapping staging -> risk.zarr");
    let risk_path = publish_risk_zarr(out_dir)?;
    let manifest_path = publish_manifest(out_dir, &manifest_value)?;

    log_done(
        baserun,
        &risk_path,
        &manifest_path,
        &meta.wave_model,
        elapsed,
    );
    Ok(())
}
