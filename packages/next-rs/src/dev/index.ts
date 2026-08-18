/**
 * `next-rs dev` — the development watcher (spec §81).
 */

export {
  NEXT_PROCESS,
  RENDERER_PROCESS,
  SERVER_PROCESS,
  startDevSession,
  type DevOptions,
  type DevSession,
} from './session'

export {
  DEFAULT_KILL_TIMEOUT_MS,
  Supervisor,
  nodeSpawner,
  type ManagedProcess,
  type ProcessSpec,
  type Spawner,
  type SupervisorOptions,
} from './supervisor'

export {
  DEFAULT_DEBOUNCE_MS,
  DEFAULT_WATCH_DIRS,
  batchKind,
  classify,
  isIgnored,
  startWatcher,
  type ChangeKind,
  type FileChange,
  type WatchOptions,
  type Watcher,
} from './watcher'
