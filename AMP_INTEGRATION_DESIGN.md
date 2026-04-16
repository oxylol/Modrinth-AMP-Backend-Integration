# AMP Integration Design Proposal

**This document requires your approval before implementation begins.**

Author: Claude Code
Date: 2026-04-16
Assumptions from user answers:
- ADS multi-instance topology (data model must handle N instances per AMP connection)
- Windows only build target
- Plain HTTP LAN (http:// default, https:// supported, self-signed warning visible but not blocking)
- Personal fork (optimize for speed; shared-file edits pragmatic)

---

## 1. The central architectural decision

The existing UI hardcodes `client.archon.*` everywhere (see ARCHITECTURE_NOTES.md §12).
Two approaches to add AMP:

### Option A — Full abstraction (ServerBackend interface, ~30 call sites refactored)
Introduce a `ServerBackend` TypeScript interface, create `ArchonBackend` + `AmpBackend` impls,
inject via `provideServerBackend` / `useServerBackend`. All call sites replaced.

**Pro:** Clean. Long-term maintainable. No if/else spread in shared code.
**Con:** 30+ touch points across shared packages. High rebase surface.

### Option B — Sidecar with targeted routing composable (recommended for personal fork)
Add a single `useServerBackend(serverId)` composable that branches on server ID prefix.
Replace the ~8 _highest-traffic_ call sites in `server-manage-core-runtime.ts` and
`root.vue` / `overview.vue` to go through this composable.
Modrinth-only code paths (install, content, files) keep calling `client.archon.*` directly
behind an `if (!isAmpServer(serverId))` guard — they just early-return / show "not supported" for AMP.

**Pro:** Minimal changes to shared files (each change marked `// FORK: AMP dispatch`).
Rebase-survivable. Faster to ship v1.
**Con:** If/else guards at ~5 places in shared code. Not upstreamable.

**Decision: Option B.**

---

## 2. Server ID namespace

Modrinth server IDs are UUIDs: `"a1b2c3d4-e5f6-..."`
AMP server IDs will use prefix: `"amp_{connectionId}_{instanceId}"`

- `connectionId`: UUID generated locally when user adds an AMP connection
- `instanceId`: Instance ID string from ADS's `ADSModule/GetInstances` (or `"default"` for single-instance AMP)

The `isAmpServer(id: string): boolean` helper checks `id.startsWith('amp_')`.
`parseAmpServerId(id)` splits on `_` returning `{ connectionId, instanceId }`.

---

## 3. AMP API fundamentals

All AMP requests are `POST /API/{Module}/{Method}` with JSON body.

**Auth flow:**
```
POST /API/Core/Login
body: { username, password, token: "", rememberMe: false }
→ { sessionID: "...", ... }

Include in every subsequent request body: { SESSIONID: sessionID }
```

Session expires → re-login automatically (detect via `result: "Failure"` in response,
or HTTP 401).

**Key endpoints (MVP):**
```
Core/GetStatus     → { State, CPU, MemUsage, MaxRAMUsage, Uptime, ... }
Core/Start         → void
Core/Stop          → void
Core/Restart       → void
Core/Kill          → void
Core/GetUpdates    → { ConsoleEntries: [{Contents, Source, Type}], Status: {...}, ... }
Core/SendConsoleMessage  body: { message: string }
ADSModule/GetInstances   → [{ InstanceID, FriendlyName, Endpoints: {ApplicationEndpoint}, ... }]
ADSModule/Servers/{instanceId}/API/{Module}/{Method}  — proxied instance calls
```

**Power state mapping (AMP → Modrinth PowerState):**
```
AMP State 0 (Undefined/Stopped) → 'stopped'
AMP State 5 (Starting)          → 'starting'
AMP State 20 (Running)          → 'running'
AMP State 30 (Stopping)         → 'stopping'
AMP State -1 (Crashed/Failed)   → 'crashed'
```

**GetUpdates delta protocol:**
- First call: pass `LastUpdate: 0`
- Subsequent calls: pass `LastUpdate` from previous response
- Response includes delta of new console lines since last call
- Poll every 500ms from Rust side (2Hz), pause when no subscriber

---

## 4. Rust Tauri plugin — `plugin:amp`

Location: `apps/app/src/api/amp.rs` (new file, ~400 lines)
Registered in `apps/app/src/main.rs` alongside other plugins.

### State managed in Rust

```rust
// apps/app/src/api/amp.rs

struct AmpConnection {
    id: Uuid,             // connectionId
    base_url: String,     // "http://192.168.1.100:8080"
    username: String,
    friendly_name: String,
    // password stored separately in Windows Credential Manager
}

struct AmpInstanceHandle {
    connection_id: Uuid,
    instance_id: String,
    session_id: Arc<RwLock<Option<String>>>,
    console_buf: Arc<RwLock<VecDeque<AmpConsoleLine>>>,  // ring buffer, 10k lines
    last_update: Arc<AtomicU64>,
    poller_handle: Option<JoinHandle<()>>,
}

// Tauri managed state
struct AmpState {
    connections: HashMap<Uuid, AmpConnection>,     // persisted to disk
    instances: HashMap<String, AmpInstanceHandle>, // keyed by "amp_{connId}_{instId}"
}
```

### Credential storage (Windows Credential Manager)

Uses the `keyring` crate (already used by theseus — check `Cargo.toml` to confirm;
if not present, add `keyring = "2"` to `apps/app/Cargo.toml`).

```rust
fn store_amp_password(connection_id: &Uuid, password: &str) -> Result<()>
fn get_amp_password(connection_id: &Uuid) -> Result<String>
fn delete_amp_password(connection_id: &Uuid) -> Result<()>
// keyring service name: "modrinth-app-amp", username: connection_id.to_string()
```

### Connection config persistence

Stored at Tauri's app data dir as `amp_connections.json` (alongside theseus state).
Load on plugin init, save on any mutation.

### Tauri commands exposed (plugin: `amp`)

```rust
#[tauri::command] async fn amp_test_connection(url: String, username: String, password: String)
  -> Result<AmpTestResult>
  // Logs in once, calls Core/GetStatus, returns { ok: bool, error?: string, instanceCount?: usize }

#[tauri::command] async fn amp_add_connection(url: String, username: String, password: String,
  friendly_name: String) -> Result<AmpConnection>
  // Generates UUID, stores password in keyring, saves connection config, starts polling

#[tauri::command] async fn amp_remove_connection(connection_id: String) -> Result<()>
  // Stops pollers, removes from state + disk, deletes keyring entry

#[tauri::command] async fn amp_list_connections() -> Result<Vec<AmpConnection>>

#[tauri::command] async fn amp_list_instances(connection_id: String)
  -> Result<Vec<AmpInstanceInfo>>
  // Calls ADSModule/GetInstances, returns list with { instanceId, friendlyName, status }
  // For single-instance AMP (not ADS): returns one entry with instanceId="default"

#[tauri::command] async fn amp_power(server_id: String, action: String) -> Result<()>
  // action: "Start" | "Stop" | "Restart" | "Kill"
  // Parses server_id prefix, routes to correct connection+instance

#[tauri::command] async fn amp_send_command(server_id: String, command: String) -> Result<()>

#[tauri::command] async fn amp_get_status(server_id: String) -> Result<AmpServerStatus>
  // Returns current status from cached state (does not HTTP-poll)
```

### Tauri events emitted (from poller tasks)

```
amp://console/{server_id}   payload: AmpConsoleLine { text: String, level: String, timestamp: u64 }
amp://status/{server_id}    payload: AmpStatusUpdate { power_state: String, cpu_percent: f32,
                                       ram_usage_bytes: u64, ram_total_bytes: u64 }
```

### Poller task (one per active instance)

```rust
// Tokio task spawned when a subscriber subscribes, parked when no subscribers
loop {
    sleep(Duration::from_millis(500)).await;
    let updates = amp_client.get_updates(&mut last_update_cursor).await?;
    for line in updates.console_entries {
        emit(app, format!("amp://console/{}", server_id), line);
    }
    if status_changed {
        emit(app, format!("amp://status/{}", server_id), status);
    }
}
```

On session expiry: re-login automatically using stored credentials, retry once.

---

## 5. TypeScript — AMP composable

New file: `apps/app-frontend/src/composables/useAmpServer.ts`

```typescript
export function useAmpServer(serverId: Ref<string>) {
  // Tauri invoke wrappers
  const power = (action: 'Start' | 'Stop' | 'Restart' | 'Kill') =>
    invoke('plugin:amp|amp_power', { serverId: serverId.value, action })

  const sendCommand = (command: string) =>
    invoke('plugin:amp|amp_send_command', { serverId: serverId.value, command })

  // Reactive console + stats from Tauri events
  const consoleLines = useModrinthServersConsole()  // reuse existing ring buffer
  const powerState = ref<PowerState>('stopped')
  const stats = ref<Stats>(createInitialStats())

  let unlistenConsole: UnlistenFn | null = null
  let unlistenStatus: UnlistenFn | null = null

  const connect = async () => {
    unlistenConsole = await listen<AmpConsoleLine>(`amp://console/${serverId.value}`, (e) => {
      consoleLines.addLegacyLog(e.payload.text)  // reuses existing log-level detection
    })
    unlistenStatus = await listen<AmpStatusUpdate>(`amp://status/${serverId.value}`, (e) => {
      powerState.value = e.payload.power_state as PowerState
      // map e.payload.cpu_percent / ram_* → Stats shape
    })
  }

  const disconnect = () => {
    unlistenConsole?.()
    unlistenStatus?.()
  }

  return { power, sendCommand, connect, disconnect, powerState, stats,
           console: consoleLines }
}
```

---

## 6. Routing composable — `useServerBackend`

New file: `packages/ui/src/composables/useServerBackend.ts`

```typescript
export function useServerBackend(serverId: Ref<string>) {
  const isAmp = computed(() => serverId.value.startsWith('amp_'))
  const client = injectModrinthClient()

  // Power action
  const power = (action: 'Start' | 'Stop' | 'Restart' | 'Kill') => {
    if (isAmp.value) {
      return invoke('plugin:amp|amp_power', { serverId: serverId.value, action })
    }
    return client.archon.servers_v0.power(serverId.value, action)
  }

  // Console command
  const sendCommand = (cmd: string) => {
    if (isAmp.value) {
      return invoke('plugin:amp|amp_send_command', { serverId: serverId.value, command: cmd })
    }
    return client.archon.sockets.send(serverId.value, { event: 'command', cmd })
  }

  return { isAmp, power, sendCommand }
}
```

---

## 7. Changes to shared files

Changes to shared packages are intentionally minimal. Every line touching existing code
gets a `// FORK: AMP dispatch` comment for rebase tracking.

