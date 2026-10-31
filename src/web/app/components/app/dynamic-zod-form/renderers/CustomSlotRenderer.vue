<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script lang="ts">
import { defineComponent, type PropType } from 'vue'

export default defineComponent({
	name: 'CustomSlotRenderer',
	props: {
		renderFn: {
			type: Function as PropType<(...args: any[]) => any>,
			required: true
		},
		slotProps: {
			type: Object as PropType<Record<string, any>>,
			default: () => ({})
		}
	},
	setup(props) {
		return () => (props.renderFn ? props.renderFn(props.slotProps) : null)
	}
})
</script>
