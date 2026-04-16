<template>
	<div
		class="transition-all pressable hoverable cursor-pointer"
		role="link"
		:tabindex="0"
		@click="navigateToServer"
		@keydown.enter.self="navigateToServer"
		@keydown.space.prevent.self="navigateToServer"
	>
		<div
			class="flex flex-row items-center overflow-x-hidden rounded-2xl border-[1px] border-solid border-surface-4 bg-bg-raised p-4 transition-all duration-150"
		>
			<div
				class="flex size-16 items-center justify-center rounded-xl border-[1px] border-solid border-button-border bg-button-bg shadow-sm shrink-0"
			>
				<ServerStackIcon class="size-8 text-secondary" />
			</div>

			<div class="ml-4 flex flex-1 flex-col gap-1.5 min-w-0">
				<div class="flex flex-row items-center gap-2">
					<h2 class="m-0 text-xl font-bold text-contrast truncate">{{ name }}</h2>
					<span
						class="inline-flex shrink-0 items-center gap-1 rounded-full border border-solid border-orange/30 bg-orange/10 px-2 py-0.5 text-xs font-semibold text-orange"
					>
						AMP
					</span>
				</div>

				<div class="flex flex-row items-center gap-3 text-sm text-secondary">
					<div class="flex items-center gap-1.5">
						<span
							class="size-2 rounded-full shrink-0"
							:class="statusDotClass"
						/>
						<span>{{ statusLabel }}</span>
					</div>
					<span class="text-surface-5">·</span>
					<span class="truncate">{{ baseUrl }}</span>
				</div>
			</div>
		</div>
	</div>
</template>

<script setup lang="ts">
import { ServerStackIcon } from '@modrinth/assets'
import { computed } from 'vue'
import { useRouter } from 'vue-router'

const props = defineProps<{
	serverId: string
	name: string
	baseUrl: string
	running?: boolean
}>()

const router = useRouter()

const statusDotClass = computed(() => {
	if (props.running === true) return 'bg-green'
	if (props.running === false) return 'bg-secondary'
	return 'bg-secondary'
})

const statusLabel = computed(() => {
	if (props.running === true) return 'Running'
	if (props.running === false) return 'Stopped'
	return 'Unknown'
})

function navigateToServer() {
	void router.push(`/hosting/manage/${props.serverId}`)
}
</script>