### `packages/ui/src/composables/server-manage-core-runtime.ts`

- Accept optional `onAmpConnect` / `onAmpDisconnect` callbacks via options object, OR
- Check `isAmpServer(serverId.value)` and skip WS connect/subscribe for AMP servers
- The `connectSocket()` / `disconnectSocket()` functions wrap the existing logic:

```typescript
// FORK: AMP dispatch — AMP servers use Tauri events, not archon WebSocket
if (isAmpServer(serverId.value)) {
  return connectAmpServer(serverId.value, ampHandlers)
}
// existing archon WS connect below unchanged
```

`ampHandlers` receives `powerState`, `stats`, `console lines` exactly as archon WS handlers do,
so `provideModrinthServerContext()` shape is unchanged.

### `packages/ui/src/layouts/wrapped/hosting/manage/root.vue`

- Power button handler: replace `client.archon.servers_v0.power(...)` with `backend.power(...)`
  where `backend = useServerBackend(serverId)` — 1 line change.
- Install/content/reinstall sections: wrap with `v-if="!isAmp"` — AMP servers don't support reinstall.
- Files tab: `v-if="!isAmp"` on the tab button for v1 (file browser is v2).

### `packages/ui/src/layouts/wrapped/hosting/manage/overview.vue`

- `sendCommand`: replace `client.archon.sockets.send(...)` with `backend.sendCommand(cmd)` — 1 line.
- Crash detection (`inspectError`): wrap with `if (!isAmp.value)` — AMP v1 has no crash analysis.

