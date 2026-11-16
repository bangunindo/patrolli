<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, provide } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import { isFieldVisible, resolveHintForPath } from '@/app/services/forms/path-helper'
import { RESOLVED_HINT_KEY, useDynamicFormContext } from './useDynamicField'

import TextControl from './renderers/TextControl.vue'
import NumberControl from './renderers/NumberControl.vue'
import SwitchControl from './renderers/SwitchControl.vue'
import SelectControl from './renderers/SelectControl.vue'
import PasswordControl from './renderers/PasswordControl.vue'
import LabelControl from './renderers/LabelControl.vue'
import ObjectControl from './renderers/ObjectControl.vue'
import ArrayControl from './renderers/ArrayControl.vue'
import RecordControl from './renderers/RecordControl.vue'
import UnionControl from './renderers/UnionControl.vue'
import AttributeFilterControl from './renderers/AttributeFilterControl.vue'
import LogAttrsControl from './renderers/LogAttrsControl.vue'
import CodeControl from './renderers/CodeControl.vue'
import JsonControl from './renderers/JsonControl.vue'

const props = defineProps<{
	descriptor: FieldDescriptor
	path?: string
}>()

const ctx = useDynamicFormContext()

const resolvedPath = computed(() => props.path || props.descriptor.pathKey)

const resolvedHint = computed(() => {
	const r = resolveHintForPath(resolvedPath.value, ctx.formHints.value, ctx.rootValues.value)
	return {
		...(props.descriptor.hints || {}),
		...(r || {})
	}
})

provide(RESOLVED_HINT_KEY, resolvedHint)

const isVisible = computed(() => isFieldVisible(resolvedHint.value, ctx.rootValues.value || {}))

const widgetType = computed(() => resolvedHint.value.widget || props.descriptor.widget)

const controlComponent = computed(() => {
	const w = widgetType.value

	if (w === 'log-attrs' || w === 'attrs') return LogAttrsControl
	if (w === 'attribute-filter') return AttributeFilterControl
	if (w === 'code' || w === 'opl') return CodeControl
	if (w === 'record' || props.descriptor.zodType === 'record') return RecordControl
	if (w === 'password' || resolvedHint.value.sensitive || props.descriptor.sensitive) {
		return PasswordControl
	}
	if (w === 'label' || resolvedHint.value.emptyPolicy === 'empty-object') return LabelControl
	if (w === 'switch' || props.descriptor.zodType === 'boolean') {
		return SwitchControl
	}
	if (w === 'select' || props.descriptor.zodType === 'enum') {
		return SelectControl
	}
	if (w === 'discriminated-union') return UnionControl
	if (w === 'array-string' || w === 'array-object' || props.descriptor.zodType === 'array') {
		return ArrayControl
	}
	if (
		(w === 'custom' || w === 'object' || !w) &&
		(props.descriptor.zodType === 'object' ||
			props.descriptor.zodType === 'intersection' ||
			(props.descriptor.children && props.descriptor.children.length > 0))
	) {
		return ObjectControl
	}
	if (
		w === 'number' ||
		props.descriptor.zodType === 'number' ||
		props.descriptor.zodType === 'int'
	) {
		return NumberControl
	}
	if (w === 'json') return JsonControl

	return TextControl
})
</script>

<template>
	<component
		:is="controlComponent"
		v-if="isVisible"
		:descriptor="descriptor"
		:path="resolvedPath"
	/>
</template>
