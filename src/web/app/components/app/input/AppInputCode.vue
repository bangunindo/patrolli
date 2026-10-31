<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { registerOplLanguage } from '@/app/helpers/opl.language'
import { useMonaco } from '@/composables/useMonaco'
import { useValidation } from '@/composables/useValidation'
import { useAppStore } from '@/stores/app.store'
import { storeToRefs } from 'pinia'
import AppInputError from '@/components/app/input/AppInputError.vue'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import type * as Monaco from 'monaco-editor'

const { loadMonaco } = useMonaco()

const appStore = useAppStore()
const { isDarkMode } = storeToRefs(appStore)

defineOptions({
	name: 'app-input-code'
})

const props = withDefaults(
	defineProps<{
		modelValue?: string | null
		language?: string
		height?: string
		required?: boolean
		label?: string
		placeholder?: string
		error?: string | string[]
		disabled?: boolean
		readonly?: boolean
	}>(),
	{
		modelValue: '',
		language: 'opl',
		height: '280px',
		required: false,
		label: 'Code',
		placeholder: '',
		error: '',
		disabled: false,
		readonly: false
	}
)

const emit = defineEmits<{
	(e: 'update:modelValue', value: string): void
	(e: 'update:error', value: string): void
	(e: 'close'): void
}>()

const editorContainer = ref<HTMLDivElement | null>(null)
const isLoading = ref(true)
const text = ref<string>(props.modelValue || '')

let editorInstance: Monaco.editor.IStandaloneCodeEditor | null = null
let monacoModule: typeof Monaco | null = null
let contentListener: Monaco.IDisposable | null = null
let isLocalChange = false
let isDestroyed = false

const instanceId = Math.random().toString(36).substring(2, 9)
const modelUriString = `internal://code-${instanceId}.${props.language || 'txt'}`

onMounted(async () => {
	if (!import.meta.client) return
	isDestroyed = false

	try {
		const monacoLoaded = await loadMonaco()
		monacoModule = monacoLoaded

		if (props.language === 'opl') {
			registerOplLanguage(monacoLoaded)
		}

		await nextTick()

		if (isDestroyed || !editorContainer.value) return

		const modelUri = monacoLoaded.Uri.parse(modelUriString)
		const existingModel = monacoLoaded.editor.getModel(modelUri)
		if (existingModel) {
			existingModel.dispose()
		}

		if (isDestroyed || !editorContainer.value) return

		const initialValue = props.modelValue ?? ''
		const model = monacoLoaded.editor.createModel(
			initialValue,
			props.language || 'opl',
			modelUri
		)

		editorInstance = monacoLoaded.editor.create(editorContainer.value, {
			model: model,
			theme: isDarkMode.value ? 'vs-dark' : 'vs',
			automaticLayout: true,
			minimap: { enabled: false },
			fontSize: 13,
			fontFamily: "'IBM Plex Mono', monospace",
			scrollBeyondLastLine: false,
			lineNumbers: 'on',
			tabSize: 2,
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
			isLocalChange = true
			emit('update:modelValue', currentText)
			isLocalChange = false
			if (currentText && currentText.trim().length > 0) {
				resetFormSubmitState()
			}
		})

		text.value = editorInstance.getValue() || ''
	} catch (error) {
		if (!isDestroyed) {
			console.error('Gagal memuat Code Editor:', error)
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
		const newText = newVal ?? ''
		text.value = newText

		if (newText && newText.trim().length > 0) {
			resetFormSubmitState()
		}

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
	}
)

watch(
	() => props.language,
	(newLang) => {
		if (editorInstance && monacoModule && newLang) {
			if (newLang === 'opl') {
				registerOplLanguage(monacoModule)
			}
			const model = editorInstance.getModel()
			if (model) {
				monacoModule.editor.setModelLanguage(model, newLang)
			}
		}
	}
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
	if (props.error) {
		const raw = Array.isArray(props.error) ? props.error : [props.error]
		for (const err of raw) {
			if (
				text.value &&
				text.value.trim().length > 0 &&
				typeof err === 'string' &&
				err.toLowerCase().includes('required')
			) {
				continue
			}
			errors.push(err)
		}
	}
	return errors
})

const { elRef, errorMessage, showMessages, resetFormSubmitState, handleAlertClose } = useValidation(
	props.label || 'Code',
	text,
	rules,
	{
		error: combinedError,
		onErrorChange: (val) => emit('update:error', val),
		onClose: () => {
			emit('close')
		}
	}
)
</script>

<template>
	<div ref="elRef" class="app-code-editor nokey nowheel nodrag nopan" @keydown.stop>
		<label v-if="label" class="block text-xs font-semibold text-foreground mb-1.5">
			{{ label }}
			<span v-if="required" class="text-destructive ml-0.5">*</span>
		</label>

		<div
			class="monaco-wrapper nokey nowheel nodrag nopan"
			:class="{ 'is-disabled': disabled || readonly }"
			:style="{ height: height }"
			@keydown.stop
		>
			<!-- Loading Overlay -->
			<div v-if="isLoading" class="monaco-loading">
				<div class="spinner"></div>
				<span>Memuat Editor {{ language.toUpperCase() }}...</span>
			</div>

			<!-- Monaco Editor Main Container -->
			<div
				ref="editorContainer"
				class="editor-instance nokey nowheel nodrag nopan"
				@keydown.stop
			></div>
		</div>

		<p v-if="placeholder && !modelValue" class="text-[11px] text-muted-foreground mt-1">
			{{ placeholder }}
		</p>

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

.editor-instance {
	width: 100%;
	height: 100%;
}

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
</style>
