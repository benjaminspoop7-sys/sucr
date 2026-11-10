// fMP4 -> MP4 remuxer. A DASH-style download lands as two fragmented files: a
// "video only" stream and a matching "audio only" stream, each a `moov` with an
// empty sample table plus repeated `moof` + `mdat` pairs. Players and most tools
// want one progressive file, so this reads both fragments, collects every sample
// across every `moof`, and writes a single non-fragmented `ftyp + moov + mdat`.
//
// It is deliberately pure byte work on the ISO BMFF box grammar: no new crate and
// no ffmpeg in the product path. The sizes, sample tables and composition offsets
// are rebuilt from the source fragment headers, which is what makes the merged
// file seekable and frame-accurate the way a plain MP4 has to be.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug)]
pub struct MuxError(pub String);

impl MuxError {
    fn msg(m: impl Into<String>) -> Self {
        MuxError(m.into())
    }
}

// One big-endian read helper does all of them: MP4 is network byte order and the
// box grammar only ever needs fixed-width integers.
fn be(buf: &[u8], off: usize, len: usize) -> u64 {
    let mut v: u64 = 0;
    for k in 0..len {
        v = (v << 8) | buf[off + k] as u64;
    }
    v
}

// A top-level box: fourcc and the half-open byte range that holds its contents,
// already past the (possibly 64-bit) header.
struct Box<'a> {
    kind: &'a [u8],
    start: usize,
    end: usize,
}

fn children<'a>(buf: &'a [u8], start: usize, end: usize) -> Vec<Box<'a>> {
    let mut out = Vec::new();
    let mut i = start;
    while i + 8 <= end {
        let mut size = be(buf, i, 4) as usize;
        let kind = &buf[i + 4..i + 8];
        let mut body = i + 8;
        if size == 1 {
            if i + 16 > end {
                break;
            }
            size = be(buf, i + 8, 8) as usize;
            body = i + 16;
        } else if size == 0 {
            size = end - i;
        }
        if size < 8 || i + size > end {
            break;
        }
        out.push(Box {
            kind,
            start: body,
            end: i + size,
        });
        i += size;
    }
    out
}

fn find<'a>(boxes: &'a [Box<'a>], fourcc: &[u8]) -> Option<&'a Box<'a>> {
    boxes.iter().find(|b| b.kind == fourcc)
}

// One decode-order sample and, while a fragment is being read, the absolute file
// offset its bytes start at.
#[derive(Clone)]
struct Samp {
    dur: u32,
    size: u32,
    cto: i64,
    sync: bool,
    off: u64,
}

// Everything needed to emit one progressive track, lifted out of one fragment file.
struct Track {
    hdlr: [u8; 4],
    timescale: u32,
    width: u32,
    height: u32,
    // The codec entry from the source stsd, without the stsd header/count.
    entries: Vec<u8>,
    samples: Vec<Samp>,
}

impl Track {
    fn media_duration(&self) -> u64 {
        self.samples.iter().map(|s| s.dur as u64).sum()
    }
    fn total_bytes(&self) -> u64 {
        self.samples.iter().map(|s| s.size as u64).sum()
    }
}

