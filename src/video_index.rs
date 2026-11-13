//! Video indexing (milestone M2).

use crate::indexer::Progress;
use crate::library::Library;
use crate::model::VisionEncoder;
use crate::store::db::FileRow;
use anyhow::Result;

pub(crate) fn index_videos(
    _lib: &Library,
    _enc: &mut VisionEncoder,
    _files: &[FileRow],
    _prog: &mut Progress,
) -> Result<()> {
    Ok(())
}
