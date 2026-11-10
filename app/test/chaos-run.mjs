// Runs the chaos server for Rust tests: prints one JSON line with the base URL,
// then stays alive until stdin closes. Sized by SUCR_CHAOS_SIZE (default 4 MiB).
import { startChaos } from './chaos-server.mjs';

const size = Number(process.env.SUCR_CHAOS_SIZE ?? 4 * 1024 * 1024);
const server = await startChaos({ size });
process.stdout.write(
  JSON.stringify({ url: server.url('/file'), base: server.base, size: server.size, etag: server.etag, lastModified: server.lastModified }) + '\n',
);

process.stdin.resume();
const stop = () => {
  server.close().finally(() => process.exit(0));
};
process.stdin.on('end', stop);
process.on('SIGTERM', stop);