### `apps/app-frontend/src/pages/hosting/Index.vue` (server list)

- Union archon list + AMP instances:
```typescript
const { data: archonServers } = useQuery({ queryFn: () => client.archon.servers_v0.list() })
const { data: ampServers } = useQuery({ queryFn: () => loadAmpServers() })
// loadAmpServers(): invoke amp_list_connections → for each → amp_list_instances → synthesize Server-like objects
const allServers = computed(() => [...(archonServers.value?.servers ?? []), ...(ampServers.value ?? [])])
```

### `apps/app/src/main.rs`

- One line: `.plugin(api::amp::init())` added alongside other plugins.

---

## 8. AMP "server" shape (synthesized to match Archon.Servers.v0.Server)

```typescript
// Synthesized AMP server for the list + card rendering
{
  server_id: `amp_${connectionId}_${instanceId}`,  // our namespaced ID
  name: instance.FriendlyName,
  status: 'available',                              // AMP servers are always "available" if reachable
  loader: mapAmpModuleToLoader(instance.Module),    // 'Vanilla' | 'Paper' etc. from AMP module name
  mc_version: instance.MCVersion ?? '',
  // ... other fields can be empty strings / nulls
  _backend: 'amp' as const,                         // extra field for badge rendering (not in Archon type)
}
```

