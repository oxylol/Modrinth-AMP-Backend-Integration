# Architecture Notes — Modrinth App Hosting Tab

Factual map of existing code, written before any AMP work begins.
Fork: oxylol/Modrinth-AMP-Backend-Integration
Date: 2026-04-16

---

## Key finding up front

The Modrinth desktop app does **not** route server operations through Tauri/Rust commands.
The Rust layer only handles OAuth credentials (`plugin:mr-auth`).
All server control (power, console, file browser, stats) goes directly from TypeScript to Modrinth's
cloud infrastructure via HTTPS + WebSocket, using the `@modrinth/api-client` package.
The `TauriModrinthClient` uses Tauri's `plugin-http` so requests pass through Rust's HTTP stack
(bypassing browser CORS and the WebView sandbox), but the business logic is entirely TypeScript.

This means the AMP backend will need to add a **new** Rust Tauri plugin for its polling loop,
credential storage, and cert handling — there is no existing Rust server abstraction to plug into.

---

## 1. Repository layout (relevant to hosting)

```
packages/
  api-client/                 HTTP + WebSocket client, platform-aware
    src/
      core/
        abstract-module.ts    Base class for all API modules
        abstract-websocket.ts WebSocket event bus interface
      features/
        auth.ts               Injects Bearer token on every request
        retry.ts              Exponential backoff on 5xx / network errors
        circuit-breaker.ts    Opens after N consecutive failures
      modules/
        archon/
          servers/v0.ts       Power, reinstall, get, list, auth endpoints
          backups/            Backup management
          content/            Mod/pack content management
          types.ts            All Archon data types (Server, WSEvent, etc.)
        kyros/
          files/v0.ts         Filesystem operations (list, upload, download)
          logs/v1.ts          Log clearing
        labrinth/             Main Modrinth API (projects, versions, etc.)
      platform/
        websocket-generic.ts  Browser WebSocket impl (auto-reconnect, backoff)
        tauri.ts              TauriModrinthClient using @tauri-apps/plugin-http
      clients/
        generic.ts            GenericModrinthClient (attaches WS client to archon.sockets)

packages/ui/
  src/
    composables/
      server-console.ts             Console ring buffer + batching + log-level detection
      server-manage-core-runtime.ts WebSocket event subscriptions + server state machine
    layouts/
      wrapped/hosting/manage/
        root.vue              Server panel host: tabs, WS init, install flow, WS events
        overview.vue          Stats + console + crash analysis
        files.vue             File browser (Kyros)
        content.vue           Mod/pack content management
      shared/
        console/layout.vue    Full console UI (filter, export, crash analysis overlay)

apps/app-frontend/
  src/pages/hosting/
    manage/Index.vue          Route entry → ServersManageRootLayout
    Index.vue                 Server list page (TanStack Query → archon.servers_v0.list)

apps/app/
  src/api/mr_auth.rs          OAuth login + credential get/logout via theseus
  src/main.rs                 Tauri plugin registration
```

---

## 2. Core data types

File: `packages/api-client/src/modules/archon/types.ts`

```typescript
// Server identity and metadata
Archon.Servers.v0.Server {
  server_id: string       // UUID (e.g. "a1b2c3d4-...")
  name: string
  owner_id: string
  status: 'installing' | 'broken' | 'available' | 'suspended'
  loader: 'Forge' | 'NeoForge' | 'Fabric' | 'Quilt' | 'Vanilla' | 'Paper' | 'Purpur' | 'Spigot'
  loader_version: string
  mc_version: string
  upstream: { project_id, version_id } | null   // modpack source
  node: string            // node hostname
  datacenter: string
  flows: { intro: boolean }  // onboarding state
}

// Power states
Archon.Websocket.v0.PowerState = 'stopped' | 'starting' | 'running' | 'stopping' | 'crashed'
Archon.Websocket.v0.FlattenedPowerState = 'not_ready' | 'starting' | 'running' | 'stopping' | 'idle'

// WebSocket event union
Archon.Websocket.v0.WSEvent (discriminated union on .event field):
  WSLogEvent        { event: 'log', message: string }
  WSLog4jEvent      { event: 'log4j', ... }
  WSStatsEvent      { event: 'stats', cpu_percent, ram_usage_bytes, ram_total_bytes,
                       storage_usage_bytes, storage_total_bytes }
  WSPowerStateEvent { event: 'power-state', state: PowerState }
  WSStateEvent      { event: 'state', power_state: FlattenedPowerState, was_oom,
                       exit_code, uptime_seconds, installing: boolean,
                       install_stage, install_stage_progress }
  WSUptimeEvent     { event: 'uptime', uptime_seconds: number }
  + backup-progress, filesystem-ops, installation-result, new-mod, auth-ok, auth-expiring

// WebSocket auth (fetched before connecting)
Archon.Websocket.v0.WSAuth { url: string, token: string }

// Filesystem auth (fetched before file ops)
Archon.Servers.v0.JWTAuth { url: string, token: string }
```

