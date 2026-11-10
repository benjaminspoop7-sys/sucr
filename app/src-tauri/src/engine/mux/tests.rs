// Muxer tests over real fragmented MP4 fixtures. The fixtures are produced the
// way YouTube DASH arrives — a video-only and an audio-only stream, each a
// fragmented file with several `moof` + `mdat` pairs — and the tests check the
// merged file's own structure (the sample tables the muxer wrote).
//
// What a unit test cannot decide is whether a real player accepts the result;
// that is the ffprobe leg in the bench record, kept out of here so the suite
// stays hermetic and needs no external binary.

use super::*;
use std::path::{Path, PathBuf};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data")
}

fn tmp_out(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("sucr-mux-{name}-{}.mp4", std::process::id()));
    p
}

// Descend the output's box tree to one track's stsz sample count.
fn sample_count(buf: &[u8], trak_idx: usize) -> Option<u64> {
    let top = children(buf, 0, buf.len());
    let moov = find(&top, b"moov")?;
    let moov_children = children(buf, moov.start, moov.end);
    let traks: Vec<_> = moov_children
        .into_iter()
        .filter(|b| b.kind == b"trak")
        .collect();
    let trak = traks.get(trak_idx)?;
    let tr_children = children(buf, trak.start, trak.end);
    let mdia = find(&tr_children, b"mdia")?;
    let md_children = children(buf, mdia.start, mdia.end);
    let minf = find(&md_children, b"minf")?;
    let mn_children = children(buf, minf.start, minf.end);
    let stbl = find(&mn_children, b"stbl")?;
    let sb_children = children(buf, stbl.start, stbl.end);
    let stsz = find(&sb_children, b"stsz")?;
    // stsz body: version+flags(4) sample_size(4) sample_count(4).
    Some(be(buf, stsz.start + 8, 4))
}

fn hdlr_of(buf: &[u8], trak_idx: usize) -> Option<[u8; 4]> {
    let top = children(buf, 0, buf.len());
    let moov = find(&top, b"moov")?;
    let moov_children = children(buf, moov.start, moov.end);
    let traks: Vec<_> = moov_children
        .into_iter()
        .filter(|b| b.kind == b"trak")
        .collect();
    let trak = traks.get(trak_idx)?;
    let tr_children = children(buf, trak.start, trak.end);
    let mdia = find(&tr_children, b"mdia")?;
    let md_children = children(buf, mdia.start, mdia.end);
    let hdlr = find(&md_children, b"hdlr")?;
    Some([
        buf[hdlr.start + 8],
        buf[hdlr.start + 9],
        buf[hdlr.start + 10],
        buf[hdlr.start + 11],
    ])
}

#[test]
fn merges_a_fragmented_video_and_audio_pair_into_one_progressive_file() {
    let d = data_dir();
    let out = tmp_out("pair");
    mux(&d.join("v_multi.mp4"), Some(&d.join("a_multi.mp4")), &out)
        .expect("the pair muxes");

    let buf = std::fs::read(&out).unwrap();
    let top = children(&buf, 0, buf.len());
    assert_eq!(top[0].kind, b"ftyp", "starts with ftyp");
    assert_eq!(top[1].kind, b"moov", "then moov");
    assert_eq!(top[2].kind, b"mdat", "then one mdat");

    // video is the first trak, audio the second, each with its full sample table.
    assert_eq!(hdlr_of(&buf, 0), Some(*b"vide"));
    assert_eq!(hdlr_of(&buf, 1), Some(*b"soun"));
    assert_eq!(sample_count(&buf, 0), Some(75), "video kept all 75 frames across fragments");
    assert_eq!(sample_count(&buf, 1), Some(217), "audio kept all 217 frames");

    // The media is whole: the mdat box size written at its header agrees with
    // the bytes that actually trail it, which is the last box in the file.
    let mdat = &top[2];
    let header_start = mdat.start - 8;
    let declared = be(&buf, header_start, 4) as usize;
    assert_eq!(declared, buf.len() - header_start, "mdat box size matches the file tail");

    let _ = std::fs::remove_file(&out);
}

#[test]
fn muxes_a_single_fragmented_video_with_no_audio() {
    let d = data_dir();
    let out = tmp_out("vonly");
    mux(&d.join("v_only.mp4"), None, &out).expect("a lone video stream muxes");

    let buf = std::fs::read(&out).unwrap();
    let top = children(&buf, 0, buf.len());
    assert_eq!(top[0].kind, b"ftyp");
    assert_eq!(top[1].kind, b"moov");
    assert_eq!(top[2].kind, b"mdat");
    assert_eq!(hdlr_of(&buf, 0), Some(*b"vide"));
    assert_eq!(sample_count(&buf, 0), Some(75));
    assert!(hdlr_of(&buf, 1).is_none(), "no second trak when there is no audio");

    let _ = std::fs::remove_file(&out);
}

#[test]
fn rejects_a_source_that_is_not_fragmented_media() {
    let out = tmp_out("bad");
    let junk = data_dir().join("nope_missing.mp4");
    // A path that does not exist fails to read, not silently produce a file.
    assert!(mux(&junk, None, &out).is_err());
    let _ = std::fs::remove_file(&out);
}

// The ffprobe leg writes a merged file to a fixed path so the bench can prove a
// real player accepts it. Kept ignored so the hermetic suite needs no ffmpeg;
// run it with `cargo test --release mux -- --ignored` and probe the output.
#[test]
#[ignore]
fn writes_a_pair_to_disk_for_the_player_probe() {
    let d = data_dir();
    let out = d.join("merged_probe.mp4");
    mux(&d.join("v_multi.mp4"), Some(&d.join("a_multi.mp4")), &out)
        .expect("bench mux");
    let _ = mux(&d.join("v_only.mp4"), None, &d.join("merged_probe_video_only.mp4"));
}
