<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatZodSchemaForMonaco } from '@/app/helpers/monaco.helper'
import { useMonaco } from '@/composables/useMonaco'
import { useValidation } from '@/composables/useValidation'
import { useAppStore } from '@/stores/app.store'
import { storeToRefs } from 'pinia'
import AppInputError from '@/components/app/input/AppInputError.vue'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import type * as Monaco from 'monaco-editor'

const { loadMonaco, registerJsonSchema, unregisterJsonSchema } = useMonaco()

const appStore = useAppStore()
const { isDarkMode } = storeToRefs(appStore)

defineOptions({
	name: 'app-input-json'
})

const props = withDefaults(
	defineProps<{
		modelValue: any
		schema?: any
		height?: string
		required?: boolean
		label?: string
		error?: string | string[]
		disabled?: boolean
		readonly?: boolean
	}>(),
	{
		schema: undefined,
		height: '350px',
		required: false,
		label: 'JSON',
		error: '',
		disabled: false,
		readonly: false
	}
)

const emit = defineEmits<{
	(e: 'update:modelValue', value: any): void
	(e: 'update:error', value: string): void
	(e: 'close'): void
}>()

const editorContainer = ref<HTMLDivElement | null>(null)
const jsonError = ref<string | null>(null)
const isLoading = ref(true)
const text = ref<string>('')

let editorInstance: Monaco.editor.IStandaloneCodeEditor | null = null
let monacoModule: typeof Monaco | null = null
let contentListener: Monaco.IDisposable | null = null
let isLocalChange = false
let isDestroyed = false

const instanceId = Math.random().toString(36).substring(2, 9)
const fileName = `payload-${instanceId}.json`
const schemaUri = `internal://schema-${instanceId}.json`
const modelUriString = `internal://${fileName}`

function applySchema(monaco: typeof Monaco) {
	if (!props.schema) return
	const pureSchema = formatZodSchemaForMonaco(props.schema)
	registerJsonSchema(monaco, {
		uri: schemaUri,
		fileMatch: [fileName],
		schema: pureSchema
	})
}

onMounted(async () => {
	if (!import.meta.client) return
	isDestroyed = false

	try {
		const monacoLoaded = await loadMonaco()
		monacoModule = monacoLoaded
		await nextTick()

		if (isDestroyed || !editorContainer.value) return

		applySchema(monacoLoaded)

		let initialValue = ''
		if (props.modelValue && typeof props.modelValue === 'object') {
			initialValue = JSON.stringify(props.modelValue, null, 2)
		} else if (typeof props.modelValue === 'string') {
			initialValue = props.modelValue
		}

		const modelUri = monacoLoaded.Uri.parse(modelUriString)
		const existingModel = monacoLoaded.editor.getModel(modelUri)
		if (existingModel) {
			existingModel.dispose()
		}

		if (isDestroyed || !editorContainer.value) return

		const model = monacoLoaded.editor.createModel(initialValue, 'json', modelUri)

		editorInstance = monacoLoaded.editor.create(editorContainer.value, {
			model: model,
			theme: isDarkMode.value ? 'vs-dark' : 'vs',
			automaticLayout: true,
			formatOnPaste: true,
			formatOnType: true,
			minimap: { enabled: false },
			fontSize: 13,
			fontFamily: "'IBM Plex Mono', monospace",
			scrollBeyondLastLine: false,
			readOnly: props.disabled || props.readonly,
			quickSuggestions: {
				other: true,
				strings: true,
				comments: false
			},
			suggestOnTriggerCharacters: true,
			acceptSuggestionOnEnter: 'on',
			tabCompletion: 'on'
		})

		contentListener = editorInstance.onDidChangeModelContent(() => {
			const currentText = editorInstance?.getValue() || ''
			text.value = currentText
			if (!currentText.trim()) {
				jsonError.value = null
				isLocalChange = true
				emit('update:modelValue', undefined)
				isLocalChange = false
				return
			}
			try {
				const parsed = JSON.parse(currentText)
				jsonError.value = null
				isLocalChange = true
				emit('update:modelValue', parsed)
				isLocalChange = false
			} catch (e: any) {
				jsonError.value = 'Format JSON tidak valid: ' + e.message
			}
		})

		text.value = editorInstance.getValue() || ''
	} catch (error) {
		if (!isDestroyed) {
			console.error('Gagal memuat Monaco Editor:', error)
			jsonError.value = 'Gagal memuat teks editor.'
		}
	} finally {
		if (!isDestroyed) {
			isLoading.value = false
		}
	}
})

