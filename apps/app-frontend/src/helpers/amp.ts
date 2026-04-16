import { invoke } from '@tauri-apps/api/core'
import type {
	AmpConnection,
	AmpConnectionDraft,
	AmpInstanceInfo,
	AmpTestResult,
} from '@modrinth/ui'

export type { AmpConnection, AmpConnectionDraft, AmpInstanceInfo, AmpTestResult }

export async function testConnection(
	baseUrl: string,
	username: string,
	password: string,
	insecureTls: boolean = false,
): Promise<AmpTestResult> {
	return invoke('plugin:amp|amp_test_connection', { url: baseUrl, username, password, insecureTls })
}

export async function addConnection(draft: AmpConnectionDraft): Promise<AmpConnection> {
	return invoke('plugin:amp|amp_add_connection', { draft })
}

export async function removeConnection(connectionId: string): Promise<void> {
	return invoke('plugin:amp|amp_remove_connection', { connectionId })
}

export async function listConnections(): Promise<AmpConnection[]> {
	return invoke('plugin:amp|amp_list_connections')
}

export async function listInstances(connectionId: string): Promise<AmpInstanceInfo[]> {
	return invoke('plugin:amp|amp_list_instances', { connectionId })
}

export async function power(
	serverId: string,
	action: 'Start' | 'Stop' | 'Restart' | 'Kill',
): Promise<void> {
	return invoke('plugin:amp|amp_power', { serverId, action })
}

export async function sendCommand(serverId: string, command: string): Promise<void> {
	return invoke('plugin:amp|amp_send_command', { serverId, command })
}

export async function getStatus(serverId: string): Promise<import('@modrinth/ui').AmpPowerStateEvent> {
	return invoke('plugin:amp|amp_get_status', { serverId })
}

export async function subscribe(serverId: string): Promise<void> {
	return invoke('plugin:amp|amp_subscribe', { serverId })
}

export async function unsubscribe(serverId: string): Promise<void> {
	return invoke('plugin:amp|amp_unsubscribe', { serverId })
}
