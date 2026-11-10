/** Row shapes that cross the IPC boundary. Storage columns are snake_case;
 *  these are the camelCase projections the renderer sees. */

export type DownloadStatus =
  | 'queued'
  | 'downloading'
  | 'paused'
  | 'finished'
  | 'verified'
  | 'needs-attention';

export interface Download {
  id: number;
  url: string;
  finalUrl: string | null;
  method: string;
  status: DownloadStatus;
  categoryId: number | null;
  folder: string | null;
  filename: string;
  totalBytes: number | null;
  wantedBytes: number | null;
  verifiedBytes: number;
  blockSize: number | null;
  blockCount: number | null;
  streamsMode: string | null;
  speedLimit: number | null;
  referer: string | null;
  streams: number | null;
  /** The width the job actually opens: the pinned count, or the tunable it falls back to. */
  streamsRequested: number | null;
  /** The most streams this job may reach. Equals `streamsRequested` when a number was pinned. */
  streamsCeiling: number | null;
  /** The width running this second, while the job is. The ramp moves it, so it is
   *  the only number that distinguishes a band of `8–32` from eleven in force. */
  streamsLive?: number | null;
  /** The width the last run stopped at, on a closed row. The registry is empty
   *  once a job ends, so this is what keeps a finished card from quoting only
   *  the number the dial promised. */
  streamsFinal?: number | null;
  checksumAlgo: string | null;
  checksumHex: string | null;
  resumable: boolean | null;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  errorCode: string | null;
  errorHint: string | null;
}

export interface DownloadFilter {
  status?: DownloadStatus;
}

/** What the add bar sends. Only `url` is required. */
export interface DownloadSpec {
  url: string;
  folder?: string;
  filename?: string;
  checksumAlgo?: 'md5' | 'sha1' | 'sha256' | 'sha512';
  checksumHex?: string;
  referer?: string;
  streams?: number;
}

export interface Network {
  id: number;
  ifIndex: number;
  displayName: string;
  kind: string;
  colorId: string | null;
  enabled: boolean;
  speedLimit: number | null;
  dataLimit: number | null;
  dataLimitPeriod: string | null;
  /** What Windows reports, not what we guess: the address a bound socket uses. */
  localIp: string;
  gateway: string | null;
  linkSpeedBps: number;
  ipv4Metric: number;
  virtual: boolean;
  /** Bytes this adapter carried, booked by the engine when an attempt ends. */
  usedToday: number;
  usedWeek: number;
  usedMonth: number;
  /** Every day this adapter has a `net_usage` row for, not just this month. */
  usedLifetime: number;
}

/**
 * What `network:list` answers with: the enumerated adapters, and the bytes the
 * engine booked to none of them — a proxied tunnel, a localhost target, a route
 * whose device has no card. The two are separate because adding the second into
 * the first would charge bytes to an interface that did not carry them; the pane
 * adds them for the headline and says which part came from where.
 */
export interface NetworkList {
  adapters: Network[];
  unattributed: {
    today: number;
    week: number;
    month: number;
    lifetime: number;
  };
}

export interface Category {
  id: number;
  name: string;
  folder: string | null;
  ruleTier: string;
  matchAll: boolean;
  sortOrder: number;
}

export interface HistoryEvent {
  id: number;
  downloadId: number;
  ts: number;
  kind: string;
  payload: unknown;
}

/** Sucker Map rows. `blocks` in an event are deltas unless `full` is true. */
export interface MapBlock {
  idx: number;
  offset: number;
  len: number;
  bytesDone: number;
  state: string;
  networkId: string;
  /** The name the cable that finished this chunk had when it carried it. Only a
   *  stored map fills it, and it is what keeps an unplugged adapter from being
   *  renamed "Offline" in a history view: the bytes remember its name. */
  networkLabel?: string | null;
}

export interface MapAttempt {
  id: number;
  networkId: string;
  networkLabel: string;
  blockIdx: number;
  rangeStart: number;
  rangeEnd: number;
  bytes: number;
  state: string;
  startedAt: number;
  durationMs: number;
  reason: string | null;
  detail: string | null;
}

export interface BlockMap {
  id: number;
  rev: number;
  full: boolean;
  blockSize: number;
  total: number;
  blocks: MapBlock[];
  attempts: MapAttempt[];
}

export interface AppPaths {
  dataDir: string;
  dbPath: string;
  logsDir: string;
  downloadsDir: string;
}

