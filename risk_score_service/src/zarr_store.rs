//! Filesystem Zarr helpers (paths, store open, coordinate copy).

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use zarrs::array::{Array, ArrayBuilder};
use zarrs::filesystem::FilesystemStore;

pub fn open_fs_store(path: &Path) -> Result<Arc<FilesystemStore>> {
    let store = FilesystemStore::new(path)
        .with_context(|| format!("FilesystemStore::new({})", path.display()))?;
    Ok(Arc::new(store))
}

pub fn zpath(name: &str) -> String {
    format!("/{name}")
}

/// Copy a 1D coordinate array (lat / lon / time) if present.
pub fn copy_coord_array(
    src_store: &Arc<FilesystemStore>,
    dst_store: &Arc<FilesystemStore>,
    name: &str,
) -> Result<()> {
    let path = zpath(name);
    let src = match Array::open(src_store.clone(), &path) {
        Ok(a) => a,
        Err(_) => return Ok(()),
    };
    let mut builder = ArrayBuilder::from_array(&src);
    builder.dimension_names(Some([name]));
    let dst = builder
        .build(dst_store.clone(), &path)
        .with_context(|| format!("build coord {name}"))?;
    dst.store_metadata()?;
    let subset = src.subset_all();
    if let Ok(data) = src.retrieve_array_subset_ndarray::<f64>(&subset) {
        let zeros = vec![0u64; src.shape().len()];
        dst.store_array_subset_ndarray(&zeros, &data.view())
            .with_context(|| format!("store coord subset f64 {name}"))?;
    } else if let Ok(data) = src.retrieve_array_subset_ndarray::<f32>(&subset) {
        let zeros = vec![0u64; src.shape().len()];
        dst.store_array_subset_ndarray(&zeros, &data.view())
            .with_context(|| format!("store coord subset f32 {name}"))?;
    }
    Ok(())
}
