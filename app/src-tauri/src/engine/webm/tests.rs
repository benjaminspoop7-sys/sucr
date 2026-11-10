// WebM muxer tests over real single-track fixtures. The pair is authored the
// way a DASH pull arrives: one VP9 picture file, one Opus sound file. What a
// unit test cannot decide is whether a player accepts the result; that is the
// ffprobe leg in the bench record, kept out of here so the suite is hermetic.

use super::*;
use std::path::{Path, PathBuf};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data")
}

fn tmp_out(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("sucr-webm-{name}-{}.webm", std::process::id()));
    p
}

// Every block's track number inside one cluster body.
fn block_tracks(buf: &[u8], body: usize, end: usize, out: &mut Vec<u8>) {
    let mut off = body;
    while off + 2 <= end {
        let Some((id, id_len, _)) = vint(buf, off, true) else { break };
        let Some((size, sz_len, unknown)) = vint(buf, off + id_len, false) else { break };
        if unknown {
            break;
        }
        let Ok(size) = usize::try_from(size) else { break };
        let b = off + id_len + sz_len;
        if b + size > end {
            break;
        }
        if id == ID_SIMPLE_BLOCK {
            if let Some((track, _, _)) = vint(buf, b, false) {
                out.push(track as u8);
            }
        } else if id == ID_BLOCK_GROUP {
            let mut j = b;
            while j + 2 <= b + size {
                let Some((gid, glen, _)) = vint(buf, j, true) else { break };
                let Some((gsize, gsz, _)) = vint(buf, j + glen, false) else { break };
                let Ok(gsize) = usize::try_from(gsize) else { break };
                let gb = j + glen + gsz;
                if gb + gsize > b + size {
                    break;
                }
                if gid == ID_BLOCK {
                    if let Some((track, _, _)) = vint(buf, gb, false) {
                        out.push(track as u8);
                    }
                }
                j = gb + gsize;
            }
        }
        off = b + size;
    }
}

#[test]
fn merges_a_vp9_picture_and_an_opus_sound_into_two_tracks() {
    let d = data_dir();
    let out = tmp_out("pair");
    mux(&d.join("v_only.webm"), &d.join("a_only.webm"), &out).expect("the pair muxes");

    let buf = std::fs::read(&out).unwrap();
    let top = elem_at(&mut Src::open(&out).unwrap(), 0).unwrap().unwrap();
    assert_eq!(top.id, ID_EBML, "starts with an EBML header");

    let seg_off = top.end() as usize;
    let mut src = Src::open(&out).unwrap();
    let seg = elem_at(&mut src, top.end()).unwrap().unwrap();
    assert_eq!(seg.id, ID_SEGMENT, "then one Segment");
    let seg_body = seg.body() as usize;
    let seg_end = seg.end() as usize;
    assert_eq!(seg_end, buf.len(), "the Segment's size matches the file");

    // Children in order: Info, Tracks, then clusters only.
    let mut off = seg_body;
    let mut kinds = Vec::new();
    while off + 2 <= seg_end {
        let (id, id_len, _) = vint(&buf, off, true).unwrap();
        let (size, sz_len, unknown) = vint(&buf, off + id_len, false).unwrap();
        assert!(!unknown, "no child of the merged segment has an unknown size");
        let total = id_len + sz_len + usize::try_from(size).unwrap();
        kinds.push((id, off, off + id_len + sz_len, off + total));
        off += total;
    }
    assert_eq!(kinds[0].0, ID_INFO);
    assert_eq!(kinds[1].0, ID_TRACKS);
    assert!(kinds[2..].iter().all(|k| k.0 == ID_CLUSTER));

    // Two entries, numbered 1 (picture) and 2 (sound).
    let (_, _, tracks_body, tracks_end) = kinds[1];
    let mut numbers = Vec::new();
    let mut j = tracks_body;
    while j + 2 <= tracks_end {
        let (id, id_len, _) = vint(&buf, j, true).unwrap();
        let (size, sz_len, _) = vint(&buf, j + id_len, false).unwrap();
        let size = usize::try_from(size).unwrap();
        let body = j + id_len + sz_len;
        assert_eq!(id, ID_TRACK_ENTRY);
        let entry = &buf[body..body + size];
        let (_, nb, ns) = sub_elem(entry, ID_TRACK_NUMBER).expect("entry has a TrackNumber");
        let mut n = 0u64;
        for k in 0..ns {
            n = (n << 8) | entry[nb + k] as u64;
        }
        numbers.push(n);
        j = body + size;
    }
    assert_eq!(numbers, vec![1, 2]);

    // Every block belongs to one of the two tracks, and both are present.
    let mut tracks_seen = Vec::new();
    for k in &kinds[2..] {
        block_tracks(&buf, k.2, k.3, &mut tracks_seen);
    }
    assert!(tracks_seen.contains(&1), "picture blocks are still on track 1");
    assert!(tracks_seen.contains(&2), "sound blocks moved to track 2");
    assert!(
        tracks_seen.iter().all(|t| *t == 1 || *t == 2),
        "no block names a third track"
    );
    let _ = seg_off;

    let _ = std::fs::remove_file(&out);
}