export interface AppInfo {
  name: string;
  version: string;
  runtime: string;
  webview: string;
  sqlite: string;
  schemaVersion: number;
  journalMode: string;
  tables: number;
  channels: string[];
  channelCount: number;
  globalSpeedLimit: number | null;
  /** The engine's resolved egress path, word for word: `direct`, `manual
   *  host:port`, `system host:port`, or a mode that resolved to nothing
   *  (`manual direct`). The page shows this instead of the three setting keys,
   *  because `system` is a question only the engine and the environment can answer. */
  proxyVia: string;
  /** What the engine knows about the one proxy credential. Never the password:
   *  `stored` says a row exists, `username` and `host` say whose it is, and
   *  `applies` is the engine's own answer to "is this the proxy now in front of
   *  the app" — so a pair typed for an address that has since been edited is
   *  shown as stranded instead of working. */
  proxyAuth: { stored: boolean; username: string | null; host: string | null; applies: boolean };
  /** Every stored origin credential, oldest first — one per host, unlike the single
   *  proxy pair. The secret is not in this list and cannot be: `secretBytes` is the
   *  only thing said about it. The page needs the list to tell a link that wants a
   *  credential apart from one that refused the credential it was given, and a read
   *  that returns no secrets is the only kind that can be handed over wholesale.
   *  `kind` is which header the row becomes, so a token row can be named as one instead
   *  of showing an empty place where a username would have been. */
  originAuths: OriginAuth[];
  startedAt: number;
  paths: AppPaths;
  smoke: boolean;
  /** What the process read back from the registry at start — never what it
   *  wrote — plus the staged add-on. This is the Settings card's first paint;
   *  the card refetches the same answer through `addon:setup` when the user
   *  asks it to set something up. */
  capture: CaptureSnapshot;
}

/** A link checked before any job exists. `ok` is the whole answer: the same
 *  one-byte range request the engine's own pre-flight makes, so a link that
 *  comes back `ok:false` is a link the engine could not pull. `code` names the
 *  reason (`scheme`, `http-status`, `blocked`, `dns`, …) and `message` is the
 *  engine's own sentence for it. */
export interface LinkProbe {
  ok: boolean;
  code: string;
  status: number | null;
  message?: string;
  totalBytes?: number | null;
  acceptRanges?: boolean;
  filename?: string | null;
  /** Whether this host's pasted browser session was carried on the check. The
   *  value is never returned, only whether it was used. */
  session?: boolean;
  /** Whether this host has a stored credential, asked of the same store the request
   *  was built from. With `code: 'needs-auth'` it is the difference between two
   *  sentences the user can act on differently: save one, or replace the one that
   *  was refused. */
  auth?: boolean;
}

/** One host's stored origin credential, as the settings list shows it. The secret
 *  itself is not here, for the same reason the cookie above is not: it is written
 *  once and never read back over the bridge.
 *
 *  `kind` is the word the credentials table uses for the header this row turns into —
 *  `'basic'` for a name-and-password pair, `'header-secret'` for a bearer token — and it
 *  is the same word the page sends back on a write. A host holds one credential, either
 *  kind, so a row's kind is also the answer to which box should have been filled. */
export interface OriginAuth {
  host: string;
  kind: 'basic' | 'header-secret';
  username: string;
  secretBytes: number;
  createdAt: number;
}

/** One host's pasted browser session, as the settings list shows it. The cookie
 *  itself is not here: it is written once and never read back over the bridge. */
export interface BrowserSession {
  host: string;
  userAgent: string | null;
  cookieBytes: number;
  createdAt: number;
}

/** One browser the machine lists as an internet client, and whether Sucr's
 *  native-messaging host is registered in the root that browser reads. A
 *  sideload cannot be pushed into a browser, so `registered` is about the
 *  relay, not the add-on: loading the folder is still one gesture the user
 *  performs at `addonsUrl`. */
export interface AddonBrowser {
  name: string;
  family: 'chrome' | 'firefox';
  familyLabel: string;
  exe: string;
  exists: boolean;
  registered: boolean;
  addonsUrl: string;
}

/** The bundled capture add-on as the Settings card sees it. */
export interface AddonReport {
  staged: {
    chrome: string;
    firefox: string;
    chromePresent: boolean;
    firefoxPresent: boolean;
  };
  browsers: AddonBrowser[];
}

/** The `capture` key of `app:get-info`. `ok:false` is a platform without the
 *  registry, where only the add-on half can be answered. */
export interface CaptureSnapshot {
  ok: boolean;
  exe?: string;
  host?: string | null;
  hostIsCurrentExe?: boolean;
  browsers: Array<{ name: string; family: string; registered: boolean; manifest: string | null }>;
  addon: AddonReport;
  dataDir?: string;
}

/** The answer to `update:check`. `available` is the only field a page needs to
 *  decide whether to show the button; `error` is set when the feed could not be
 *  read at all, and is separate from "read fine, nothing newer". */
export interface UpdateReport {
  ok: boolean;
  current: string;
  latest?: string;
  available: boolean;
  url?: string;
  notes?: string;
  error?: string;
}

/** The answer to `debug:report` action `state`. `enabled` is the live sink, not
 *  the stored key, so a switch that could not open its file still reads off. */
export interface DebugReport {
  enabled: boolean;
  path: string;
  bytes: number;
  tail: string;
}
