// A local server that misbehaves on purpose. Every range request can drop,
// stall, corrupt or cut at an exact byte, with ETag/If-Range support, so the
// engine's resume, retry, integrity and publish paths are exercised against
// real sockets instead of mocks.
import http from 'node:http';

export function body(size, seed = 1) {
  const buf = Buffer.allocUnsafe(size);
  for (let i = 0; i < size; i++) buf[i] = (i * 31 + seed * 17) & 0xff;
  return buf;
}

/**
 * Query knobs (per request):
 *   drop=<n>      send n bytes of the served slice, then destroy the socket
 *   stall=<n>     send n bytes, then hold the connection open for hold ms
 *   hold=<ms>     how long a stall holds (default 5000)
 *   cut=<n>       send n bytes then end without finishing the body
 *   corrupt=<n>   flip one byte at n within the served slice
 *   norange=1     ignore Range and answer 200 with the whole file
 *   probe206=1    honour only the 1-byte probe range; answer 200 to real ranges
 *   etag=<tag>    serve with a different ETag (If-Range mismatch)
 *   noetag=1      serve no ETag at all, so Last-Modified is the only validator
 *   modified=<stamp> serve that Last-Modified stamp instead of the fixed one
 *   delay=<ms>    wait before writing headers
 *   bps=<n>       pace the body at n bytes per second
 *   throttle=<n>  answer 429 + Retry-After to the first n real requests
 *                 (the 1-byte range probe is always exempt)
 *   after=<secs>  the Retry-After value a throttle sends (default 1)
 */
export async function startChaos(options = {}) {
  const size = options.size ?? 4 * 1024 * 1024;
  const seed = options.seed ?? 1;
  const content = body(size, seed);
  const etag = options.etag ?? `"sucr-${size}-${seed}"`;
  const lastModified = new Date(1_700_000_000_000).toUTCString();
  const stats = { requests: [], ranges: [], drops: 0, fullBodies: 0, throttled: 0 };
  // Every request carries the same query, so the refusal budget is taken from
  // the first one and spent across the run.
  let throttlesLeft = null;

  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? '/', 'http://127.0.0.1');
    const q = url.searchParams;
    stats.requests.push({ method: req.method, url: req.url, range: req.headers.range ?? null, ifRange: req.headers['if-range'] ?? null });

    const delay = Number(q.get('delay') ?? 0);
    const bps = Number(q.get('bps') ?? 0);
    const dropAt = q.has('drop') ? Number(q.get('drop')) : null;
    const stallAt = q.has('stall') ? Number(q.get('stall')) : null;
    const cutAt = q.has('cut') ? Number(q.get('cut')) : null;
    const corruptAt = q.has('corrupt') ? Number(q.get('corrupt')) : null;
    const holdMs = Number(q.get('hold') ?? 5000);
    const noRange = q.get('norange') === '1';
    const probeOnly = q.get('probe206') === '1';
    const servedEtag = q.get('noetag') === '1' ? null : (q.get('etag') ?? etag);
    const servedModified = q.get('modified') ?? lastModified;
    if (throttlesLeft === null) throttlesLeft = Number(q.get('throttle') ?? 0);
    const afterSecs = Number(q.get('after') ?? 1);

    const answer = () => {
      let start = 0;
      let end = size - 1;
      let status = 200;

      const range = req.headers.range;
      const honour = range && !noRange && !(probeOnly && range !== 'bytes=0-0');
      // Rate limits bite on the real transfers, not on the 1-byte probe that
      // established the plan — which is what a live host does too. A single
      // stream sends no Range at all, so it counts as a real transfer.
      if (throttlesLeft > 0 && range !== 'bytes=0-0') {
        throttlesLeft -= 1;
        stats.throttled += 1;
        res.writeHead(429, { 'Retry-After': String(afterSecs) });
        res.end();
        return;
      }
      if (honour) {
        const m = /^bytes=(\d+)-(\d*)$/.exec(range);
        if (m) {
          start = Number(m[1]);
          end = m[2] ? Math.min(Number(m[2]), size - 1) : size - 1;
          if (start > end || start >= size) {
            res.writeHead(416, { 'Content-Range': `bytes */${size}` });
            res.end();
            return;
          }
          status = 206;
          stats.ranges.push([start, end]);
        }
      }

      const ifRange = req.headers['if-range'];
      if (ifRange && ifRange !== servedEtag) {
        status = 200;
        start = 0;
        end = size - 1;
      }

      const slice = content.subarray(start, end + 1);
      const headers = {
        'Content-Type': 'application/octet-stream',
        'Accept-Ranges': noRange ? 'none' : 'bytes',
        'Last-Modified': servedModified,
        'Content-Length': String(slice.length),
      };
      // A literal `null` here is sent as the four-character string "null", so
      // the header is left out of the object entirely when the knob asks for it.
      if (servedEtag !== null) headers.ETag = servedEtag;
      if (status === 206) headers['Content-Range'] = `bytes ${start}-${end}/${size}`;

      res.writeHead(status, headers);
      if (req.method === 'HEAD') {
        res.end();
        return;
      }
      if (status === 200) stats.fullBodies++;

      let sent = 0;
      let pacedFrom = 0;
      const STEP = bps > 0 ? 16 * 1024 : 64 * 1024;

      const pump = () => {
        if (sent >= slice.length) {
          res.end();
          return;
        }
        if (bps > 0 && pacedFrom === 0) pacedFrom = Date.now();
        const n = Math.min(STEP, slice.length - sent);
        const chunk = Buffer.from(slice.subarray(sent, sent + n));
        if (corruptAt !== null && corruptAt >= sent && corruptAt < sent + n) {
          chunk[corruptAt - sent] ^= 0xff;
        }

        if (dropAt !== null && sent + n > dropAt) {
          const keep = Math.max(0, dropAt - sent);
          if (keep > 0) res.write(chunk.subarray(0, keep));
          res.destroy();
          stats.drops++;
          return;
        }
        if (cutAt !== null && sent + n > cutAt) {
          const keep = Math.max(0, cutAt - sent);
          if (keep > 0) res.write(chunk.subarray(0, keep));
          res.destroy();
          return;
        }
        if (stallAt !== null && sent + n > stallAt) {
          const keep = Math.max(0, stallAt - sent);
          if (keep > 0) res.write(chunk.subarray(0, keep));
          stats.drops++;
          setTimeout(() => res.destroy(), holdMs);
          return;
        }
        res.write(chunk, () => {
          sent += n;
          if (bps > 0) {
            const due = pacedFrom + (sent / bps) * 1000;
            setTimeout(pump, Math.max(0, due - Date.now()));
          } else {
            pump();
          }
        });
      };
      pump();
    };

    if (delay > 0) setTimeout(answer, delay);
    else answer();
  });

  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  const base = `http://127.0.0.1:${address.port}`;

  return {
    url: (pathAndQuery = '/file') => `${base}${pathAndQuery}`,
    base,
    size,
    content,
    etag,
    lastModified,
    stats,
    close: () =>
      new Promise((resolve, reject) => {
        server.closeAllConnections?.();
        server.close((err) => (err ? reject(err) : resolve()));
      }),
  };
}