Console line type (defined in `packages/ui/src/composables/server-console.ts`):
```typescript
LogLine { text: string, level: 'info' | 'warn' | 'error' | 'debug' | 'trace' | 'unknown' }
```

Stats type (from `@modrinth/utils`):
```typescript
Stats {
  current: { cpu_percent, ram_usage_bytes, ram_total_bytes, storage_usage_bytes, storage_total_bytes }
  past:    { ... same ... }
  graph:   { cpu: number[], ram: number[] }  // last 10 data points
}
```

---

## 3. Frontend: server list page

File: `apps/app-frontend/src/pages/hosting/Index.vue`

- Fetches server list with TanStack Query:
  ```typescript
  const { data: servers } = useQuery({
    queryKey: ['servers', 'list'],
    queryFn: () => client.archon.servers_v0.list()
  })
  ```
- Returns `Archon.Servers.v0.ServerGetResponse` (`{ servers: Server[], count: number }`)
- Each server card links to `/hosting/manage/{server_id}`

---

## 4. Server detail panel

Entry: `apps/app-frontend/src/pages/hosting/manage/Index.vue`
Root layout: `packages/ui/src/layouts/wrapped/hosting/manage/root.vue` (~1596 lines)

**Tab structure** (root.vue:845–871):
- Overview → `overview.vue`
- Content → `content.vue`
- Files → `files.vue`
- Backups → (inline in root.vue)

**State provided to child components** via `provideModrinthServerContext()`:
```
composable: packages/ui/src/composables/server-manage-core-runtime.ts

Provides:
  isConnected: Ref<boolean>
  serverPowerState: Ref<PowerState | null>
  stats: Ref<Stats>
  uptimeSeconds: Ref<number>
  backupsState, fsOps, fsQueuedOps, uploadState
  busyReasons: Ref<BusyReason[]>
```

---

## 5. WebSocket connection flow

Source: `packages/ui/src/composables/server-manage-core-runtime.ts` (the main orchestrator)

```
1. root.vue mounts → connectSocket() called (line 1439)
2. client.archon.sockets.safeConnect(serverId)
   a. GET /modrinth/v0/servers/{id}/ws → { url, token }
   b. Opens ws:// or wss://{url}
   c. Sends { event: 'auth', jwt: token }
   d. Server replies { event: 'auth-ok' }
3. server-manage-core-runtime registers subscriptions:
   client.archon.sockets.on(serverId, 'log',         handleLog)
   client.archon.sockets.on(serverId, 'log4j',       handleLog4j)
   client.archon.sockets.on(serverId, 'stats',       handleStats)
   client.archon.sockets.on(serverId, 'power-state', handlePowerState)
   client.archon.sockets.on(serverId, 'state',       handleState)
   client.archon.sockets.on(serverId, 'uptime',      handleUptime)
4. Auto-reconnects on drop (exponential backoff: 1s base, 30s max, 10 attempts)
```

WebSocket internals: `packages/api-client/src/platform/websocket-generic.ts`
- `safeConnect()` — idempotent, only opens one connection per serverId
- `on(serverId, event, handler)` — returns unsubscribe function
- `send(serverId, payload)` — serializes to JSON, sends over open socket
- `disconnect(serverId)` — closes and removes connection

---

## 6. Console streaming — full path

