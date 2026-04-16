// FORK: AMP dispatch composable.
//
// Wraps the two operations that differ between Modrinth and AMP backends:
//   - power actions (start/stop/restart/kill)
//   - console command send
//
// The AMP-side implementation is injected at runtime by the app-frontend; the
// web frontend never provides it so `injectAmpBackend(null)` is safely null.

import type { Ref } from 'vue'
import { computed } from 'vue'

import { injectModrinthClient } from '../providers/api-client'
import { injectAmpBackend, isAmpServerId } from '../providers/amp-backend'
import type { AmpPowerAction } from '../providers/amp-backend'

export function useServerBackend(serverId: Ref<string | null | undefined>) {
	const client = injectModrinthClient()
	const ampBackend = injectAmpBackend(null)

	const isAmp = computed(() => isAmpServerId(serverId.value))

	async function power(action: AmpPowerAction): Promise<void> {
		const id = serverId.value
		if (!id) return
		if (isAmp.value) {
			if (!ampBackend) throw new Error('AMP backend not available in this context')
			return ampBackend.power(id, action)
		}
		await client.archon.servers_v0.power(id, action as 'Start' | 'Stop' | 'Restart' | 'Kill')
	}

	async function sendCommand(command: string): Promise<void> {
		const id = serverId.value
		if (!id) return
		if (isAmp.value) {
			if (!ampBackend) throw new Error('AMP backend not available in this context')
			return ampBackend.sendCommand(id, command)
		}
		client.archon.sockets.send(id, { event: 'command', cmd: command })
	}

	return { isAmp, power, sendCommand, ampBackend }
}