// Parse one fragmented MP4 into a single Track. The moov carries the handler,
// timescale, geometry and the codec entry (stsd); the samples live in the moofs.
// Real DASH keeps the track in one `trak`, so the first is taken.
fn read_track(buf: &[u8]) -> Result<Track, MuxError> {
    let top = children(buf, 0, buf.len());
    let moov = find(&top, b"moov").ok_or_else(|| MuxError::msg("no moov in a source fragment"))?;
    let moov_children = children(buf, moov.start, moov.end);
    let trak = find(&moov_children, b"trak").ok_or_else(|| MuxError::msg("no trak in the source"))?;
    let tr_children = children(buf, trak.start, trak.end);
    let mdia = find(&tr_children, b"mdia").ok_or_else(|| MuxError::msg("no mdia"))?;
    let md_children = children(buf, mdia.start, mdia.end);

    let mdhd = find(&md_children, b"mdhd").ok_or_else(|| MuxError::msg("no mdhd"))?;
    let timescale = read_mdhd_ts(buf, mdhd)?;

    let hdlr = find(&md_children, b"hdlr").ok_or_else(|| MuxError::msg("no hdlr"))?;
    // handler_type is the 4 bytes at body offset 8 of hdlr (after version+flags
    // and the pre-defined field).
    let htype = [
        buf[hdlr.start + 8],
        buf[hdlr.start + 9],
        buf[hdlr.start + 10],
        buf[hdlr.start + 11],
    ];

    let minf = find(&md_children, b"minf").ok_or_else(|| MuxError::msg("no minf"))?;
    let mn_children = children(buf, minf.start, minf.end);
    let stbl = find(&mn_children, b"stbl").ok_or_else(|| MuxError::msg("no stbl"))?;
    let sb_children = children(buf, stbl.start, stbl.end);
    let stsd = find(&sb_children, b"stsd").ok_or_else(|| MuxError::msg("no stsd"))?;
    // stsd body = version+flags(4) + entry_count(4) + entries.
    let entries = buf[stsd.start + 8..stsd.end].to_vec();

    let (width, height) = read_tkhd(buf, find(&tr_children, b"tkhd"));

    let mut samples: Vec<Samp> = Vec::new();
    for moof in top.iter().filter(|b| b.kind == b"moof") {
        read_fragment(buf, moof, &mut samples)?;
    }
    if samples.is_empty() {
        return Err(MuxError::msg("the source carries no samples"));
    }

    Ok(Track {
        hdlr: htype,
        timescale,
        width,
        height,
        entries,
        samples,
    })
}

fn read_mdhd_ts(buf: &[u8], box_: &Box) -> Result<u32, MuxError> {
    let ver = buf[box_.start];
    let off = if ver == 0 { box_.start + 12 } else { box_.start + 20 };
    let ts = be(buf, off, 4) as u32;
    if ts == 0 {
        return Err(MuxError::msg("zero timescale in mdhd"));
    }
    Ok(ts)
}

fn read_tkhd(buf: &[u8], box_: Option<&Box>) -> (u32, u32) {
    let t = match box_ {
        Some(t) => t,
        None => return (0, 0),
    };
    // width and height are the last two 16.16 fields regardless of version.
    if t.end - t.start < 8 {
        return (0, 0);
    }
    let off = t.end - 8;
    let w = (be(buf, off, 4) >> 16) as u32;
    let h = (be(buf, off + 4, 4) >> 16) as u32;
    (w, h)
}

// Walk one moof: its track fragment records where each sample's bytes live, and
// the media data box holds them. Decode order is preserved by walking truns in
// file order, the same order the fragments were written.
fn read_fragment(buf: &[u8], moof: &Box, out: &mut Vec<Samp>) -> Result<(), MuxError> {
    let mc = children(buf, moof.start, moof.end);
    // The mdat follows this moof; find it by scanning from the moof's end.
    let after = children(buf, moof.end, buf.len());
    let mdat_payload = find(&after, b"mdat")
        .ok_or_else(|| MuxError::msg("moof with no following mdat"))?
        .start;

    for traf in mc.iter().filter(|b| b.kind == b"traf") {
        let tc = children(buf, traf.start, traf.end);
        let tfhd = find(&tc, b"tfhd").ok_or_else(|| MuxError::msg("traf with no tfhd"))?;
        let tfhd_flags = be(buf, tfhd.start + 1, 3) as u32;
        let mut p = tfhd.start + 4 + 4; // version+flags, then track_ID
        if tfhd_flags & 0x01 != 0 {
            p += 8; // base_data_offset
        }
        if tfhd_flags & 0x02 != 0 {
            p += 4; // sample_description_index
        }
        let mut def_dur = 0u32;
        let mut def_size = 0u32;
        let mut def_flags = 0u32;
        if tfhd_flags & 0x08 != 0 {
            def_dur = be(buf, p, 4) as u32;
            p += 4;
        }
        if tfhd_flags & 0x10 != 0 {
            def_size = be(buf, p, 4) as u32;
            p += 4;
        }
        if tfhd_flags & 0x20 != 0 {
            def_flags = be(buf, p, 4) as u32;
        }

        for trun in tc.iter().filter(|b| b.kind == b"trun") {
            let tr_ver = buf[trun.start];
            let tr_flags = be(buf, trun.start + 1, 3) as u32;
            let count = be(buf, trun.start + 4, 4) as usize;
            let mut q = trun.start + 8;
            let mut data_off: i64 = 0;
            if tr_flags & 0x01 != 0 {
                data_off = (be(buf, q, 4) as i32) as i64;
                q += 4;
            }
            let mut first_flags = def_flags;
            if tr_flags & 0x04 != 0 {
                first_flags = be(buf, q, 4) as u32;
                q += 4;
            }
            // data_offset is relative to the enclosing moof box start; the box
            // start is 8 before the body we hold. A zero means "the run begins at
            // the mdat payload", the layout some encoders emit.
            let base = if data_off == 0 {
                mdat_payload as u64
            } else {
                (moof.start - 8 + data_off as usize) as u64
            };
            let mut cur = base;
            for i in 0..count {
                let mut dur = def_dur;
                let mut size = def_size;
                let mut flags = def_flags;
                let mut cto = 0i64;
                if tr_flags & 0x100 != 0 {
                    dur = be(buf, q, 4) as u32;
                    q += 4;
                }
                if tr_flags & 0x200 != 0 {
                    size = be(buf, q, 4) as u32;
                    q += 4;
                }
                if tr_flags & 0x400 != 0 {
                    flags = be(buf, q, 4) as u32;
                    q += 4;
                } else if i == 0 {
                    flags = first_flags;
                }
                if tr_flags & 0x800 != 0 {
                    cto = if tr_ver == 1 {
                        (be(buf, q, 4) as i32) as i64
                    } else {
                        be(buf, q, 4) as i64
                    };
                    q += 4;
                }
                let depends_on = (flags >> 24) & 0x07;
                out.push(Samp {
                    dur,
                    size,
                    cto,
                    sync: depends_on == 0,
                    off: cur,
                });
                cur += size as u64;
            }
        }
    }
    Ok(())
}

