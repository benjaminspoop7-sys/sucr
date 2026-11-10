// WebM -> WebM remuxer for a DASH pair. The picture and the sound of a
// high-resolution YouTube stream arrive as two single-track Matroska/EBML
// files; this joins them into one two-track WebM by rebuilding Info + Tracks
// and interleaving the source clusters, copying every media byte verbatim.
//
// Like the MP4 muxer next door it is pure byte work on the container grammar:
// no crate, no ffmpeg in the product path. Only one byte per audio block is
// rewritten (its track number, 1 -> 2) because a cluster from the audio file
// is otherwise already a valid cluster for the merged segment.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::mux::MuxError;

const ID_EBML: u64 = 0x1A45_DFA3;
const ID_SEGMENT: u64 = 0x1853_8067;
const ID_INFO: u64 = 0x1549_A966;
const ID_TRACKS: u64 = 0x1654_AE6B;
const ID_CLUSTER: u64 = 0x1F43_B675;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_TRACK_NUMBER: u64 = 0xD7;
const ID_TRACK_UID: u64 = 0x73C5;
const ID_TIME_SCALE: u64 = 0x2AD7_B1;
const ID_DURATION: u64 = 0x4489;
const ID_SIMPLE_BLOCK: u64 = 0xA3;
const ID_BLOCK_GROUP: u64 = 0xA0;
const ID_BLOCK: u64 = 0xA1;
const ID_TIMECODE: u64 = 0xE7;
const ID_MUXING_APP: u64 = 0x4D80;
const ID_WRITING_APP: u64 = 0x5741;

// One cluster can be read into memory whole; a WebM cluster is a few
// megabytes in practice, and a cap keeps a hostile file from ballooning it.
const MAX_CLUSTER: u64 = 256 * 1024 * 1024;

struct Src {
    f: File,
    size: u64,
}

impl Src {
    fn open(path: &Path) -> Result<Self, MuxError> {
        let f = File::open(path).map_err(|e| MuxError(format!("open {}: {e}", path.display())))?;
        let size = f
            .metadata()
            .map_err(|e| MuxError(format!("stat {}: {e}", path.display())))?
            .len();
        Ok(Src { f, size })
    }

    // Read up to `n` bytes; a short answer at end of file is not an error.
    fn read(&mut self, off: u64, n: usize) -> Result<Vec<u8>, MuxError> {
        if off >= self.size {
            return Ok(Vec::new());
        }
        let want = n.min((self.size - off) as usize);
        let mut buf = vec![0u8; want];
        self.f
            .seek(SeekFrom::Start(off))
            .map_err(|e| MuxError(format!("seek: {e}")))?;
        let mut got = 0;
        while got < want {
            match self.f.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(k) => got += k,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(MuxError(format!("read: {e}"))),
            }
        }
        buf.truncate(got);
        Ok(buf)
    }
}

// One EBML element located in a source file: `at` is its header offset, `len`
// the header width, `size` the body length.
#[derive(Clone, Copy)]
struct Elem {
    id: u64,
    at: u64,
    len: u64,
    size: u64,
}

impl Elem {
    fn body(&self) -> u64 {
        self.at + self.len
    }
    fn end(&self) -> u64 {
        self.body() + self.size
    }
}

// An id keeps its marker bit; a size strips it. An all-ones size value means
// "unknown length": the element runs to the end of the file.
fn vint(buf: &[u8], off: usize, keep_marker: bool) -> Option<(u64, usize, bool)> {
    let first = *buf.get(off)?;
    let mut len = 1usize;
    let mut mask = 0x80u16;
    while len <= 8 && (first as u16) & mask == 0 {
        mask >>= 1;
        len += 1;
    }
    if len > 8 || off + len > buf.len() {
        return None;
    }
    let mut val = if keep_marker {
        first as u64
    } else {
        first as u64 & ((mask as u64) - 1)
    };
    for k in 1..len {
        val = (val << 8) | buf[off + k] as u64;
    }
    let unknown = !keep_marker && val == (1u64 << (7 * len)) - 1;
    Some((val, len, unknown))
}

fn size_len(size: u64) -> usize {
    let mut len = 1usize;
    while len < 8 && size >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    len
}

fn put_size(size: u64) -> Vec<u8> {
    let len = size_len(size);
    let mut out = vec![0u8; len];
    let mut v = size;
    for k in (0..len).rev() {
        out[k] = (v & 0xFF) as u8;
        v >>= 8;
    }
    out[0] |= 0x80 >> (len - 1);
    out
}