watch(
	() => props.modelValue,
	(newVal) => {
		if (isLocalChange || !editorInstance) return

		const currentText = editorInstance.getValue()
		text.value = currentText
		try {
			const currentParsed = currentText ? JSON.parse(currentText) : null
			if (JSON.stringify(currentParsed) === JSON.stringify(newVal)) return
		} catch {
			// ignore dirty json parse error
		}

		const newText =
			newVal && typeof newVal === 'object' ? JSON.stringify(newVal, null, 2) : newVal || ''

		if (currentText === newText) return

		const model = editorInstance.getModel()
		if (model) {
			editorInstance.executeEdits('v-model-sync', [
				{
					range: model.getFullModelRange(),
					text: newText,
					forceMoveMarkers: true
				}
			])
			editorInstance.pushUndoStop()
		}
	},
	{ deep: true }
)

watch(
	() => props.schema,
	() => {
		if (monacoModule) {
			applySchema(monacoModule)
		}
	},
	{ deep: true }
)

watch(
	() => [props.disabled, props.readonly],
	([disabled, readonly]) => {
		if (editorInstance) {
			editorInstance.updateOptions({
				readOnly: Boolean(disabled || readonly)
			})
		}
	}
)

watch(isDarkMode, (newVal) => {
	if (editorInstance && monacoModule) {
		monacoModule.editor.setTheme(newVal ? 'vs-dark' : 'vs')
	}
})

onBeforeUnmount(() => {
	isDestroyed = true

	if (contentListener) {
		contentListener.dispose()
		contentListener = null
	}

	if (editorInstance) {
		editorInstance.dispose()
		editorInstance = null
	}

	if (monacoModule) {
		unregisterJsonSchema(monacoModule, schemaUri)
		const modelUri = monacoModule.Uri.parse(modelUriString)
		const currentModel = monacoModule.editor.getModel(modelUri)
		if (currentModel) {
			currentModel.dispose()
		}
	}
})

const rules = computed(() => {
	const r: string[] = []
	if (props.required) r.push('required')
	return r
})

const combinedError = computed<string[]>(() => {
	const errors: string[] = []
	if (jsonError.value) errors.push(jsonError.value)
	if (props.error) {
		if (Array.isArray(props.error)) {
			errors.push(...props.error)
		} else {
			errors.push(props.error)
		}
	}
	return errors
})

const {
	elRef,
	errorMessage,
	showMessages,
	resetFormSubmitState: _resetFormSubmitState,
	handleAlertClose
} = useValidation(props.label || 'JSON', text, rules, {
	error: combinedError,
	onErrorChange: (val) => emit('update:error', val),
	onClose: () => {
		jsonError.value = null
		emit('close')
	}
})
</script>

<template>
	<div ref="elRef" class="app-json-editor nokey nowheel nodrag nopan" @keydown.stop>
		<!-- Relative positioning on wrapper -->
		<div
			class="monaco-wrapper nokey nowheel nodrag nopan"
			:class="{ 'is-disabled': disabled || readonly }"
			:style="{ height: height }"
			@keydown.stop
		>
			<!-- Loading Overlay -->
			<div v-if="isLoading" class="monaco-loading">
				<div class="spinner"></div>
				<span>Memuat Editor...</span>
			</div>

			<!-- Monaco Editor Main Container -->
			<div
				ref="editorContainer"
				class="editor-instance nokey nowheel nodrag nopan"
				@keydown.stop
			></div>
		</div>

		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>
</template>

<style scoped>
.monaco-wrapper {
	position: relative;
	border: 1px solid #cbd5e1;
	border-radius: 6px;
	overflow: hidden;
	margin-bottom: 4px;
	width: 100%;
	font-family: 'IBM Plex Mono', monospace;
}
.dark .monaco-wrapper {
	border-color: #475569;
}
.monaco-wrapper.is-disabled {
	opacity: 0.75;
}

/* Style editor instance to fill wrapper height */
.editor-instance {
	width: 100%;
	height: 100%;
}

/* Loading Overlay Styles */
.monaco-loading {
	position: absolute;
	top: 0;
	left: 0;
	right: 0;
	bottom: 0;
	display: flex;
	flex-direction: column;
	align-items: center;
	justify-content: center;
	background-color: #ffffff;
	z-index: 5;
	gap: 12px;
	font-size: 14px;
	color: #64748b;
	font-family: 'IBM Plex Sans', sans-serif;
}
.dark .monaco-loading {
	background-color: #1e1e1e;
	color: #94a3b8;
}

/* Simple CSS Spinner */
.spinner {
	width: 24px;
	height: 24px;
	border: 3px solid #cbd5e1;
	border-top-color: #3b82f6;
	border-radius: 50%;
	animation: spin 0.8s linear infinite;
}
.dark .spinner {
	border-color: #475569;
	border-top-color: #3b82f6;
}

@keyframes spin {
	to {
		transform: rotate(360deg);
	}
}

.error-message-text {
	color: #ef4444;
	font-size: 12px;
	margin-top: 4px;
	font-weight: 500;
	font-family: 'IBM Plex Sans', sans-serif;
}
</style>