// Box writing helpers -----------------------------------------------------

struct W {
    buf: Vec<u8>,
}

impl W {
    fn new() -> Self {
        W { buf: Vec::new() }
    }
    fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    // Wrap a finished body in a size+fourcc header and append it.
    fn close(&mut self, fourcc: &[u8; 4], body: &[u8]) {
        let size = (body.len() + 8) as u32;
        self.u32(size);
        self.raw(fourcc);
        self.raw(body);
    }
}

fn write_matrix(w: &mut W) {
    for v in [
        0x0001_0000u32,
        0,
        0,
        0,
        0x0001_0000,
        0,
        0,
        0,
        0x4000_0000,
    ] {
        w.u32(v);
    }
}

fn box_mvhd(movie_ts: u32, movie_dur: u64, next_id: u32) -> Vec<u8> {
    let mut w = W::new();
    w.u32(0); // version + flags
    w.u32(0); // creation
    w.u32(0); // modification
    w.u32(movie_ts);
    w.u32(movie_dur as u32);
    w.u32(0x0001_0000); // rate
    w.u16(0x0100); // volume
    w.u16(0); // reserved
    w.u32(0);
    w.u32(0); // reserved (8)
    write_matrix(&mut w);
    for _ in 0..6 {
        w.u32(0);
    } // predefined (24)
    w.u32(next_id);
    w.buf
}

fn box_tkhd(id: u32, dur_movie: u64, w: u32, h: u32, audio: bool) -> Vec<u8> {
    let mut x = W::new();
    x.u32(7); // version 0, flags enabled|in_movie
    x.u32(0); // creation
    x.u32(0); // modification
    x.u32(id);
    x.u32(0); // reserved
    x.u32(dur_movie as u32);
    x.u32(0);
    x.u32(0); // reserved (8)
    x.u16(0); // layer
    x.u16(0); // alternate_group
    x.u16(if audio { 0x0100 } else { 0 }); // volume
    x.u16(0); // reserved
    write_matrix(&mut x);
    x.u32(w << 16);
    x.u32(h << 16);
    x.buf
}

fn box_mdhd(ts: u32, dur: u64) -> Vec<u8> {
    let mut x = W::new();
    x.u32(0); // version + flags
    x.u32(0);
    x.u32(0);
    x.u32(ts);
    x.u32(dur as u32);
    x.u16(0x55C4); // language 'und'
    x.u16(0); // pre_defined
    x.buf
}