#[test]
fn keeps_every_cluster_of_both_sources() {
    let d = data_dir();
    let out = tmp_out("count");
    mux(&d.join("v_only.webm"), &d.join("a_only.webm"), &out).expect("the pair muxes");
    let mut v = Src::open(&d.join("v_only.webm")).unwrap();
    let vs = open_segment(&mut v).unwrap();
    let v_clusters = clusters(&mut v, &vs).unwrap().len();
    let mut a = Src::open(&d.join("a_only.webm")).unwrap();
    let asg = open_segment(&mut a).unwrap();
    let a_clusters = clusters(&mut a, &asg).unwrap().len();

    let buf = std::fs::read(&out).unwrap();
    let ids: Vec<u64> = {
        let mut src = Src::open(&out).unwrap();
        let top = elem_at(&mut src, 0).unwrap().unwrap();
        let seg = elem_at(&mut src, top.end()).unwrap().unwrap();
        let mut off = seg.body() as usize;
        let mut ids = Vec::new();
        while off + 2 <= seg.end() as usize {
            let (id, id_len, _) = vint(&buf, off, true).unwrap();
            let (size, sz_len, unknown) = vint(&buf, off + id_len, false).unwrap();
            assert!(!unknown);
            ids.push(id);
            off += id_len + sz_len + usize::try_from(size).unwrap();
        }
        ids
    };
    let merged = ids.iter().filter(|id| **id == ID_CLUSTER).count();
    assert_eq!(merged, v_clusters + a_clusters, "no cluster was dropped or duplicated");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn rejects_a_source_that_is_not_webm() {
    let out = tmp_out("junk");
    let junk = data_dir().join("v_multi.mp4");
    assert!(mux(&junk, &junk, &out).is_err(), "an MP4 is not an EBML file");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn refuses_two_sources_with_different_timestamp_scales() {
    // The fixtures share a scale, so this leg only proves the guard is wired to
    // a real field: a deliberately wrong scale is injected by rewriting Info.
    let d = data_dir();
    let alt = tmp_out("scale");
    let mut bytes = {
        let mut src = Src::open(&d.join("a_only.webm")).unwrap();
        let n = usize::try_from(src.size).unwrap();
        src.read(0, n).unwrap()
    };
    let (scale_at, scale_end) = {
        let mut src = Src::open(&d.join("a_only.webm")).unwrap();
        let top = elem_at(&mut src, 0).unwrap().unwrap();
        let seg = elem_at(&mut src, top.end()).unwrap().unwrap();
        let info = find_child(&mut src, seg.body(), seg.end(), ID_INFO)
            .unwrap()
            .expect("fixture has an Info");
        let ts = find_child(&mut src, info.body(), info.end(), ID_TIME_SCALE)
            .unwrap()
            .expect("fixture has a TimestampScale");
        (ts.body() as usize, ts.end() as usize)
    };
    for b in &mut bytes[scale_at..scale_end] {
        *b = 0;
    }
    // 1 ms instead of 1 us, encoded in the same width.
    bytes[scale_end - 1] = 1;
    std::fs::write(&alt, &bytes).unwrap();
    let out = tmp_out("scale-out");
    let err = mux(&d.join("v_only.webm"), &alt, &out).unwrap_err();
    assert!(err.0.contains("TimestampScale"), "got: {}", err.0);
    let _ = std::fs::remove_file(&alt);
    let _ = std::fs::remove_file(&out);
}

// The ffprobe leg: writes a merged pair to a fixed path so the bench can prove
// a real demuxer accepts it. Ignored so the hermetic suite needs no ffmpeg.
// `SUCR_WEBM_BENCH=<video>;<audio>;<out>` points it at real downloads instead
// of the fixtures, which is how the muxer is checked at full length.
#[test]
#[ignore]
fn writes_a_pair_to_disk_for_the_player_probe() {
    let bench = std::env::var("SUCR_WEBM_BENCH").ok();
    let (v, a, out) = match &bench {
        Some(spec) => {
            let parts: Vec<&str> = spec.split(';').collect();
            assert_eq!(parts.len(), 3, "SUCR_WEBM_BENCH is <video>;<audio>;<out>");
            (
                PathBuf::from(parts[0]),
                PathBuf::from(parts[1]),
                PathBuf::from(parts[2]),
            )
        }
        None => (
            data_dir().join("v_only.webm"),
            data_dir().join("a_only.webm"),
            data_dir().join("merged_probe.webm"),
        ),
    };
    mux(&v, &a, &out).expect("bench mux");
}
