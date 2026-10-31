<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { ref, watch } from 'vue'

const props = defineProps<{
	showModal: boolean
	unlinkedTemplates: Array<{ template_id: string; name: string }>
}>()

const emit = defineEmits(['dismiss', 'confirm'])

const choices = ref<Record<string, boolean>>({})

watch(
	() => props.showModal,
	(isOpen) => {
		if (isOpen) {
			const newChoices: Record<string, boolean> = {}
			props.unlinkedTemplates.forEach((t) => {
				newChoices[t.template_id] = false // Default: Clear items
			})
			choices.value = newChoices
		}
	}
)

const onConfirm = () => {
	const result = props.unlinkedTemplates.map((t) => ({
		unlink_from_template_id: t.template_id,
		keep_items: choices.value[t.template_id]
	}))
	emit('confirm', result)
}

const onDismiss = () => {
	emit('dismiss')
}
</script>

<template>
	<UiDialog :open="showModal" @update:open="(val: boolean) => !val && onDismiss()">
		<UiDialogContent class="max-w-md p-6 flex flex-col gap-4">
			<UiDialogHeader>
				<UiDialogTitle class="text-lg font-semibold text-foreground">
					Unlink Templates Confirmation
				</UiDialogTitle>
			</UiDialogHeader>

			<div class="flex flex-col gap-4 py-2 max-h-[60vh] overflow-y-auto">
				<div
					v-for="template in unlinkedTemplates"
					:key="template.template_id"
					class="p-4 rounded-lg border border-border bg-muted/40 flex flex-col gap-3"
				>
					<div class="font-medium text-foreground">
						{{ template.name || template.template_id }}
					</div>

					<div class="flex gap-4">
						<label
							class="flex items-center gap-2 cursor-pointer text-sm text-foreground"
						>
							<input
								type="radio"
								:name="`keep_items_${template.template_id}`"
								:value="false"
								v-model="choices[template.template_id]"
								class="text-primary focus:ring-primary"
							/>
							<span>Unlink and Clear</span>
						</label>
						<label
							class="flex items-center gap-2 cursor-pointer text-sm text-foreground"
						>
							<input
								type="radio"
								:name="`keep_items_${template.template_id}`"
								:value="true"
								v-model="choices[template.template_id]"
								class="text-primary focus:ring-primary"
							/>
							<span>Unlink (Keep Items)</span>
						</label>
					</div>
				</div>
			</div>

			<UiDialogFooter class="flex flex-row gap-2 justify-end pt-3 border-t">
				<UiButton variant="outline" size="sm" @click="onDismiss"> Cancel </UiButton>
				<UiButton size="sm" @click="onConfirm"> Update </UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>