fn box_hdlr(htype: [u8; 4]) -> Vec<u8> {
    let mut x = W::new();
    x.u32(0); // version + flags
    x.u32(0); // pre_defined
    x.raw(&htype);
    for _ in 0..3 {
        x.u32(0);
    } // reserved (12)
    x.raw(b"\0"); // empty name
    x.buf
}

fn box_vmhd() -> Vec<u8> {
    let mut x = W::new();
    x.u32(1); // version 0, flags 1
    x.u16(0); // graphicsmode
    for _ in 0..3 {
        x.u16(0);
    } // opcolor
    x.buf
}

fn box_smhd() -> Vec<u8> {
    let mut x = W::new();
    x.u32(0); // version + flags
    x.u16(0); // sync_offset
    x.u16(0); // reserved
    x.buf
}

fn box_dinf() -> Vec<u8> {
    // A self-contained dref: dinf -> dref (count 1) -> url (flags 1 = self).
    let mut url = W::new();
    url.u32(1); // version 0, flags 1 (self-contained)
    let mut url_box = W::new();
    url_box.close(b"url ", &url.buf);

    let mut dref = W::new();
    dref.u32(0); // version + flags
    dref.u32(1); // entry_count
    dref.raw(&url_box.buf);
    let mut dref_box = W::new();
    dref_box.close(b"dref", &dref.buf);

    let mut dinf = W::new();
    dinf.close(b"dinf", &dref_box.buf);
    dinf.buf
}

fn box_stsd(entries: &[u8]) -> Vec<u8> {
    let mut x = W::new();
    x.u32(0); // version + flags
    x.u32(1); // one entry
    x.raw(entries);
    x.buf
}

fn box_stts(samples: &[Samp]) -> Vec<u8> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for s in samples {
        match runs.last_mut() {
            Some((c, d)) if *d == s.dur => *c += 1,
            _ => runs.push((1, s.dur)),
        }
    }
    let mut x = W::new();
    x.u32(0);
    x.u32(runs.len() as u32);
    for (c, d) in runs {
        x.u32(c);
        x.u32(d);
    }
    x.buf
}

fn box_ctts(samples: &[Samp]) -> Option<Vec<u8>> {
    if samples.iter().all(|s| s.cto == 0) {
        return None;
    }
    let mut runs: Vec<(u32, i64)> = Vec::new();
    for s in samples {
        match runs.last_mut() {
            Some((c, o)) if *o == s.cto => *c += 1,
            _ => runs.push((1, s.cto)),
        }
    }
    // version 1 carries signed composition offsets.
    let mut x = W::new();
    x.u32(1 << 24); // version 1, flags 0
    x.u32(runs.len() as u32);
    for (c, o) in runs {
        x.u32(c);
        x.u32((o as i32) as u32);
    }
    Some(x.buf)
}

fn box_stsz(samples: &[Samp]) -> Vec<u8> {
    let mut x = W::new();
    x.u32(0); // version + flags
    x.u32(0); // sample_size 0 -> per-entry sizes follow
    x.u32(samples.len() as u32);
    for s in samples {
        x.u32(s.size);
    }
    x.buf
}

fn box_stsc(count: u32) -> Vec<u8> {
    let mut x = W::new();
    x.u32(0);
    x.u32(1); // one entry
    x.u32(1); // first_chunk
    x.u32(count); // samples_per_chunk = all of them
    x.u32(1); // sample_description_index
    x.buf
}

fn box_stss(samples: &[Samp]) -> Option<Vec<u8>> {
    let syncs: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.sync)
        .map(|(i, _)| i as u32 + 1)
        .collect();
    // No stss means "every sample is a sync sample"; emit only for mixed tracks.
    if syncs.is_empty() || syncs.len() == samples.len() {
        return None;
    }
    let mut x = W::new();
    x.u32(0);
    x.u32(syncs.len() as u32);
    for n in syncs {
        x.u32(n);
    }
    Some(x.buf)
}

