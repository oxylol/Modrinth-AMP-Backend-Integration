<template>
	<NewModal ref="modal" header="Add external server" @hide="onHide">
		<div class="flex flex-col gap-4 md:w-[520px]">
			<div class="flex flex-col gap-2">
				<label for="amp-url-input">
					<span class="text-sm font-semibold text-contrast">Panel URL</span>
				</label>
				<StyledInput
					id="amp-url-input"
					v-model="form.baseUrl"
					placeholder="http://192.168.1.10:8080"
					wrapper-class="w-full"
					:disabled="isBusy"
				/>
			</div>

			<div class="flex flex-col gap-2">
				<label for="amp-username-input">
					<span class="text-sm font-semibold text-contrast">Username</span>
				</label>
				<StyledInput
					id="amp-username-input"
					v-model="form.username"
					placeholder="admin"
					wrapper-class="w-full"
					:disabled="isBusy"
				/>
			</div>

			<div class="flex flex-col gap-2">
				<label for="amp-password-input">
					<span class="text-sm font-semibold text-contrast">Password</span>
				</label>
				<StyledInput
					id="amp-password-input"
					v-model="form.password"
					type="password"
					placeholder="••••••••"
					wrapper-class="w-full"
					:disabled="isBusy"
				/>
			</div>

			<div class="flex flex-col gap-2">
				<label for="amp-name-input">
					<span class="text-sm font-semibold text-contrast">Friendly name</span>
				</label>
				<StyledInput
					id="amp-name-input"
					v-model="form.friendlyName"
					placeholder="My Home Server"
					wrapper-class="w-full"
					:disabled="isBusy"
				/>
			</div>

			<Transition
				enter-active-class="transition-all duration-200 ease-out"
				enter-from-class="opacity-0 max-h-0"
				enter-to-class="opacity-100 max-h-20"
				leave-active-class="transition-all duration-150 ease-in"
				leave-from-class="opacity-100 max-h-20"
				leave-to-class="opacity-0 max-h-0"
			>
				<div v-if="testResult" class="overflow-hidden">
					<div
						class="flex items-center gap-2 rounded-lg px-3 py-2 text-sm"
						:class="
							testResult.ok
								? 'bg-green/10 text-green border border-solid border-green/30'
								: 'bg-red/10 text-red border border-solid border-red/30'
						"
					>
						<CheckCircleIcon v-if="testResult.ok" class="size-4 shrink-0" />
						<XCircleIcon v-else class="size-4 shrink-0" />
						<span>{{ testResult.message }}</span>
					</div>
				</div>
			</Transition>
		</div>

		<template #actions>
			<div class="flex w-full flex-row gap-2 justify-end">
				<ButtonStyled type="outlined">
					<button class="!border-[1px] !border-surface-4" :disabled="isBusy" @click="hide">
						<XIcon />
						Cancel
					</button>
				</ButtonStyled>
				<ButtonStyled type="outlined">
					<button :disabled="!canTest || isBusy" @click="testConnection">
						<LoaderCircleIcon v-if="isTesting" class="animate-spin" />
						<PlugIcon v-else />
						{{ isTesting ? 'Testing…' : 'Test connection' }}
					</button>
				</ButtonStyled>
				<ButtonStyled color="brand">
					<button :disabled="!canAdd || isBusy" @click="addServer">
						<LoaderCircleIcon v-if="isAdding" class="animate-spin" />
						<PlusIcon v-else />
						{{ isAdding ? 'Adding…' : 'Add server' }}
					</button>
				</ButtonStyled>
			</div>
		</template>
	</NewModal>
</template>

<script setup lang="ts">
import { CheckCircleIcon, LoaderCircleIcon, PlugIcon, PlusIcon, XCircleIcon, XIcon } from '@modrinth/assets'
import { computed, ref } from 'vue'

import type { AmpTestResult } from '#ui/providers/amp-backend'
import { injectAmpBackend } from '#ui/providers/amp-backend'
import ButtonStyled from '../base/ButtonStyled.vue'
import StyledInput from '../base/StyledInput.vue'
import NewModal from '../modal/NewModal.vue'

const emit = defineEmits<{
	added: []
}>()

const modal = ref<InstanceType<typeof NewModal> | null>(null)
const ampBackend = injectAmpBackend(null)

const form = ref({
	baseUrl: '',
	username: '',
	password: '',
	friendlyName: '',
})

const testResult = ref<AmpTestResult | null>(null)
const testPassed = ref(false)
const isTesting = ref(false)
const isAdding = ref(false)

const isBusy = computed(() => isTesting.value || isAdding.value)

const canTest = computed(
	() => !!form.value.baseUrl.trim() && !!form.value.username.trim() && !!form.value.password.trim(),
)

const canAdd = computed(() => testPassed.value && !!form.value.friendlyName.trim())

function show() {
	modal.value?.show()
}

function hide() {
	modal.value?.hide()
}

function onHide() {
	form.value = { baseUrl: '', username: '', password: '', friendlyName: '' }
	testResult.value = null
	testPassed.value = false
	isTesting.value = false
	isAdding.value = false
}

async function testConnection() {
	if (!ampBackend || !canTest.value) return

	isTesting.value = true
	testResult.value = null
	testPassed.value = false

	try {
		const result = await ampBackend.testConnection({
			baseUrl: form.value.baseUrl.trim(),
			username: form.value.username.trim(),
			password: form.value.password,
			insecureTls: false,
		})
		testResult.value = result
		testPassed.value = result.ok
	} catch (error) {
		testResult.value = {
			ok: false,
			message: extractErrorMessage(error) || 'Connection failed',
			instanceCount: 0,
		}
		testPassed.value = false
	} finally {
		isTesting.value = false
	}
}

async function addServer() {
	if (!ampBackend || !canAdd.value) return

	isAdding.value = true

	try {
		await ampBackend.addConnection({
			baseUrl: form.value.baseUrl.trim(),
			username: form.value.username.trim(),
			password: form.value.password,
			friendlyName: form.value.friendlyName.trim(),
			insecureTls: false,
		})
		emit('added')
		hide()
	} catch (error) {
		testResult.value = {
			ok: false,
			message: extractErrorMessage(error) || 'Failed to add server',
			instanceCount: 0,
		}
	} finally {
		isAdding.value = false
	}
}

function extractErrorMessage(error: unknown): string {
	if (error instanceof Error) return error.message
	if (typeof error === 'string') return error
	if (error && typeof error === 'object' && 'message' in error) return String((error as Record<string, unknown>).message)
	return String(error)
}

defineExpose({ show, hide })
</script>
