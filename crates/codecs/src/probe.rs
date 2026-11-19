//! Container-specific media probe details shared by the CLI and browser bridge.

use std::sync::Arc;

use serde_json::{Value, json};

fn rate(r: craft_media::mxf::Rational) -> String {
    format!("{}/{}", r.num, r.den)
}

fn mxf_details(src: &crate::MxfSource) -> Value {
    let f = src.file();
    let tracks: Vec<Value> = f
        .tracks
        .iter()
        .map(|t| {
            let mut value = json!({
                "trackId": t.track_id,
                "trackNumber": format!("{:08x}", t.track_number),
                "kind": format!("{:?}", t.kind),
                "codec": t.codec.name(),
                "wrapping": format!("{:?}", t.wrapping),
                "editRate": rate(t.edit_rate),
                "origin": t.origin,
                "startPosition": t.start_position,
                "duration": t.duration,
            });
            if t.kind == craft_media::mxf::TrackKind::Picture {
                value["pictures"] = json!(t.samples.len());
                value["keyFrames"] = json!(t.samples.iter().filter(|sample| sample.key).count());
                value["indexed"] = json!(t.indexed);
                value["temporalOffsets"] = json!(t.temporal_offsets);
                value["reorderedFromBitstream"] = json!(t.needs_reorder);
            }
            if let Some(sound) = &t.sound {
                value["sampleRate"] = json!(rate(sound.sample_rate));
                value["channels"] = json!(sound.channels);
                value["bits"] = json!(sound.bits);
                value["sampleFrames"] = json!(t.stored_sample_frames());
            }
            value
        })
        .collect();
    json!({
        "operationalPattern": f.operational_pattern.name(),
        "timecode": f.timecode.map(|timecode| json!({"start": timecode.start, "roundedBase": timecode.rounded_base, "dropFrame": timecode.drop_frame, "text": timecode.format()})),
        "materialPackage": f.material_package_name,
        "product": f.product_name,
        "company": f.company_name,
        "partitions": f.partitions.len(),
        "indexSegments": f.index_segments.len(),
        "runIn": f.run_in,
        "tracks": tracks,
        "warnings": f.warnings,
    })
}

fn ogg_details(src: &crate::OggSource) -> Value {
    let f = src.file();
    let streams: Vec<Value> = f
        .streams
        .iter()
        .map(|stream| {
            let mut value = json!({
                "serial": stream.serial,
                "codec": stream.codec.name(),
                "packets": stream.packets.len(),
                "pages": stream.pages,
                "lastGranule": stream.last_granule,
                "endOfStream": stream.saw_eos,
            });
            if stream.codec == craft_media::ogg::Codec::Opus
                && let Some(header) = stream.headers.first()
                && let Some(skip) = header.get(10..12)
            {
                let pre_skip = u16::from_le_bytes([skip[0], skip[1]]);
                let timing = craft_media::ogg::OpusTiming::of(stream, u32::from(pre_skip));
                value["preSkip"] = json!(pre_skip);
                value["samples48k"] = json!(timing.total);
            }
            value
        })
        .collect();
    json!({"streams": streams, "warnings": f.warnings})
}

fn mpeg_details(src: &crate::MpegSource) -> Value {
    let Some(file) = src.file() else {
        return json!({"format": "MPEG video elementary stream"});
    };
    let (video_index, audio_index) = src.stream_indices();
    let streams: Vec<Value> = file
        .streams
        .iter()
        .enumerate()
        .map(|(index, stream)| {
            let id = match stream.id {
                craft_media::mpegts::StreamId::Pid(pid) => json!({"pid": pid}),
                craft_media::mpegts::StreamId::Ps { stream_id, sub_id } => {
                    json!({"streamId": format!("0x{stream_id:02x}"), "subStreamId": sub_id.map(|value| format!("0x{value:02x}"))})
                }
            };
            let mut value = json!({
                "id": id,
                "streamType": format!("0x{:02x}", stream.stream_type),
                "codec": stream.codec.name(),
                "kind": format!("{:?}", stream.kind()),
                "units": stream.units.len(),
                "keyUnits": stream.units.iter().filter(|unit| unit.key).count(),
                "pesPackets": stream.pes_packets,
                "ptsRange": stream.pts_range().map(|(start, end)| json!([start, end])),
                "used": Some(index) == video_index || Some(index) == audio_index,
            });
            if let Some(language) = &stream.language {
                value["language"] = json!(language);
            }
            if let Some(lpcm) = stream.lpcm {
                value["lpcm"] = json!({"sampleRate": lpcm.sample_rate, "channels": lpcm.channels, "bits": lpcm.bits});
            }
            if stream.units.iter().any(|unit| unit.picture.is_some()) {
                let picture_count =
                    |coding_type: u8| stream.units.iter().filter(|unit| unit.picture.is_some_and(|picture| picture.coding_type == coding_type)).count();
                value["pictures"] = json!({
                    "I": picture_count(1),
                    "P": picture_count(2),
                    "B": picture_count(3),
                    "fieldPairs": stream.units.iter().filter(|unit| unit.picture.is_some_and(|picture| picture.structure != 3)).count(),
                });
            }
            value
        })
        .collect();
    json!({
        "format": file.format.name(),
        "program": file.program,
        "pcrRange": file.pcr_range.map(|(start, end)| json!([start, end])),
        "streams": streams,
        "warnings": file.warnings,
    })
}

/// Probe media bytes and include container-specific details shared with `filmcraft-cli probe`.
pub fn probe_bytes(name: &str, bytes: Arc<[u8]>) -> Result<Value, String> {
    let head_len = bytes.len().min(70_000);
    let head = bytes.get(..head_len).ok_or_else(|| format!("{name}: invalid media header range"))?;
    if craft_media::mxf::sniff(head) {
        let source = crate::MxfSource::open(name, bytes).map_err(|error| format!("{name}: {error}"))?;
        let mut value = serde_json::to_value(filmcraft_media::MediaSource::info(&source)).map_err(|error| format!("serialize media info: {error}"))?;
        value["mxf"] = mxf_details(&source);
        return Ok(value);
    }
    if crate::mpeg::sniff(head) {
        let source = crate::MpegSource::open(name, bytes).map_err(|error| format!("{name}: {error}"))?;
        let mut value = serde_json::to_value(filmcraft_media::MediaSource::info(&source)).map_err(|error| format!("serialize media info: {error}"))?;
        value["mpeg"] = mpeg_details(&source);
        return Ok(value);
    }
    if craft_media::ogg::sniff(head)
        && let Ok(source) = crate::OggSource::open(name, bytes.clone())
    {
        let mut value = serde_json::to_value(filmcraft_media::MediaSource::info(&source)).map_err(|error| format!("serialize media info: {error}"))?;
        value["ogg"] = ogg_details(&source);
        return Ok(value);
    }
    let source = crate::open_bytes(name, bytes.clone()).map_err(|error| format!("{name}: {error}"))?;
    let mut value = serde_json::to_value(source.info()).map_err(|error| format!("serialize media info: {error}"))?;
    if filmcraft_media::wav::sniff(head)
        && let Ok(wave) = filmcraft_media::wav::WavSource::parse(name, bytes)
        && let Some(time_reference) = wave.time_reference()
    {
        value["bwf"] = json!({"timeReference": time_reference});
    }
    Ok(value)
}