fn build_trak(t: &Track, id: u32, chunk_off: u64, movie_ts: u32) -> Vec<u8> {
    let audio = t.hdlr == *b"soun";
    let mdur = t.media_duration();
    let dur_movie = mdur.saturating_mul(movie_ts as u64) / t.timescale as u64;

    let mut tkhd = W::new();
    tkhd.close(b"tkhd", &box_tkhd(id, dur_movie, t.width, t.height, audio));
    let mut mdhd = W::new();
    mdhd.close(b"mdhd", &box_mdhd(t.timescale, mdur));
    let mut hdlr = W::new();
    hdlr.close(b"hdlr", &box_hdlr(t.hdlr));

    let mut stbl_body = W::new();
    let mut stsd = W::new();
    stsd.close(b"stsd", &box_stsd(&t.entries));
    stbl_body.raw(&stsd.buf);
    let mut stts = W::new();
    stts.close(b"stts", &box_stts(&t.samples));
    stbl_body.raw(&stts.buf);
    if !audio {
        if let Some(c) = box_ctts(&t.samples) {
            let mut cc = W::new();
            cc.close(b"ctts", &c);
            stbl_body.raw(&cc.buf);
        }
        if let Some(s) = box_stss(&t.samples) {
            let mut ss = W::new();
            ss.close(b"stss", &s);
            stbl_body.raw(&ss.buf);
        }
    }
    let mut stsc = W::new();
    stsc.close(b"stsc", &box_stsc(t.samples.len() as u32));
    stbl_body.raw(&stsc.buf);
    let mut stsz = W::new();
    stsz.close(b"stsz", &box_stsz(&t.samples));
    stbl_body.raw(&stsz.buf);
    // stco: one chunk, its absolute offset into the output.
    let mut stco = W::new();
    stco.u32(0);
    stco.u32(1);
    stco.u32(chunk_off as u32);
    let mut stco_box = W::new();
    stco_box.close(b"stco", &stco.buf);
    stbl_body.raw(&stco_box.buf);
    let mut stbl = W::new();
    stbl.close(b"stbl", &stbl_body.buf);

    let mut minf_body = W::new();
    if audio {
        let mut smhd = W::new();
        smhd.close(b"smhd", &box_smhd());
        minf_body.raw(&smhd.buf);
    } else {
        let mut vmhd = W::new();
        vmhd.close(b"vmhd", &box_vmhd());
        minf_body.raw(&vmhd.buf);
    }
    minf_body.raw(&box_dinf());
    minf_body.raw(&stbl.buf);
    let mut minf = W::new();
    minf.close(b"minf", &minf_body.buf);

    let mut mdia_body = W::new();
    mdia_body.raw(&mdhd.buf);
    mdia_body.raw(&hdlr.buf);
    mdia_body.raw(&minf.buf);
    let mut mdia = W::new();
    mdia.close(b"mdia", &mdia_body.buf);

    let mut trak_body = W::new();
    trak_body.raw(&tkhd.buf);
    trak_body.raw(&mdia.buf);
    let mut trak = W::new();
    trak.close(b"trak", &trak_body.buf);
    trak.buf
}

fn build_moov(video: &Track, audio: Option<&Track>, video_off: u64, audio_off: u64) -> Vec<u8> {
    const MOVIE_TS: u32 = 1_000_000;
    let dur_of = |t: &Track| t.media_duration().saturating_mul(MOVIE_TS as u64) / t.timescale as u64;
    let movie_dur = dur_of(video).max(audio.map(dur_of).unwrap_or(0));

    let mut mvhd = W::new();
    mvhd.close(b"mvhd", &box_mvhd(MOVIE_TS, movie_dur, if audio.is_some() { 3 } else { 2 }));

    let mut traks = W::new();
    traks.raw(&build_trak(video, 1, video_off, MOVIE_TS));
    if let Some(a) = audio {
        traks.raw(&build_trak(a, 2, audio_off, MOVIE_TS));
    }

    let mut body = W::new();
    body.raw(&mvhd.buf);
    body.raw(&traks.buf);
    let mut moov = W::new();
    moov.close(b"moov", &body.buf);
    moov.buf
}

fn box_ftyp() -> Vec<u8> {
    let mut x = W::new();
    x.raw(b"isom");
    x.u32(0x200); // minor
    x.raw(b"isom");
    x.raw(b"mp42");
    x.raw(b"mp41");
    let mut f = W::new();
    f.close(b"ftyp", &x.buf);
    f.buf
}

