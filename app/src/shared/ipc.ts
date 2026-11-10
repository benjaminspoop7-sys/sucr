import type {
  AddonReport,
  AppInfo,
  AppPaths,
  BlockMap,
  BrowserSession,
  Category,
  DebugReport,
  Download,
  DownloadFilter,
  DownloadSpec,
  HistoryEvent,
  LinkProbe,
  NetworkList,
  UpdateReport,
} from './models';

/** The one schema. Channels follow `domain:verb-noun` (plan §2.4).
 *
 *  Adding a channel here is a three-part edit, and TypeScript enforces all
 *  three: the handler map in main, the runtime list below, and the renderer
 *  surface `window.sucr` (derived from this interface — never hand-written). */
export interface IpcContract {
  'app:get-info': { args: []; result: AppInfo };
  /** The add bar's quick folders, resolved against the disk: one entry per name
   *  that actually has a directory behind it, on the drive `base` sits on or in
   *  the profile. A name with no folder anywhere is simply absent, so a chip is
   *  never offered for a location that would have to be created first. */
  'app:get-folder-presets': {
    args: [base?: string];
    result: Array<{ preset: 'downloads' | 'desktop' | 'documents'; path: string }>;
  };
  'app:get-paths': { args: []; result: AppPaths };
  /** Whether a folder the user is aiming at is already there. `path` comes back
   *  anchored, so the notice and a later create agree on which folder they mean.
   *  Null for an empty field, which means "the default download folder". */
  'app:folder-state': {
    args: [path: string];
    result: { path: string; exists: boolean; isDir: boolean } | null;
  };
  /** Makes the folder the user asked for. Never overwrites: a file already
   *  holding the name is refused and `error` says so. */
  'app:create-folder': {
    args: [path: string];
    result: { path: string; created: boolean; alreadyPresent: boolean; error: string | null };
  };
  'app:quit': { args: []; result: void };
  /** The shell's folder dialog. Resolves to the chosen absolute path, or null
   *  when the user cancels. Blocking by nature — the dialog owns the thread. */
  'app:pick-folder': { args: []; result: string | null };
  'download:list': { args: [filter?: DownloadFilter]; result: Download[] };
  'download:get': { args: [id: number]; result: Download | null };
  'download:add': { args: [spec: DownloadSpec]; result: Download };
  'download:start': { args: [id: number]; result: Download };
  'download:pause': { args: [id: number]; result: Download };
  'download:remove': {
    args: [id: number, deleteFile?: boolean];
    /** `removed` is false when the job would not retire inside the wait: then
     *  nothing was deleted — not the row, not the partial file — and the two
     *  file flags are simply not meaningful yet. */
    result: { fileDeleted: boolean; filePresent: boolean; removed: boolean };
  };
  'download:open': { args: [id: number]; result: { ok: boolean; error?: string } };
  'download:reveal': { args: [id: number]; result: { ok: boolean; error?: string } };
  'download:map': { args: [id: number]; result: BlockMap | null };
  'download:watch': { args: [id: number]; result: BlockMap | null };
  'download:unwatch': { args: [id: number]; result: void };
  /** Replaces a dead link with a fresh one for the same file. The new URL is
   *  probed first and must honour ranges, keep the same length and carry the
   *  same validator; a refusal leaves the job untouched. On success the job is
   *  started again and keeps the slices it already has. Rejects with the reason. */
  'download:refresh-url': { args: [id: number, newUrl: string]; result: Download };
  /** Checks a link before a job exists, with the engine's own one-byte range
   *  request. Writes nothing. Resolves rather than rejects on a dead link — the
   *  verdict is the data. */
  'download:probe': { args: [url: string]; result: LinkProbe };
  'network:list': { args: []; result: NetworkList };
  'network:set-enabled': { args: [id: number, enabled: boolean]; result: { ok: true } };
  /** Saves an adapter's rate cap and monthly allowance. `null` or a non-positive
   *  number means unlimited, which is the same rule the download and global caps
   *  use. Only `month` is accepted as a period: the schema also allows `day` and
   *  `week`, and a weekly allowance enforced monthly would misreport the number
   *  on the card. Writes the row and re-arms the live limiter, so a save acts on
   *  the transfer already running. All three columns are written together, so a
   *  caller that means to change one must read and resend the other. */
  'network:set-limits': {
    args: [id: number, speedLimit: number | null, allowance: number | null, period: 'month' | null];
    result: { ok: true };
  };
  /** Stores the Basic pair for the proxy the user typed, or forgets it when both
   *  arguments are empty. Only a manual proxy takes a typed pair, and the reply
   *  names the host it was keyed to so the page can say so when the address
   *  changes underneath it. The password never comes back — not in this reply,
   *  not in `app:get-info`, which reports only whether one is stored. Rejects
   *  with the reason when there is no manual proxy, or a password with no name. */
  'proxy:set-auth': {
    args: [username: string, password?: string | null];
    result:
      | { stored: false; forgotten: boolean }
      | { stored: true; host: string; username: string; passwordBytes: number };
  };
  /** Stores the credential for the host one link points at, and says which kind it is
   *  with the fourth argument: `'basic'` (the default, a name and a password) or
   *  `'header-secret'` (a bearer token, pasted whole). Forgetting needs no kind — an
   *  empty secret is the forget for either, and the host is the only key.
   *
   *  The first argument may be a host or a whole link: both reduce to the same key the
   *  request is answered by, which is the host alone — so a credential typed for
   *  `files.example` is sent to every port of that host, where a browser would key it to
   *  the exact origin. One credential per host, either way: writing the other kind
   *  replaces what was there, because a request carries one `Authorization`, not two.
   *  The secret never comes back, in this reply or in `app:get-info`. */
  'origin:set-auth': {
    args: [
      link: string,
      username?: string | null,
      password?: string | null,
      kind?: 'basic' | 'header-secret' | null,
    ];
    result:
      | { stored: false; host: string; forgotten: boolean }
      | {
          stored: true;
          host: string;
          kind: 'basic' | 'header-secret';
          username: string;
          secretBytes: number;
        };
  };
  'category:list': { args: []; result: Category[] };
  'history:tail': { args: [downloadId: number, limit?: number]; result: HistoryEvent[] };
  /** Files one download under a category, or clears it with null. The extension
   *  only decides the starting category at add time; this is the override, and it
   *  does not move the file on disk. Rejects when either row is missing. */
  'download:set-category': {
    args: [id: number, categoryId: number | null];
    result: Download;
  };
  'download:set-speed-limit': {
    args: [id: number, bytesPerSec: number | null];
    result: Download;
  };
  'app:set-speed-limit': { args: [bytesPerSec: number | null]; result: AppInfo };
  'settings:list': { args: []; result: Record<string, string> };
  'settings:set': { args: [key: string, value: string | null]; result: void };
  /** Hosts whose browser session the user pasted, oldest first. Never carries
   *  the cookie value — a secret does not need to reach the page to be listed. */
  'session:list': { args: []; result: BrowserSession[] };
  /** Stores one host's session, replacing what was there. The first argument may
   *  be a host or a whole link; both reduce to the identity a job looks up. The
   *  user agent matters: Cloudflare binds `cf_clearance` to the UA that earned
   *  it, so a session pasted without it will be refused again. Rejects with the
   *  reason when the cookie cannot be a header value. */
  'session:set': {
    args: [link: string, cookie: string, userAgent?: string | null];
    result: { host: string; userAgent: string | null; cookieBytes: number };
  };
  'session:forget': { args: [link: string]; result: { host: string; forgotten: boolean } };
  /** The bundled browser add-on: what is staged, which browsers exist, and the
   *  one toggle each still needs. `install` re-stages the folder beside the
   *  database and re-registers the relay for every root a browser reads;
   *  `open` starts one *registered* browser at its own add-on page; `folder`
   *  reveals the staged copy in Explorer. */
  'addon:setup': {
    args: [action?: 'state' | 'install' | 'open' | 'folder', target?: string | null];
    result:
      | AddonReport
      | { ok: boolean; addon?: AddonReport; error?: string; url?: string; browser?: string; path?: string };
  };
  /** Reads the release feed saved in `update.feed_url` and compares its version
   *  with the running build. `open` hands the installer link to Windows instead
   *  of downloading it here, so the browser owns the file and its warnings. */
  'update:check': {
    args: [action?: 'check' | 'open', url?: string];
    result: UpdateReport;
  };
  /** The debug log beside the database. `state` reads the live switch, the file
   *  size and its last few lines; `clear` empties it; `open` reveals the folder
   *  in Explorer. The log is written by the app, never sent anywhere. */
  'debug:report': {
    args: [action?: 'state' | 'open' | 'clear' | 'issue'];
    result: DebugReport | { path: string; bytes?: number } | { ok: boolean; url: string; error?: string };
  };
}

