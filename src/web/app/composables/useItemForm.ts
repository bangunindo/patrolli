// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { storeToRefs } from 'pinia'
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'

import appHelper from '@/app/helpers/app.helper'
import { getItemCategory } from '@/app/helpers/item.helper'
import { useItemsStore } from '@/stores/items.store'
import {
	CreateHostItemBody,
	CreateTemplateItemBody,
	UpdateAppliedTemplateItemBody,
	UpdateHostItemBody,
	UpdateTemplateItemBody
} from '@@/shared/generated/items.type'
import { FormMode } from '@@/shared/types/app.type'
import { ItemCategory, ItemType } from '@@/shared/types/item.type'
import { itemHints, appliedTemplateItemHints } from '@/app/services/forms/ui-hints/item.hints'

export const serializeDummyRow = (row: any) => {
	if (!row || typeof row !== 'object') return row
	const copy = { ...row }

	if (copy.observed_time === '' || copy.observed_time === undefined) {
		copy.observed_time = null
	} else if (typeof copy.observed_time === 'string') {
		const trimmed = copy.observed_time.trim()
		if (!trimmed) {
			copy.observed_time = null
		} else if (/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) {
			copy.observed_time = `${trimmed}T00:00:00Z`
		} else if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(trimmed)) {
			copy.observed_time = trimmed
		} else {
			try {
				const d = new Date(trimmed)
				copy.observed_time = isNaN(d.getTime())
					? null
					: d.toISOString().replace('.000Z', 'Z')
			} catch {
				copy.observed_time = null
			}
		}
	}

	if (copy.severity === '' || copy.severity === undefined) {
		copy.severity = null
	}

	if (copy.source_ip === '' || copy.source_ip === undefined) {
		copy.source_ip = null
	}

	if (copy.source_port === '' || copy.source_port === undefined || copy.source_port === null) {
		copy.source_port = null
	} else {
		const portNum = Number(copy.source_port)
		copy.source_port = isNaN(portNum) ? null : portNum
	}

	if (copy.body && typeof copy.body === 'object') {
		if (copy.body.SmolStr !== undefined) {
			const trimmed = String(copy.body.SmolStr).trim()
			copy.body = trimmed ? { SmolStr: trimmed } : null
		} else if (copy.body.String !== undefined) {
			const trimmed = String(copy.body.String).trim()
			copy.body = trimmed ? { String: trimmed } : null
		} else {
			copy.body = null
		}
	} else if (typeof copy.body === 'string') {
		const trimmed = copy.body.trim()
		copy.body = trimmed ? { String: trimmed } : null
	} else {
		copy.body = null
	}

	if (Array.isArray(copy.attrs)) {
		copy.attrs = copy.attrs
			.filter((tuple: any) => {
				if (!Array.isArray(tuple) || tuple.length < 2) return false
				const key =
					tuple[0]?.String ??
					tuple[0]?.SmolStr ??
					(typeof tuple[0] === 'string' ? tuple[0] : '')
				return Boolean(key && String(key).trim())
			})
			.map((tuple: any) => {
				const rawKey =
					tuple[0]?.String ??
					tuple[0]?.SmolStr ??
					(typeof tuple[0] === 'string' ? tuple[0] : '')
				const keyObj = { String: String(rawKey).trim() }
				const valObj = tuple[1]

				let finalVal: any = 'Null'
				if (valObj === 'Null' || valObj === null || valObj === undefined) {
					finalVal = 'Null'
				} else if (valObj?.Bool !== undefined) {
					finalVal = { Bool: Boolean(valObj.Bool) }
				} else if (valObj?.Int64 !== undefined) {
					const num = Number(valObj.Int64)
					finalVal = { Int64: isNaN(num) ? 0 : Math.trunc(num) }
				} else if (valObj?.Double !== undefined) {
					const num = Number(valObj.Double)
					finalVal = { Double: isNaN(num) ? 0.0 : num }
				} else if (valObj?.String !== undefined) {
					const inner = valObj.String
					const str =
						typeof inner === 'object'
							? (inner?.String ?? inner?.SmolStr ?? '')
							: String(inner)
					finalVal = { String: { String: str } }
				} else if (typeof valObj === 'boolean') {
					finalVal = { Bool: valObj }
				} else if (typeof valObj === 'number') {
					finalVal = Number.isInteger(valObj) ? { Int64: valObj } : { Double: valObj }
				} else if (typeof valObj === 'string') {
					finalVal = { String: { String: valObj } }
				}

				return [keyObj, finalVal]
			})
	} else if (copy.attrs && typeof copy.attrs === 'object') {
		copy.attrs = Object.entries(copy.attrs)
			.filter(([k]) => Boolean(k && k.trim()))
			.map(([k, v]) => {
				const keyObj = { String: k.trim() }
				let finalVal: any = 'Null'
				if (typeof v === 'boolean') {
					finalVal = { Bool: v }
				} else if (typeof v === 'number') {
					finalVal = Number.isInteger(v) ? { Int64: v } : { Double: v }
				} else if (v === null || v === 'Null') {
					finalVal = 'Null'
				} else {
					finalVal = { String: { String: String(v ?? '') } }
				}
				return [keyObj, finalVal]
			})
	} else {
		copy.attrs = []
	}

	return copy
}