// The entry point. Read both fragmented sources, build the track model, size the
// header, then stream media bytes from the sources into the output (so a large
// video is copied, never held twice).
pub fn mux(video_path: &Path, audio_path: Option<&Path>, out_path: &Path) -> Result<(), MuxError> {
    let vbuf = std::fs::read(video_path).map_err(|e| MuxError::msg(format!("read video: {e}")))?;
    let video = read_track(&vbuf)?;
    let audio = match audio_path {
        Some(p) => {
            let abuf = std::fs::read(p).map_err(|e| MuxError::msg(format!("read audio: {e}")))?;
            Some(read_track(&abuf)?)
        }
        None => None,
    };

    let total_v = video.total_bytes();
    let total_a = audio.as_ref().map(|t| t.total_bytes()).unwrap_or(0);

    // Two passes: build the header with offsets of 0 to learn its length, then
    // rebuild with real offsets. Only the 4-byte stco values change, so the
    // length is identical and the absolute media start is now known.
    let ftyp = box_ftyp();
    let probe = build_moov(&video, audio.as_ref(), 0, 0);
    let mdat_payload_start = ftyp.len() as u64 + probe.len() as u64 + 8;
    let video_off = mdat_payload_start;
    let audio_off = video_off + total_v;

    let moov = build_moov(&video, audio.as_ref(), video_off, audio_off);
    if moov.len() != probe.len() {
        return Err(MuxError::msg("header length shifted between passes"));
    }
    let mdat_size = 8u64 + total_v + total_a;
    if mdat_size > u32::MAX as u64 {
        return Err(MuxError::msg("merged media exceeds a 32-bit mdat box"));
    }

    let mut out =
        BufWriter::new(File::create(out_path).map_err(|e| MuxError::msg(format!("create out: {e}")))?);
    out.write_all(&ftyp).map_err(w_err)?;
    out.write_all(&moov).map_err(w_err)?;
    out.write_all(&[(mdat_size as u32).to_be_bytes(), *b"mdat"].concat())
        .map_err(w_err)?;

    stream_samples(&mut out, video_path, &video.samples).map_err(MuxError::msg)?;
    if let Some(a) = &audio {
        let a_path = audio_path.expect("audio track implies a path");
        stream_samples(&mut out, a_path, &a.samples).map_err(MuxError::msg)?;
    }
    out.flush().map_err(w_err)?;
    Ok(())
}

fn w_err(e: std::io::Error) -> MuxError {
    MuxError::msg(format!("write: {e}"))
}

fn stream_samples<W: Write>(out: &mut W, src: &Path, samples: &[Samp]) -> Result<(), String> {
    let mut f = File::open(src).map_err(|e| format!("open src: {e}"))?;
    let mut gap = 0u64;
    let mut tmp = vec![0u8; 1 << 16];
    for s in samples {
        if s.off > gap {
            f.seek(SeekFrom::Start(gap)).map_err(|e| format!("seek: {e}"))?;
            copy_skip(&mut f, s.off - gap, &mut tmp).map_err(|e| format!("skip: {e}"))?;
        }
        write_n(out, &mut f, s.size as usize, &mut tmp).map_err(|e| format!("copy: {e}"))?;
        gap = s.off + s.size as u64;
    }
    Ok(())
}

fn write_n<R: Read, W: Write>(w: &mut W, r: &mut R, n: usize, tmp: &mut [u8]) -> Result<(), String> {
    let mut left = n;
    while left > 0 {
        let take = left.min(tmp.len());
        let got = read_at_least(r, &mut tmp[..take])?;
        if got == 0 {
            return Err("short media".into());
        }
        w.write_all(&tmp[..got]).map_err(|e| e.to_string())?;
        left -= got;
    }
    Ok(())
}

fn copy_skip<R: Read>(r: &mut R, n: u64, tmp: &mut [u8]) -> Result<(), String> {
    let mut left = n;
    while left > 0 {
        let take = left.min(tmp.len() as u64) as usize;
        let got = read_at_least(r, &mut tmp[..take])?;
        if got == 0 {
            return Err("short source".into());
        }
        left -= got as u64;
    }
    Ok(())
}

fn read_at_least<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize, String> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests;