export type Channel = keyof IpcContract;
export type Args<C extends Channel> = IpcContract[C]['args'];
export type Result<C extends Channel> = IpcContract[C]['result'];

type Domain<C extends string> = C extends `${infer D}:${string}` ? D : never;
type Verb<C extends string> = C extends `${string}:${infer V}` ? V : never;
type Camel<S extends string> = S extends `${infer A}-${infer B}` ? `${A}${Capitalize<Camel<B>>}` : S;

/** `window.sucr` has exactly one method per channel, grouped by domain. */
export type SucrApi = {
  [D in Domain<Channel>]: {
    [C in Channel as Domain<C> extends D ? Camel<Verb<C>> : never]: (
      ...args: Args<C>
    ) => Promise<Result<C>>;
  };
};

/** Runtime mirror of the schema, consumed by preload and main. */
export const CHANNELS = [
  'app:get-info',
  'app:get-folder-presets',
  'app:folder-state',
  'app:create-folder',
  'app:get-paths',
  'app:quit',
  'app:pick-folder',
  'download:list',
  'download:get',
  'download:add',
  'download:start',
  'download:pause',
  'download:remove',
  'download:open',
  'download:reveal',
  'download:map',
  'download:watch',
  'download:unwatch',
  'download:refresh-url',
  'download:probe',
  'network:list',
  'network:set-enabled',
  'network:set-limits',
  'proxy:set-auth',
  'origin:set-auth',
  'category:list',
  'history:tail',
  'download:set-category',
  'download:set-speed-limit',
  'app:set-speed-limit',
  'settings:list',
  'settings:set',
  'session:list',
  'session:set',
  'session:forget',
  'addon:setup',
  'update:check',
  'debug:report',
] as const satisfies readonly Channel[];

type AssertNever<T extends never> = T;
// Fails to compile if a channel is added to IpcContract but not to CHANNELS.
type _AllChannelsListed = AssertNever<Exclude<Channel, (typeof CHANNELS)[number]>>;

export function splitChannel(channel: Channel): { domain: string; verb: string } {
  const sep = channel.indexOf(':');
  return { domain: channel.slice(0, sep), verb: camel(channel.slice(sep + 1)) };
}

/** Same transformation as the `Camel` type above; the two are tested together. */
export function camel(s: string): string {
  return s.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase());
}