```
1. Archon WebSocket emits: { event: 'log', message: "[10:30:45] [Server/INFO]: Done!" }

2. websocket-generic.ts ws.onmessage:
   const data = JSON.parse(e.data) as WSEvent
   this.emitter.emit(`${serverId}:${data.event}`, data)

3. server-manage-core-runtime.ts handleLog():
   modrinthServersConsole.recordWsEvent(...)
   modrinthServersConsole.addLegacyLog(data.message)

4. server-console.ts addLegacyLog(message):
   Split on \r?\n → map each line through textToLogLine()
   → detectLogLevel() via regex (INFO/WARN/ERROR/DEBUG/TRACE/Exception/\tat)
   → push to ring buffer → schedule flush (300ms debounce or 256 lines)

5. Flush: archive old lines, update output.value (reactive Ref<LogLine[]>)

6. overview.vue:
   provideConsoleManager({ logLines: modrinthServersConsole.output, sendCommand })

7. console/layout.vue renders logLines (virtual scroll)
```

Console ring buffer (server-console.ts):
- Capacity: 500,000 lines
- Storage: columnar arrays (`texts: string[]`, `levels: Uint8Array`)
- Continuation grouping: consecutive lines starting with `\t` or `    ` (stack traces) grouped
- `output` ref exposed to UI; updated reactively on flush

---

## 7. Power actions

```typescript
// UI button click (root.vue ~line 140)
await client.archon.servers_v0.power(serverId, 'Start' | 'Stop' | 'Restart' | 'Kill')
// POST /modrinth/v0/servers/{id}/power  { action: 'Start' }
// Response: void (WebSocket then delivers power-state event)
```

State update path: WebSocket `power-state` event → `handlePowerState()` → `serverPowerState.value = data.state`

---

## 8. Console command send

```typescript
// overview.vue line 112
client.archon.sockets.send(serverId, { event: 'command', cmd: '/say hello' })
// Sent over open WebSocket
```

---

## 9. File browser

Requires separate JWT from Archon:
```typescript
const auth = await client.archon.servers_v0.getFilesystemAuth(serverId)
// GET /modrinth/v0/servers/{id}/fs → { url, token }
// All file ops then go to https://{auth.url} with Bearer {auth.token}
```

Kyros API module (`packages/api-client/src/modules/kyros/files/v0.ts`):
- `listDirectory(path, page, limit)`
- `uploadFile(path, file, options)` — XHR upload with progress
- `downloadFile(path)`
- `deleteFile(path)` / `moveFile(src, dst)`

---

## 10. Crash detection

File: `packages/ui/src/layouts/wrapped/hosting/manage/overview.vue:77–146`

```typescript
// Watch for crashed power state
watch(() => serverPowerState.value, (newVal) => {
  if (newVal === 'crashed') inspectError()
})

// inspectError():
const blob = await client.kyros.files_v0.downloadFile('/logs/latest.log')
const log = await blob.text()
const data = await client.mclogs.insights_v1.analyse(log)
if (data.analysis?.problems?.length) crashAnalysis.value = data
```

Dismiss stored per-server in localStorage with 30-minute cooldown.
No pattern-based crash detection in Rust — fully handled by MCLogs API analysis.

---

## 11. Auth / credential storage

Rust side: `apps/app/src/api/mr_auth.rs`
```rust
#[tauri::command] async fn get() -> Result<Option<ModrinthCredentials>> {
    Ok(theseus::mr_auth::get_credentials().await?)
}
```

`theseus` is an external library (not in this repo). It uses platform OS credential storage
(Windows DPAPI / Credential Manager on Windows, Keychain on macOS, SecretService on Linux).

Frontend: `apps/app-frontend/src/helpers/mr_auth.ts`
```typescript
export const get = () => invoke<ModrinthCredentials | null>('plugin:mr-auth|get')
type ModrinthCredentials = { session: string, expires: string, user_id: string, active: boolean }
```

The `session` string is a Bearer token injected by `AuthFeature` into every `client.request()` call.

---

## 12. No existing backend abstraction

There is no trait, interface, or provider abstracting "what backend powers this server."
Every server-related call hardcodes `client.archon.*` or `client.kyros.*`.
Call sites to change when adding AMP:
- `packages/ui/src/composables/server-manage-core-runtime.ts` — WS subscribe/unsubscribe, stat/power/log handlers
- `packages/ui/src/layouts/wrapped/hosting/manage/root.vue` — connectSocket(), power actions, install
- `packages/ui/src/layouts/wrapped/hosting/manage/overview.vue` — sendCommand, crash detection
- `packages/ui/src/layouts/wrapped/hosting/manage/files.vue` — getFilesystemAuth, all file ops
- `apps/app-frontend/src/pages/hosting/manage/Index.vue` — server fetch
- `apps/app-frontend/src/pages/hosting/Index.vue` — server list
