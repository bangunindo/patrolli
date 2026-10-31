<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import { useHostsStore } from '@/stores/hosts.store'
import type { IHost } from '@@/shared/types/host.type'
import { computed, onMounted, reactive, ref, watch } from 'vue'
import ModalUnapplyTemplate from '@/components/configuration/host/ModalUnapplyTemplate.vue'

const props = withDefaults(defineProps<{ hosts?: IHost[] }>(), {
	hosts: () => []
})

const store = useHostsStore()

const form = reactive({
	template_id: []
})
const showUnapplyModal = ref(false)
const unlinkedTemplatesToConfirm = ref<Array<{ template_id: string; name: string }>>([])
const loading = ref(false)

const clusterURLs = computed(() => {
	const urls = props.hosts
		.map((host) => host.cluster_url)
		.filter((url): url is string => Boolean(url))
	return urls.length > 0 ? [...new Set(urls)] : undefined
})

const onConfirmUnapply = async (payload) => {
	try {
		loading.value = true
		await store.unlinkHostsFromTemplates(hostIds.value, payload, clusterURLs.value)
	} catch (error) {
		console.log(error)
	} finally {
		store.fetchHosts()
		showUnapplyModal.value = false
		loading.value = false
	}
}

const onLinkTemplates = async () => {
	try {
		loading.value = true

		await store.linkHostsToTemplates(hostIds.value, form, clusterURLs.value)
	} catch (error) {
		console.log(error)
	} finally {
		store.fetchHosts()
		loading.value = false
	}
}

const setUnlinkTemplates = () => {
	const temp = store.selection.templates.filter((template) => {
		return (
			template.template_id.includeTexts(form.template_id) &&
			template.template_id.includeTexts(hostTemplates.value ?? [])
		)
	})

	unlinkedTemplatesToConfirm.value = temp
}

const hostIds = computed(() => {
	return props.hosts.flatMap((host) => host.host_id)
})

const hostTemplates = computed(() => {
	let temp: any = []

	props.hosts.forEach((host) => {
		if (host.templates) {
			const templates = host?.templates as any[]
			const template_ids = templates.flatMap((template) => template.template_id)
			temp = [...temp, ...template_ids].noDuplicate()
		}
	})

	return temp
})

watch(
	() => form.template_id,
	() => {
		setUnlinkTemplates()
	}
)

watch(
	() => props.hosts,
	() => {
		store.fetchTemplates(clusterURLs.value)
		setUnlinkTemplates()
	},
	{ immediate: true }
)

onMounted(() => {
	store.fetchTemplates(clusterURLs.value)
})
</script>

<template>
	<div class="flex gap-4 items-center pb-3">
		<div class="flex gap-2 items-center">
			<label class="font-semibold">Select Templates</label>
			<AppInputSelect
				v-model="form.template_id"
				variant="outline"
				placeholder="Select Templates"
				:options="store.selection.templates"
				search
				multiple
				class="w-[300px]"
			/>
		</div>

		<div class="flex gap-2">
			<UiButton
				variant="outline"
				type="button"
				class="text-primary"
				:disabled="form.template_id.length == 0 || loading"
				@click="onLinkTemplates"
			>
				<UiIcon v-if="loading" name="loading" class="w-4 h-4 mr-2 animate-spin" />
				Link Template
			</UiButton>
			<UiButton
				variant="outline"
				type="button"
				class="text-amber-500 hover:text-amber-600"
				:disabled="unlinkedTemplatesToConfirm.length == 0"
				@click="showUnapplyModal = true"
			>
				Unlink Template
			</UiButton>
		</div>
	</div>

	<ModalUnapplyTemplate
		:show-modal="showUnapplyModal"
		:unlinked-templates="unlinkedTemplatesToConfirm"
		@confirm="onConfirmUnapply"
		@dismiss="showUnapplyModal = false"
	/>
</template>