Server card in the list: add a small `AMP` badge using the existing badge component pattern.
No other visual changes — same card layout.

---

## 9. "Add external server" UI flow

New component: `packages/ui/src/components/servers/AddAmpServerModal.vue`
New page/button: small "Add external server" button on `apps/app-frontend/src/pages/hosting/Index.vue`
alongside the existing "Get a server" / purchase flow.

Modal fields:
1. **AMP URL** — text input, placeholder `http://192.168.1.100:8080`
2. **Username** — text input
3. **Password** — password input
4. **Friendly name** — text input (pre-filled from AMP panel name on successful test)
5. **Accept insecure TLS** — checkbox, hidden unless URL starts with `https://` (plain HTTP gets a
   "connection is unencrypted" warning badge instead)

Flow:
1. User fills URL + credentials, clicks "Test connection"
2. Frontend calls `invoke('plugin:amp|amp_test_connection', ...)`
3. Rust: login → Core/GetStatus → if ADS: ADSModule/GetInstances count
4. On success: show green "Connected — found N instance(s)" + pre-fill friendly name
5. User clicks "Add server" → `invoke('plugin:amp|amp_add_connection', ...)`
6. Modal closes, server list re-queries (invalidate TanStack Query key `['servers', 'amp']`)

---

## 10. Connection health indicator

AMP-specific error states not in Modrinth's existing `PowerState` type:
- `amp-unreachable` — network error during poll
- `amp-auth-failed` — session expired, re-login failed (password changed?)

These are stored in Rust state alongside `instanceHandle` and returned as part of `amp_get_status`.

Server card renders a connection status indicator:
- Green dot = polling OK
- Yellow dot = last poll >5s ago (slow network)
- Red dot = unreachable / auth failed, with tooltip explaining the error

This is AMP-card-only and uses a simple conditional in the server card component.
It does NOT affect the shared `PowerState` type.

---

## 11. v1 scope (implement now)

- [ ] Rust: `apps/app/src/api/amp.rs` — full AMP client + poller + Tauri commands
- [ ] Rust: `apps/app/src/main.rs` — register plugin
- [ ] TS: `apps/app-frontend/src/composables/useAmpServer.ts`
- [ ] TS: `packages/ui/src/composables/useServerBackend.ts`
- [ ] UI: `server-manage-core-runtime.ts` — AMP branch in connect/disconnect
- [ ] UI: `root.vue` — power dispatch, hide install/content tabs for AMP
- [ ] UI: `overview.vue` — command dispatch, no crash analysis for AMP
- [ ] UI: `AddAmpServerModal.vue`
- [ ] UI: `hosting/Index.vue` — union server list, "Add external server" button, AMP badge
- [ ] UI: server card connection health indicator

## 12. v2 scope (after v1 is stable)

- Settings modal driven by `Core/GetSettingsSpec` → map AMP setting types to Modrinth's settings UI
- File browser via `FileManagerPlugin/*` — reuse `files.vue` layout with AMP backend
- Crash detection: `FileManagerPlugin/ReadFileChunk` on `/logs/latest.log` → existing mclogs API
- Full ADS support: UI to browse instances within a connection, add/remove instances independently

---

## 13. Files to create (new)

```
apps/app/src/api/amp.rs                              Rust Tauri plugin
apps/app-frontend/src/composables/useAmpServer.ts    TS Tauri event bridge
packages/ui/src/composables/useServerBackend.ts      Routing composable
packages/ui/src/components/servers/AddAmpServerModal.vue
```

## 14. Files to modify (existing, minimal changes)

```
apps/app/src/main.rs                                 +1 line: register plugin
apps/app/Cargo.toml                                  +keyring dep if not present
apps/app-frontend/src/pages/hosting/Index.vue        union list + Add button
packages/ui/src/composables/server-manage-core-runtime.ts  AMP branch in connect/disconnect
packages/ui/src/layouts/wrapped/hosting/manage/root.vue    power dispatch + v-if guards
packages/ui/src/layouts/wrapped/hosting/manage/overview.vue  command + crash guard
```

---

**Awaiting your approval to proceed to implementation.**
Changes to shared package files are marked above — please confirm you're comfortable with those
touch points before I start writing Rust/TypeScript.