fn put_id(id: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut v = id;
    while v > 0 {
        bytes.insert(0, (v & 0xFF) as u8);
        v >>= 8;
    }
    bytes
}

fn elem_at(src: &mut Src, off: u64) -> Result<Option<Elem>, MuxError> {
    let head = src.read(off, 16)?;
    if head.len() < 2 {
        return Ok(None);
    }
    let Some((id, id_len, _)) = vint(&head, 0, true) else {
        return Ok(None);
    };
    let Some((size, sz_len, unknown)) = vint(&head, id_len, false) else {
        return Ok(None);
    };
    let len = (id_len + sz_len) as u64;
    let at = off;
    if unknown {
        if id != ID_SEGMENT {
            return Ok(None);
        }
        let size = src.size.saturating_sub(off + len);
        return Ok(Some(Elem { id, at, len, size }));
    }
    if off + len + size > src.size {
        return Err(MuxError(format!(
            "element 0x{id:x} at {off} runs past the end of the file"
        )));
    }
    Ok(Some(Elem { id, at, len, size }))
}

fn find_child(src: &mut Src, start: u64, end: u64, want: u64) -> Result<Option<Elem>, MuxError> {
    let mut off = start;
    while off + 2 <= end {
        let Some(e) = elem_at(src, off)? else {
            return Ok(None);
        };
        if e.id == want {
            return Ok(Some(e));
        }
        let next = e.end();
        if next <= off {
            return Ok(None);
        }
        off = next;
    }
    Ok(None)
}

fn elem(id: u64, body: &[u8]) -> Vec<u8> {
    let mut out = put_id(id);
    out.extend_from_slice(&put_size(body.len() as u64));
    out.extend_from_slice(body);
    out
}

fn uint_bytes(v: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut n = v;
    loop {
        bytes.insert(0, (n & 0xFF) as u8);
        n >>= 8;
        if n == 0 {
            break;
        }
    }
    bytes
}

// Locate a child element inside an in-memory element body.
fn sub_elem(buf: &[u8], want: u64) -> Option<(usize, usize, usize)> {
    let mut off = 0usize;
    while off + 2 <= buf.len() {
        let (id, id_len, _) = vint(buf, off, true)?;
        let (size, sz_len, unknown) = vint(buf, off + id_len, false)?;
        if unknown {
            return None;
        }
        let size = usize::try_from(size).ok()?;
        let body = off + id_len + sz_len;
        if body + size > buf.len() {
            return None;
        }
        if id == want {
            return Some((off, body, size));
        }
        off = body + size;
    }
    None
}

// Rewrite a TrackEntry's track number in place; the encoding width is kept, so
// the element's declared size never changes.
fn set_track_number(entry: &[u8], n: u64) -> Result<Vec<u8>, MuxError> {
    let (_, body, size) = sub_elem(entry, ID_TRACK_NUMBER)
        .ok_or_else(|| MuxError("a TrackEntry has no TrackNumber".into()))?;
    let mut out = entry.to_vec();
    for k in 0..size {
        out[body + k] = ((n >> (8 * (size - 1 - k))) & 0xFF) as u8;
    }
    // The marker bit belongs to the first byte only, and it is already there.
    Ok(out)
}

fn read_uint(entry: &[u8], id: u64) -> Option<u64> {
    let (_, body, size) = sub_elem(entry, id)?;
    let mut v = 0u64;
    for k in 0..size {
        v = (v << 8) | entry[body + k] as u64;
    }
    Some(v)
}

fn set_uint(entry: &mut [u8], id: u64, v: u64) {
    if let Some((_, body, size)) = sub_elem(entry, id) {
        for k in 0..size {
            entry[body + k] = ((v >> (8 * (size - 1 - k))) & 0xFF) as u8;
        }
    }
}

fn read_float(entry: &[u8], id: u64) -> Option<f64> {
    let (_, body, size) = sub_elem(entry, id)?;
    let b = &entry[body..body + size];
    match size {
        4 => Some(f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64),
        8 => Some(f64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ])),
        _ => None,
    }
}

struct Segment {
    header: Vec<u8>,
    body: u64,
    end: u64,
}

fn open_segment(src: &mut Src) -> Result<Segment, MuxError> {
    let top = elem_at(src, 0)?.ok_or_else(|| MuxError("not an EBML file".into()))?;
    if top.id != ID_EBML {
        return Err(MuxError("not an EBML file".into()));
    }
    let header = src.read(0, top.end() as usize)?;
    let seg = elem_at(src, top.end())?.ok_or_else(|| MuxError("no Segment".into()))?;
    if seg.id != ID_SEGMENT {
        return Err(MuxError("no Segment".into()));
    }
    Ok(Segment {
        header,
        body: seg.body(),
        end: seg.end(),
    })
}