export const deserializeDummyRow = (row: any) => {
	if (!row || typeof row !== 'object') return row
	const copy = { ...row }

	if (
		copy.observed_time &&
		typeof copy.observed_time === 'string' &&
		copy.observed_time.includes('T')
	) {
		copy.observed_time = copy.observed_time.split('T')[0]
	}

	if (copy.body && typeof copy.body === 'object') {
		if (copy.body.SmolStr !== undefined) {
			const str = String(copy.body.SmolStr)
			copy.body = str ? { SmolStr: str } : null
		} else if (copy.body.String !== undefined) {
			const str = String(copy.body.String)
			copy.body = str ? { String: str } : null
		} else {
			copy.body = null
		}
	} else if (typeof copy.body === 'string') {
		const trimmed = copy.body.trim()
		copy.body = trimmed ? { String: trimmed } : null
	} else {
		copy.body = null
	}

	if (Array.isArray(copy.attrs)) {
		copy.attrs = structuredClone(copy.attrs)
	} else if (copy.attrs && typeof copy.attrs === 'object') {
		copy.attrs = Object.entries(copy.attrs)
			.filter(([k]) => Boolean(k && k.trim()))
			.map(([k, v]) => [{ String: k.trim() }, { String: { String: String(v ?? '') } }])
	} else {
		copy.attrs = []
	}

	return copy
}

