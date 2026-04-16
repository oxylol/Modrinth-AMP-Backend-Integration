// FORK: Concrete AmpBackend implementation for the Tauri app.
//
// Provided at App.vue root so all server panel components can inject it.
// Uses Tauri invoke() for commands and listen() for streamed events.

import { listen } from '@tauri-apps/api/event'
import type {
	AmpBackend,
	AmpConnectionDraft,
	AmpConsoleLineEvent,
	AmpPowerAction,
	AmpPowerStateEvent,
	AmpSubscriptionHandlers,
	AmpUnsubscribe,
} from '@modrinth/ui'
import * as amp from '@/helpers/amp'

export function createTauriAmpBackend(): AmpBackend {
	return {
		testConnection(draft: Omit<AmpConnectionDraft, 'friendlyName'>) {
			return amp.testConnection(draft.baseUrl, draft.username, draft.password, draft.insecureTls)
		},

		addConnection(draft: AmpConnectionDraft) {
			return amp.addConnection(draft)
		},

		removeConnection(connectionId: string) {
			return amp.removeConnection(connectionId)
		},

		listConnections() {
			return amp.listConnections()
		},

		listInstances(connectionId: string) {
			return amp.listInstances(connectionId)
		},

		power(serverId: string, action: AmpPowerAction) {
			return amp.power(serverId, action as 'Start' | 'Stop' | 'Restart' | 'Kill')
		},

		sendCommand(serverId: string, command: string) {
			return amp.sendCommand(serverId, command)
		},

		async getStatus(serverId: string): Promise<AmpPowerStateEvent> {
			return amp.getStatus(serverId)
		},

		async subscribe(
			serverId: string,
			handlers: AmpSubscriptionHandlers,
		): Promise<AmpUnsubscribe> {
			await amp.subscribe(serverId)

			const unlistenConsole = handlers.onConsole
				? await listen<AmpConsoleLineEvent>(`amp://console/${serverId}`, (e) => {
						handlers.onConsole!(e.payload)
					})
				: null

			const unlistenStatus = handlers.onStatus
				? await listen<AmpPowerStateEvent>(`amp://status/${serverId}`, (e) => {
						handlers.onStatus!(e.payload)
					})
				: null

			return async () => {
				unlistenConsole?.()
				unlistenStatus?.()
				await amp.unsubscribe(serverId)
			}
		},
	}
}
