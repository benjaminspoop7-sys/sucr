import { invoke } from '@tauri-apps/api/core';
import { CHANNELS, splitChannel, type SucrApi } from '../shared/ipc';

/** `window.sucr` is generated from the schema, not hand-written: one method per
 *  channel, named by `domain.verb`. The type in shared/ipc.ts is derived from
 *  the same interface, and the Rust side publishes the same channel list so the
 *  smoke test can prove the two halves agree. */
const surface: Record<string, Record<string, (...args: unknown[]) => Promise<unknown>>> = {};

for (const channel of CHANNELS) {
  const { domain, verb } = splitChannel(channel);
  const group = (surface[domain] ??= {});
  group[verb] = (...args: unknown[]) => invoke('sucr_invoke', { channel, args });
}

/* `download:probe` is the one channel that does not travel through sucr_invoke.
   That command is sync and runs on the main thread, so a link check would freeze
   the window for as long as the host takes to answer (the engine's own connect
   timeout is 30 s). Same name, same result type, different door. */
const downloadSurface = (surface['download'] ??= {});
downloadSurface['probe'] = (url: unknown) => invoke('sucr_probe', { url: String(url) });

export const sucr = surface as unknown as SucrApi;

window.sucr = sucr;

export { invoke };