export function useItemForm(options: {
	isTemplateContext?: boolean
	emits: (event: 'after:submit') => void
}) {
	const route = useRoute()
	const itemsStore = useItemsStore()
	const { itemsFormMode, currentItem, loading, selection } = storeToRefs(itemsStore)

	const selectedCluster = ref<string>(
		currentItem.value?.cluster_url ||
			appHelper.getClusterValue(route?.query?.cluster as string) ||
			appHelper.getDefaultClusterValue()
	)

	const getEffectiveClusterUrls = (): string[] | undefined => {
		const url =
			selectedCluster.value ||
			currentItem.value?.cluster_url ||
			appHelper.getClusterValue(route?.query?.cluster as string)
		return url ? [url] : undefined
	}

	let isInitializing = false

	const form = reactive({
		adm_status: 1,
		name: '',
		host_id: '',
		template_id: '',
		flags: 0,
		schedule: '',
		timeout_ms: undefined as number | string | null | undefined,
		labels: '{}',
		endpoint_id: undefined as string | null | undefined,
		listener_id: undefined as string | null | undefined,
		config: {
			type: ItemType.SNMP || 'snmp',
			config: undefined
		} as any
	})

	const category = computed<ItemCategory>(() =>
		getItemCategory(currentItem.value, itemsFormMode.value, options.isTemplateContext)
	)

	const isFieldsRequired = computed(
		() =>
			category.value === ItemCategory.HOST_ONLY ||
			category.value === ItemCategory.TEMPLATE_ROOT
	)

	const canHaveReceiver = computed(
		() =>
			category.value === ItemCategory.HOST_ONLY ||
			category.value === ItemCategory.APPLIED_TEMPLATE
	)

	const isConfigTypeDisabled = computed(
		() => itemsFormMode.value === FormMode.UPDATE && category.value !== ItemCategory.HOST_ONLY
	)

	const categoryLabelMap: Record<ItemCategory, string> = {
		[ItemCategory.HOST_ONLY]: 'Host Only',
		[ItemCategory.APPLIED_TEMPLATE]: 'Applied Template',
		[ItemCategory.TEMPLATE_ROOT]: 'Root Template Item',
		[ItemCategory.TEMPLATE_INHERITED]: 'Inherited Template Item'
	}

	const categoryLabel = computed(() => categoryLabelMap[category.value] || '')

	const namePlaceholder = computed(() =>
		!isFieldsRequired.value && currentItem.value?.resolved_name
			? `Inherited: ${currentItem.value.resolved_name}`
			: 'Name of item'
	)

	const schedulePlaceholder = computed(() =>
		!isFieldsRequired.value &&
		currentItem.value?.resolved_schedule !== undefined &&
		currentItem.value?.resolved_schedule !== null &&
		currentItem.value?.resolved_schedule !== ''
			? `Inherited: ${currentItem.value.resolved_schedule}`
			: '30'
	)

	const timeoutPlaceholder = computed(() =>
		!isFieldsRequired.value &&
		currentItem.value?.resolved_timeout_ms !== undefined &&
		currentItem.value?.resolved_timeout_ms !== null
			? `Inherited: ${currentItem.value.resolved_timeout_ms}`
			: '1000'
	)

	const unwrapConfigData = (raw: any): any => {
		if (!raw || typeof raw !== 'object') return null

		let inner = raw
		if ('config' in raw && raw.config && typeof raw.config === 'object') {
			inner = raw.config
		}

		if (typeof inner === 'object' && inner !== null) {
			const keys = Object.keys(inner).filter((k) => k !== 'type')
			if (keys.length > 0) return inner
		}

		return null
	}

	const extractConfig = (item: any) => {
		const type = item.type ?? item.config?.type ?? ItemType.SNMP ?? 'snmp'
		let configData = unwrapConfigData(item.config)

		if (!configData) {
			configData = unwrapConfigData(item.resolved_config)
		}

		let safeConfig = {}
		try {
			safeConfig = configData ? JSON.parse(JSON.stringify(configData)) : {}
		} catch {
			safeConfig = {}
		}

		if (type === 'dummy' && safeConfig && typeof safeConfig === 'object') {
			const dummyCfg = safeConfig as any
			if (dummyCfg.payload?.row) {
				dummyCfg.payload.row = deserializeDummyRow(dummyCfg.payload.row)
			}
		}

		return {
			type,
			config: safeConfig
		}
	}

	const populateForm = (item: any) => {
		if (!item) return

		const isRequired = isFieldsRequired.value

		if (isRequired) {
			form.name = item.name || item.resolved_name || ''
			form.schedule = item.schedule || item.resolved_schedule || ''
			form.timeout_ms = item.timeout_ms ?? item.resolved_timeout_ms ?? ('' as any)
		} else {
			form.name = item.name ?? ''
			form.schedule = item.schedule ?? null
			form.timeout_ms = item.timeout_ms ?? null
		}

		form.host_id = item.host_id ?? ''
		form.template_id = item.template_id ?? ''
		form.adm_status = item.adm_status ?? item.resolved_adm_status ?? 1
		form.flags = item.flags ?? 0

		const extractedConfig = extractConfig(item)
		form.config = {
			type: extractedConfig.type,
			config: extractedConfig.config
		}

		form.endpoint_id = item.endpoint_id ?? undefined
		form.listener_id = item.listener_id ?? undefined
		form.labels =
			typeof item.labels === 'string' ? item.labels : JSON.stringify(item.labels ?? {})
	}

	const setDefaultFormEdit = async () => {
		isInitializing = true
		const urls = currentItem.value?.cluster_url ? [currentItem.value?.cluster_url] : undefined

		if (currentItem.value) {
			populateForm(currentItem.value)
		}

		if (currentItem.value?.item_id) {
			await itemsStore.fetchItemById(currentItem.value.item_id, urls)
			if (currentItem.value) {
				populateForm(currentItem.value)
			}
		}

		await Promise.all([itemsStore.fetchHosts(urls), itemsStore.fetchTemplates(urls)])

		await nextTick()
		isInitializing = false
	}

	const closeFormModal = () => {
		loading.value.form = false
		itemsStore.toggleItemsModalForm(false)
		itemsStore.toggleItemsTemplateModalForm(false)
	}

	const parseLabels = (labels: string | object) => {
		try {
			return typeof labels === 'string' ? JSON.parse(labels || '{}') : labels
		} catch {
			return {}
		}
	}

	const cleanConfigNumericFields = (cfg: any, configType: string) => {
		if (!cfg) return
		const numFields = [
			'count',
			'speed',
			'read_buffer_size',
			'read_timeout_ms',
			'max_packet_size',
			'retries',
			'max_bulk_repetitions'
		]
		numFields.forEach((key) => {
			if ([undefined, null, ''].includes(cfg[key] as any)) {
				cfg[key] = undefined
			} else {
				cfg[key] = Number(cfg[key])
			}
		})

		if (configType === 'snmp' && [undefined, null].includes(cfg.oid)) {
			cfg.oid = ''
		}
	}

	const transformFormData = (rawData: any) => {
		const data = JSON.parse(JSON.stringify(rawData))

		if (data.schedule === '' || data.schedule === undefined || data.schedule === null) {
			if (itemsFormMode.value === FormMode.ADD) {
				data.schedule = '30s'
			} else if (!isFieldsRequired.value) {
				data.schedule = null
			}
		}
		if (data.timeout_ms === '' || data.timeout_ms === undefined || data.timeout_ms === null) {
			if (itemsFormMode.value === FormMode.ADD) {
				data.timeout_ms = 1000
			} else if (!isFieldsRequired.value) {
				data.timeout_ms = null
			}
		} else {
			const n = Number(data.timeout_ms)
			data.timeout_ms = isNaN(n) ? (isFieldsRequired.value ? 1000 : null) : n
		}

		if (data.config?.type === 'dummy') {
			if (!data.config.config || typeof data.config.config !== 'object') {
				data.config.config = {}
			}
			const cfg = data.config.config

			// Remove leftover fields from other collector modules
			delete cfg.oid
			delete cfg.oids
			delete cfg.accept_protobuf
			delete cfg.stream
			delete cfg.match_strategy

			// Clean count
			if (cfg.count === '' || cfg.count === null || cfg.count === undefined) {
				delete cfg.count
			} else {
				const n = Number(cfg.count)
				cfg.count = isNaN(n) ? undefined : n
			}

			// Ensure payload and row exist
			if (!cfg.payload || typeof cfg.payload !== 'object') {
				cfg.payload = {}
			}
			if (!cfg.payload.row || typeof cfg.payload.row !== 'object') {
				cfg.payload.row = {
					observed_time: null,
					severity: null,
					source_ip: null,
					source_port: null,
					body: null,
					attrs: []
				}
			} else {
				cfg.payload.row = serializeDummyRow(cfg.payload.row)
			}
		} else if (data.config?.type === 'snmp') {
			if (data.config.config) {
				delete data.config.config.payload
				delete data.config.config.oids
				delete data.config.config.accept_protobuf
				delete data.config.config.stream
				if (data.config.config.oid === undefined || data.config.config.oid === null) {
					data.config.config.oid = ''
				}
			}
		}
		return data
	}

	const buildPayload = (sourceData?: any) => {
		const rawSrc = sourceData || form
		// Always apply transformFormData to ensure modules (like dummy) have clean & serialized config
		const src = transformFormData(rawSrc)
		const nameTrimmed = (src.name || '').trim()
		const parsedLabels = parseLabels(src.labels)

		if (src.config?.config) {
			cleanConfigNumericFields(src.config.config, src.config.type)
			if (src.config.type === 'dummy' && src.config.config.payload?.row) {
				src.config.config.payload.row = serializeDummyRow(src.config.config.payload.row)
			}
		}

		const payload: any = {
			adm_status: src.adm_status ?? 1,
			flags: parseInt(`${src.flags}`) || 0,
			labels: parsedLabels
		}

		if (itemsFormMode.value === FormMode.ADD) {
			payload.name = nameTrimmed
			payload.schedule = src.schedule
			payload.timeout_ms = Number(src.timeout_ms)
			payload.config = src.config
			if (!options.isTemplateContext) {
				payload.host_id = form.host_id
			} else {
				payload.template_id = (route?.params?.template_id as string) || form.template_id
			}
		} else {
			if (isFieldsRequired.value) {
				payload.name = nameTrimmed
				payload.schedule = src.schedule
				payload.timeout_ms = Number(src.timeout_ms)
				payload.config = src.config
			} else {
				payload.name = nameTrimmed ? nameTrimmed : null
				payload.schedule = src.schedule ? src.schedule : null
				payload.timeout_ms =
					src.timeout_ms !== null &&
					src.timeout_ms !== undefined &&
					(src.timeout_ms as any) !== ''
						? Number(src.timeout_ms)
						: null
				payload.config = src.config
			}
		}

		if (canHaveReceiver.value) {
			if (
				src.config?.type === ItemType.SNMP ||
				src.config?.type === ItemType.SNMP_WALK ||
				src.config?.type === ItemType.PROMETHEUS_SCRAPE
			) {
				payload.endpoint_id = form.endpoint_id || null
			} else if (src.config?.type === ItemType.LISTENER) {
				payload.listener_id = form.listener_id || null
			}
		}

		return payload
	}

	const getSchemaForCategory = () => {
		if (itemsFormMode.value === FormMode.ADD) {
			if (options.isTemplateContext || category.value === ItemCategory.TEMPLATE_ROOT) {
				return CreateTemplateItemBody
			}
			return CreateHostItemBody
		}

		switch (category.value) {
			case ItemCategory.HOST_ONLY:
				return UpdateHostItemBody
			case ItemCategory.APPLIED_TEMPLATE:
				return UpdateAppliedTemplateItemBody
			case ItemCategory.TEMPLATE_ROOT:
			case ItemCategory.TEMPLATE_INHERITED:
			default:
				return UpdateTemplateItemBody
		}
	}

	const activeSchema = computed(() => getSchemaForCategory())

	const activeHints = computed(() => {
		const base = { ...itemHints }
		if (category.value === ItemCategory.APPLIED_TEMPLATE) {
			const merged: Record<string, any> = { ...base }
			for (const [key, hint] of Object.entries(appliedTemplateItemHints)) {
				merged[key] = { ...(base[key] || {}), ...hint }
			}
			merged.name = {
				...(merged.name || {}),
				placeholder: namePlaceholder.value
			}
			merged.schedule = {
				...(merged.schedule || {}),
				placeholder: schedulePlaceholder.value
			}
			merged.timeout_ms = {
				...(merged.timeout_ms || {}),
				placeholder: timeoutPlaceholder.value
			}
			return merged
		}
		return base
	})

	const isEndpointRequiredForType = computed(() => {
		const t = form.config?.type
		return t === ItemType.SNMP || t === ItemType.SNMP_WALK || t === ItemType.PROMETHEUS_SCRAPE
	})

	const isListenerRequiredForType = computed(() => {
		return form.config?.type === ItemType.LISTENER
	})

	const saveOrUpdate = async (payload: any) => {
		const urls = getEffectiveClusterUrls()
		if (itemsFormMode.value === FormMode.ADD) {
			if (options.isTemplateContext || category.value === ItemCategory.TEMPLATE_ROOT) {
				const targetTemplateId = (route?.params?.template_id as string) || form.template_id
				return await itemsStore.createItemForTemplate(targetTemplateId, payload, urls)
			} else {
				return await itemsStore.createItemForHost(form.host_id, payload, urls)
			}
		} else {
			const item_id = currentItem.value?.item_id ?? ''
			if (category.value === ItemCategory.HOST_ONLY) {
				return await itemsStore.updateItemHost(item_id, payload, urls)
			} else if (category.value === ItemCategory.APPLIED_TEMPLATE) {
				return await itemsStore.updateItemApliedTemplate(item_id, payload, urls)
			} else {
				return await itemsStore.updateItemTemplate(item_id, payload, urls)
			}
		}
	}

	const handleFormSubmit = async (validData?: any) => {
		loading.value.form = true

		const payload = buildPayload(validData)
		if (itemsFormMode.value === FormMode.ADD && !options.isTemplateContext && !form.host_id) {
			console.error('[useItemForm] Form submission stopped: host_id is required')
			loading.value.form = false
			return
		}

		const schema = getSchemaForCategory()
		const parseResult = schema.safeParse(payload)

		if (!parseResult.success) {
			console.error(
				'[useItemForm] Form validation failed via Zod schema:',
				parseResult.error,
				'Payload was:',
				payload
			)
			loading.value.form = false
			return
		}

		try {
			const res = await saveOrUpdate(parseResult.data)
			if (res) {
				closeFormModal()
				options.emits('after:submit')
			} else {
				loading.value.form = false
			}
		} catch (err) {
			console.error('[useItemForm] API saveOrUpdate error:', err)
			loading.value.form = false
		}
	}

	onMounted(() => {
		const urls = getEffectiveClusterUrls()

		if (itemsFormMode.value === FormMode.UPDATE) {
			setDefaultFormEdit()
		} else {
			itemsStore.fetchHosts(urls)
			itemsStore.fetchTemplates(urls)
		}
	})

	watch(
		[() => currentItem.value, () => itemsFormMode.value],
		([newItem, newMode]) => {
			if (newItem?.cluster_url || newItem?.cluster_name) {
				selectedCluster.value = appHelper.getClusterValue(
					newItem.cluster_name || newItem.cluster_url
				)
			}
			if (newMode === FormMode.UPDATE && newItem) {
				populateForm(newItem)
			}
		},
		{ immediate: true, deep: true }
	)

	watch(selectedCluster, (newCluster) => {
		if (itemsFormMode.value === FormMode.ADD) {
			form.host_id = ''
			form.endpoint_id = undefined
			form.listener_id = undefined
			const urls = newCluster ? [newCluster] : undefined
			itemsStore.fetchHosts(urls)
			itemsStore.fetchTemplates(urls)
			if (selection.value.endpoints !== undefined) {
				itemsStore.fetchEndpoints(urls)
			}
			if (selection.value.listeners !== undefined) {
				itemsStore.fetchListeners(urls)
			}
		}
	})

	watch(
		() => form.config?.type,
		(val, oldval) => {
			if (isInitializing) return
			if (val === oldval) return

			form.endpoint_id = undefined
			form.listener_id = undefined

			if (val === 'dummy') {
				if (!form.config.config?.payload?.row) {
					form.config.config = {
						count: 1,
						payload: {
							row: {
								observed_time: null,
								severity: 'info',
								source_ip: null,
								source_port: null,
								body: null,
								attrs: []
							}
						}
					}
				}
			} else if (val === 'snmp') {
				form.config.config = { oid: '' }
			} else if (val === 'snmp_walk') {
				form.config.config = { oids: [] }
			} else if (val === 'prometheus_scrape') {
				form.config.config = { accept_protobuf: false, stream: false }
			} else if (val === 'listener') {
				form.config.config = {}
			}
		}
	)

	watch(
		() => form.host_id,
		() => {
			if (isInitializing) return
			form.endpoint_id = undefined
			form.listener_id = undefined
		}
	)

	return {
		form,
		category,
		categoryLabel,
		isFieldsRequired,
		canHaveReceiver,
		isConfigTypeDisabled,
		namePlaceholder,
		schedulePlaceholder,
		timeoutPlaceholder,
		itemsFormMode,
		currentItem,
		loading,
		selection,
		route,
		selectedCluster,
		getEffectiveClusterUrls,
		FormMode,
		ItemCategory,
		ItemType,
		activeSchema,
		activeHints,
		isEndpointRequiredForType,
		isListenerRequiredForType,
		transformFormData,
		handleFormSubmit,
		closeFormModal
	}
}
