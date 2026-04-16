// FORK: AMP backend DI contract.
//
// This file defines the interface shared code can call to drive an AMP-backed
// server without importing Tauri. The concrete implementation is provided at
// the app-frontend root (apps/app-frontend/src/App.vue) and uses Tauri
// invoke() + listen() under the hood. Web frontends don't provide a backend,
// so `injectAmpBackend(null)` yields null and AMP features gracefully no-op.

import { createContext } from './create-context'

export type AmpPowerAction = 'Start' | 'Stop' | 'Restart' | 'Kill'

export interface AmpPowerStateEvent {
	serverId: string
	powerState: 'stopped' | 'starting' | 'running' | 'stopping' | 'crashed' | 'unknown'
	cpuPercent: number
	ramUsageBytes: number
	ramTotalBytes: number
	uptimeSeconds: number
	connectionOk: boolean
	connectionError?: string | null
}

export interface AmpConsoleLineEvent {
	serverId: string
	text: string
	level: string
	source?: string | null
	timestampMs: number
}

export interface AmpConnection {
	connectionId: string
	baseUrl: string
	username: string
	friendlyName: string
	insecureTls: boolean
}

export interface AmpConnectionDraft {
	baseUrl: string
	username: string
	password: string
	friendlyName: string
	insecureTls: boolean
}

export interface AmpInstanceInfo {
	instanceId: string
	friendlyName: string
	module: string
	running: boolean
	serverId: string
}

export interface AmpTestResult {
	ok: boolean
	message: string
	instanceCount: number
	panelName?: string | null
}

export interface AmpSubscriptionHandlers {
	onConsole?: (line: AmpConsoleLineEvent) => void
	onStatus?: (status: AmpPowerStateEvent) => void
}

export type AmpUnsubscribe = () => Promise<void> | void

export interface AmpBackend {
	testConnection(draft: Omit<AmpConnectionDraft, 'friendlyName'>): Promise<AmpTestResult>
	addConnection(draft: AmpConnectionDraft): Promise<AmpConnection>
	removeConnection(connectionId: string): Promise<void>
	listConnections(): Promise<AmpConnection[]>
	listInstances(connectionId: string): Promise<AmpInstanceInfo[]>
	power(serverId: string, action: AmpPowerAction): Promise<void>
	sendCommand(serverId: string, command: string): Promise<void>
	getStatus(serverId: string): Promise<AmpPowerStateEvent>
	subscribe(serverId: string, handlers: AmpSubscriptionHandlers): Promise<AmpUnsubscribe>
}

export const [injectAmpBackend, provideAmpBackend] = createContext<AmpBackend>(
	'root',
	'ampBackend',
)

const AMP_SERVER_ID_PREFIX = 'amp_'

export function isAmpServerId(serverId: string | null | undefined): boolean {
	return !!serverId && serverId.startsWith(AMP_SERVER_ID_PREFIX)
}