struct Cluster {
    at: u64,
    len: u64,
    timecode: u64,
}

// Every top-level cluster of one source, in file order, with its timecode in
// the file's own timestamp scale.
fn clusters(src: &mut Src, seg: &Segment) -> Result<Vec<Cluster>, MuxError> {
    let mut out = Vec::new();
    let mut off = seg.body;
    while off + 2 <= seg.end {
        let Some(e) = elem_at(src, off)? else { break };
        if e.id == ID_CLUSTER {
            if e.size > MAX_CLUSTER {
                return Err(MuxError(format!(
                    "cluster of {} bytes is too large to remux",
                    e.size
                )));
            }
            let probe = src.read(e.body(), (e.size.min(64)) as usize)?;
            let tc = match sub_elem(&probe, ID_TIMECODE) {
                Some((_, body, size)) => {
                    let mut v = 0u64;
                    for k in 0..size.min(8) {
                        v = (v << 8) | probe[body + k] as u64;
                    }
                    v
                }
                None => 0,
            };
            out.push(Cluster {
                at: e.at,
                len: e.end() - e.at,
                timecode: tc,
            });
        }
        let next = e.end();
        if next <= off {
            break;
        }
        off = next;
    }
    Ok(out)
}

// Copy one cluster, moving every block onto track 2 when the cluster came from
// the audio file. Sizes are untouched: only the track-number byte changes.
fn write_cluster(
    src: &mut Src,
    c: &Cluster,
    out: &mut impl Write,
    to_track: Option<u64>,
) -> Result<(), MuxError> {
    let mut buf = src.read(c.at, c.len as usize)?;
    if let Some(n) = to_track {
        let (_, id_len, _) = vint(&buf, 0, true).ok_or_else(|| MuxError("bad cluster id".into()))?;
        let (_, sz_len, _) = vint(&buf, id_len, false)
            .ok_or_else(|| MuxError("bad cluster size".into()))?;
        let start = id_len + sz_len;
        let end = buf.len();
        let mut off = start;
        while off + 2 <= end {
            let Some((id, child_len, _)) = vint(&buf, off, true) else { break };
            let Some((size, sz_len, unknown)) = vint(&buf, off + child_len, false) else {
                break;
            };
            if unknown {
                break;
            }
            let Ok(size) = usize::try_from(size) else { break };
            let body = off + child_len + sz_len;
            let stop = body + size;
            if stop > end {
                break;
            }
            if id == ID_SIMPLE_BLOCK {
                patch_track(&mut buf[body..stop], n)?;
            } else if id == ID_BLOCK_GROUP {
                let mut j = body;
                while j + 2 <= stop {
                    let Some((gid, glen, _)) = vint(&buf[..stop], j, true) else { break };
                    let Some((gsize, gsz_len, _)) = vint(&buf[..stop], j + glen, false) else {
                        break;
                    };
                    let Ok(gsize) = usize::try_from(gsize) else { break };
                    let gbody = j + glen + gsz_len;
                    if gbody + gsize > stop {
                        break;
                    }
                    if gid == ID_BLOCK {
                        patch_track(&mut buf[gbody..gbody + gsize], n)?;
                    }
                    j = gbody + gsize;
                }
            }
            off = stop;
        }
    }
    out.write_all(&buf).map_err(|e| MuxError(format!("write: {e}")))?;
    Ok(())
}

// A block body opens with the track number as a vint. Source files carry track
// 1, so the byte is 0x81; the merged segment's audio is track 2.
fn patch_track(body: &mut [u8], n: u64) -> Result<(), MuxError> {
    let Some((val, len, _)) = vint(body, 0, false) else {
        return Err(MuxError("a block has no track number".into()));
    };
    if val != 1 {
        return Err(MuxError(format!("unexpected block track number {val}")));
    }
    if n > 0x7F || len != 1 {
        return Err(MuxError("audio track number does not fit one byte".into()));
    }
    body[0] = 0x80 | n as u8;
    Ok(())
}

struct Leg {
    src: Src,
    seg: Segment,
    entry: Vec<u8>,
    scale: u64,
    duration: f64,
    clusters: Vec<Cluster>,
}

