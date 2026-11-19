//! `filmcraft-cli probe`: media info as JSON, plus container details for MPEG TS / PS (program,
//! streams with their codecs, access units, picture types and timestamp ranges), MXF (operational
//! pattern, tracks, index, timecode), Ogg (logical streams, pre-skip, granules), Broadcast WAV
//! (time reference) and image sequences (frames, missing numbers).

use std::sync::Arc;

use serde_json::{Value, json};

/// Probe `path`; with `image_sequence`, `path` is the first frame of a numbered still sequence.
pub fn probe(path: &str, image_sequence: bool) -> Result<Value, String> {
    if image_sequence {
        use filmcraft_media::sequence::{ImageSequenceSource, Numbered, sequence_frames};
        let n = Numbered::parse(path).ok_or_else(|| format!("{path}: not a numbered still image"))?;
        let dir = if n.dir.is_empty() { ".".to_string() } else { n.dir.clone() };
        let names: Vec<String> =
            std::fs::read_dir(&dir).map_err(|e| format!("{dir}: {e}"))?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        let frames = sequence_frames(&n, &names);
        let name = std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let loader: filmcraft_media::sequence::FrameLoader = Arc::new(|p: &str| filmcraft_media::reader::read_regular_file(std::path::Path::new(p)));
        // the timebase of a fresh install (Settings ▸ Media ▸ Indeterminate Media Timebase)
        let src = ImageSequenceSource::new(&name, frames, filmcraft_time::FrameRate::FPS_29_97, loader).map_err(|e| format!("{path}: {e}"))?;
        let mut v = serde_json::to_value(filmcraft_media::MediaSource::info(&src)).unwrap_or_default();
        v["imageSequence"] = json!({
            "first": src.frame_path(0),
            "firstNumber": n.number,
            "lastNumber": n.number + src.frame_count() as u64 - 1,
            "frames": src.frame_count(),
            "missing": src.missing_frames().iter().map(|i| n.number + *i as u64).collect::<Vec<_>>(),
        });
        return Ok(v);
    }
    let bytes: Arc<[u8]> = filmcraft_media::reader::read_regular_file(std::path::Path::new(path)).map_err(|e| format!("{path}: {e}"))?.into();
    let name = std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    filmcraft_codecs::probe_bytes(&name, bytes).map_err(|e| format!("{path}: {e}"))
}