fn leg(path: &Path, track_number: u64) -> Result<Leg, MuxError> {
    let mut src = Src::open(path)?;
    let seg = open_segment(&mut src)?;
    let info = find_child(&mut src, seg.body, seg.end, ID_INFO)?
        .ok_or_else(|| MuxError("no Info element".into()))?;
    let info_body = src.read(info.body(), info.size as usize)?;
    let tracks = find_child(&mut src, seg.body, seg.end, ID_TRACKS)?
        .ok_or_else(|| MuxError("no Tracks element".into()))?;
    let entry_at = find_child(&mut src, tracks.body(), tracks.end(), ID_TRACK_ENTRY)?
        .ok_or_else(|| MuxError("no TrackEntry".into()))?;
    let mut entry = src.read(entry_at.body(), entry_at.size as usize)?;
    entry = set_track_number(&entry, track_number)?;
    let scale = read_uint(&info_body, ID_TIME_SCALE).unwrap_or(1_000_000);
    let duration = read_float(&info_body, ID_DURATION).unwrap_or(0.0);
    let clusters = clusters(&mut src, &seg)?;
    Ok(Leg {
        src,
        seg,
        entry,
        scale,
        duration,
        clusters,
    })
}

// The entry point. Read both single-track sources, rebuild the header, then
// interleave their clusters in time order, streaming each one through.
pub fn mux(video: &Path, audio: &Path, out: &Path) -> Result<(), MuxError> {
    let mut v = leg(video, 1)?;
    let mut a = leg(audio, 2)?;

    // Two TrackEntries with distinct UIDs; a collision is legal to repair and
    // cheap to prevent.
    let v_uid = read_uint(&v.entry, ID_TRACK_UID);
    let a_uid = read_uint(&a.entry, ID_TRACK_UID);
    if let (Some(vu), Some(au)) = (v_uid, a_uid) {
        if vu == au {
            set_uint(&mut a.entry, ID_TRACK_UID, au.wrapping_add(1));
        }
    }

    let mut tracks_body = elem(ID_TRACK_ENTRY, &v.entry);
    tracks_body.extend_from_slice(&elem(ID_TRACK_ENTRY, &a.entry));
    let tracks = elem(ID_TRACKS, &tracks_body);

    // The merged segment keeps each source's timecodes untouched, so it must
    // keep the picture's timestamp scale; a source authored with a different
    // scale would need every timecode rewritten, which is not a remux.
    if a.scale != v.scale {
        return Err(MuxError(format!(
            "the two sources disagree on TimestampScale ({} vs {})",
            v.scale, a.scale
        )));
    }
    let duration = v.duration.max(a.duration);
    let mut info_body = elem(ID_TIME_SCALE, &uint_bytes(v.scale));
    info_body.extend_from_slice(&elem(ID_DURATION, &duration.to_be_bytes()));
    info_body.extend_from_slice(&elem(ID_MUXING_APP, b"Sucr"));
    info_body.extend_from_slice(&elem(ID_WRITING_APP, b"Sucr"));
    let info = elem(ID_INFO, &info_body);

    // Clusters interleave by timecode, picture first on a tie. Both sources
    // keep their own order, so no stream is reordered against itself.
    let clusters_len: u64 = v.clusters.iter().map(|c| c.len).sum::<u64>()
        + a.clusters.iter().map(|c| c.len).sum::<u64>();
    let seg_len = (info.len() + tracks.len()) as u64 + clusters_len;

    let f = File::create(out).map_err(|e| MuxError(format!("create out: {e}")))?;
    let mut w = BufWriter::new(f);
    w.write_all(&v.seg.header)
        .map_err(|e| MuxError(format!("write: {e}")))?;
    w.write_all(&put_id(ID_SEGMENT))
        .map_err(|e| MuxError(format!("write: {e}")))?;
    w.write_all(&put_size(seg_len))
        .map_err(|e| MuxError(format!("write: {e}")))?;
    w.write_all(&info).map_err(|e| MuxError(format!("write: {e}")))?;
    w.write_all(&tracks).map_err(|e| MuxError(format!("write: {e}")))?;

    let mut vi = 0usize;
    let mut ai = 0usize;
    while vi < v.clusters.len() || ai < a.clusters.len() {
        let take_video = match (v.clusters.get(vi), a.clusters.get(ai)) {
            (Some(vc), Some(ac)) => (vc.timecode, false) <= (ac.timecode, true),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_video {
            write_cluster(&mut v.src, &v.clusters[vi], &mut w, None)?;
            vi += 1;
        } else {
            write_cluster(&mut a.src, &a.clusters[ai], &mut w, Some(2))?;
            ai += 1;
        }
    }
    w.flush().map_err(|e| MuxError(format!("write: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests;
