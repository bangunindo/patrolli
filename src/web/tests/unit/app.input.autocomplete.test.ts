// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import AppInputAutocomplete from '@/components/app/input/AppInputAutocomplete.vue'

const mountAutocomplete = (props: Record<string, any> = {}, slots: Record<string, any> = {}) => {
	return mount(AppInputAutocomplete, {
		props,
		slots,
		global: {
			stubs: {
				Teleport: {
					template: '<div><slot /></div>'
				}
			}
		}
	})
}

describe('AppInputAutocomplete', () => {
	afterEach(() => {
		vi.useRealTimers()
	})

	it('filters primitive options client-side and selects an option in single mode', async () => {
		const wrapper = mountAutocomplete({
			label: 'Protocol',
			options: ['SNMP', 'ICMP', 'HTTP'],
			modelValue: ''
		})

		const input = wrapper.find('input')
		await input.trigger('focus')
		await input.setValue('sn')
		await nextTick()

		const items = wrapper.findAll('[role="option"]')
		expect(items).toHaveLength(1)
		expect(items[0].text()).toContain('SNMP')

		await items[0].trigger('click')
		await nextTick()

		expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['SNMP'])
		expect(wrapper.emitted('select')?.[0]).toEqual(['SNMP'])
	})

	it('resolves object options using custom labelKey and valueKey', async () => {
		const options = [
			{ id: 'host-1', title: 'Core Router' },
			{ id: 'host-2', title: 'Edge Switch' }
		]

		const wrapper = mountAutocomplete({
			options,
			labelKey: 'title',
			valueKey: 'id',
			modelValue: 'host-2'
		})

		const input = wrapper.find('input')
		expect((input.element as HTMLInputElement).value).toBe('Edge Switch')

		await input.trigger('focus')
		await input.setValue('Core')
		await nextTick()

		const items = wrapper.findAll('[role="option"]')
		expect(items).toHaveLength(1)
		expect(items[0].text()).toContain('Core Router')

		await items[0].trigger('click')
		await nextTick()

		expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['host-1'])
		expect(wrapper.emitted('select')?.[0]).toEqual([options[0]])
	})

	it('restores selected label on close in strict mode (allowCustomValue = false)', async () => {
		const wrapper = mountAutocomplete({
			options: [
				{ label: 'Alpha', value: 'alpha' },
				{ label: 'Beta', value: 'beta' }
			],
			modelValue: 'alpha',
			allowCustomValue: false
		})

		const input = wrapper.find('input')
		await input.trigger('focus')
		await input.setValue('Unmatched text')
		await nextTick()

		expect(wrapper.findAll('[role="option"]')).toHaveLength(0)
		expect(wrapper.text()).toContain('No results found.')

		await input.trigger('keydown', { key: 'Escape' })
		await nextTick()

		expect(wrapper.emitted('update:modelValue')).toBeUndefined()
		expect((input.element as HTMLInputElement).value).toBe('Alpha')
	})

	it('commits custom free-text value when allowCustomValue is enabled', async () => {
		const wrapper = mountAutocomplete({
			options: ['cpu.usage', 'mem.usage'],
			modelValue: '',
			allowCustomValue: true
		})

		const input = wrapper.find('input')
		await input.trigger('focus')
		await input.setValue('disk.iops')
		await nextTick()

		expect(wrapper.text()).toContain('Use "disk.iops"')

		await input.trigger('keydown', { key: 'Enter' })
		await nextTick()

		expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['disk.iops'])
		expect(wrapper.emitted('select')?.[0]).toEqual(['disk.iops'])
	})

	it('supports multiple selection with removable badges and Backspace removal', async () => {
		const wrapper = mountAutocomplete({
			options: [
				{ label: 'Linux', value: 'linux' },
				{ label: 'Network', value: 'network' },
				{ label: 'Database', value: 'db' }
			],
			multiple: true,
			modelValue: ['linux', 'network']
		})

		expect(wrapper.text()).toContain('Linux')
		expect(wrapper.text()).toContain('Network')

		const removeLinuxBtn = wrapper.find('button[aria-label="Remove Linux"]')
		expect(removeLinuxBtn.exists()).toBe(true)
		await removeLinuxBtn.trigger('click')
		await nextTick()

		expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([['network']])

		await wrapper.setProps({ modelValue: ['network', 'db'] })
		const input = wrapper.find('input')
		await input.trigger('keydown', { key: 'Backspace' })
		await nextTick()

		const updates = wrapper.emitted('update:modelValue') || []
		expect(updates[updates.length - 1]).toEqual([['network']])
	})

	it('skips client-side filtering and emits debounced search in remote mode', async () => {
		vi.useFakeTimers()

		const wrapper = mountAutocomplete({
			options: [{ label: 'Server A', value: 'srv-a' }],
			remote: true,
			debounceMs: 250,
			modelValue: ''
		})

		const input = wrapper.find('input')
		await input.trigger('focus')
		await input.setValue('xyz')
		await nextTick()

		// Remote mode renders all provided options without filtering them out
		const items = wrapper.findAll('[role="option"]')
		expect(items).toHaveLength(1)
		expect(items[0].text()).toContain('Server A')

		expect(wrapper.emitted('search')).toBeUndefined()
		vi.advanceTimersByTime(250)
		expect(wrapper.emitted('search')?.[0]).toEqual(['xyz'])

		await wrapper.setProps({ loading: true })
		await nextTick()
		expect(wrapper.find('[role="status"]').exists()).toBe(true)
		expect(wrapper.text()).toContain('Loading...')

		await wrapper.setProps({ loading: false })
		await nextTick()
		await wrapper.find('[role="option"]').trigger('click')
		await nextTick()
		// Does not reset remote search immediately upon selection
		expect(wrapper.emitted('search')).toHaveLength(1)

		// Resets remote search when dropdown is reopened
		await input.trigger('focus')
		await nextTick()
		expect(wrapper.emitted('search')?.[1]).toEqual([''])
	})

	it('handles clearable, disabled, and readonly states', async () => {
		const wrapper = mountAutocomplete({
			options: ['One', 'Two'],
			modelValue: 'One',
			clearable: true
		})

		const clearBtn = wrapper.find('button[aria-label="Clear selection"]')
		expect(clearBtn.exists()).toBe(true)
		await clearBtn.trigger('click')
		await nextTick()

		expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([''])
		expect(wrapper.emitted('clear')).toHaveLength(1)

		await wrapper.setProps({ modelValue: 'One', disabled: true })
		expect(wrapper.find('button[aria-label="Clear selection"]').exists()).toBe(false)
		expect(wrapper.find('input').attributes('disabled')).toBeDefined()

		await wrapper.setProps({ disabled: false, readonly: true })
		expect(wrapper.find('button[aria-label="Clear selection"]').exists()).toBe(false)
		expect(wrapper.find('input').attributes('readonly')).toBeDefined()
	})

	it('displays validation error via AppInputError', async () => {
		const wrapper = mountAutocomplete({
			label: 'Target Host',
			options: ['Host A'],
			modelValue: '',
			required: true,
			error: 'Target Host is required'
		})

		await nextTick()
		expect(wrapper.text()).toContain('Target Host is required')
	})

	it('renders subLabelKey in options, supports filtering by subLabel, and keeps chips compact', async () => {
		const options = [
			{ id: 'ep-1', name: 'SNMP Core', cluster_name: 'cluster-prod' },
			{ id: 'ep-2', name: 'HTTP Edge', cluster_name: 'cluster-staging' }
		]

		const wrapper = mountAutocomplete({
			options,
			labelKey: 'name',
			subLabelKey: 'cluster_name',
			valueKey: 'id',
			multiple: true,
			modelValue: ['ep-1']
		})

		const badge = wrapper.find('[aria-label="Remove SNMP Core"]')
		expect(badge.exists()).toBe(true)
		expect(wrapper.text()).toContain('SNMP Core')
		expect(wrapper.text()).not.toContain('cluster-prod')

		const input = wrapper.find('input')
		await input.trigger('focus')
		await input.setValue('staging')
		await nextTick()

		const items = wrapper.findAll('[role="option"]')
		expect(items).toHaveLength(1)
		expect(items[0].text()).toContain('HTTP Edge')
		expect(items[0].text()).toContain('cluster-staging')
	})

	it('limits visible tags with maxVisibleTags and shows +N more overflow badge', async () => {
		const options = Array.from({ length: 17 }, (_, i) => ({
			id: `ep-${i + 1}`,
			name: `Endpoint ${i + 1}`
		}))
		const allIds = options.map((o) => o.id)

		const wrapper = mountAutocomplete({
			options,
			labelKey: 'name',
			valueKey: 'id',
			multiple: true,
			modelValue: allIds
		})

		// Default maxVisibleTags is 2 -> 2 removable chips + 1 "+15 more" badge
		expect(wrapper.findAll('button[aria-label^="Remove "]')).toHaveLength(2)
		expect(wrapper.text()).toContain('Endpoint 1')
		expect(wrapper.text()).toContain('Endpoint 2')
		expect(wrapper.text().match(/\+15 more/g)).toHaveLength(1)

		// Custom maxVisibleTags = 4
		await wrapper.setProps({ maxVisibleTags: 4 })
		expect(wrapper.findAll('button[aria-label^="Remove "]')).toHaveLength(4)
		expect(wrapper.text()).toContain('+13 more')

		// maxVisibleTags = 0 disables truncation
		await wrapper.setProps({ maxVisibleTags: 0 })
		expect(wrapper.findAll('button[aria-label^="Remove "]')).toHaveLength(17)
		expect(wrapper.text()).not.toContain('more')
	})

	it('stops Escape propagation while open and lets Escape bubble when closed', async () => {
		const onParentKeydown = vi.fn()
		const ParentWrapper = {
			components: { AppInputAutocomplete },
			methods: { onParentKeydown },
			template: `
				<div @keydown="onParentKeydown">
					<AppInputAutocomplete :options="['Alpha', 'Beta']" model-value="Alpha" />
				</div>
			`
		}

		const wrapper = mount(ParentWrapper, {
			global: {
				stubs: {
					Teleport: {
						template: '<div><slot /></div>'
					}
				}
			}
		})

		const input = wrapper.find('input')
		await input.trigger('focus')
		await nextTick()
		expect(wrapper.findAll('[role="option"]')).toHaveLength(2)

		// First Escape closes only the combobox list and does not bubble to parent
		await input.trigger('keydown', { key: 'Escape' })
		await nextTick()
		expect(wrapper.findAll('[role="option"]')).toHaveLength(0)
		expect(onParentKeydown).not.toHaveBeenCalled()

		// Second Escape while closed bubbles to parent overlay
		await input.trigger('keydown', { key: 'Escape' })
		await nextTick()
		expect(onParentKeydown).toHaveBeenCalledTimes(1)
	})

	it('cancels uncommitted edits on Escape in allowCustomValue single mode', async () => {
		const wrapper = mountAutocomplete({
			options: [
				{ label: 'Alpha', value: 'alpha' },
				{ label: 'Beta', value: 'beta' }
			],
			modelValue: 'alpha',
			allowCustomValue: true
		})

		const input = wrapper.find('input')

		// Typing custom text then pressing Escape reverts without emitting update:modelValue
		await input.trigger('focus')
		await input.setValue('custom.value')
		await nextTick()
		await input.trigger('keydown', { key: 'Escape' })
		await nextTick()

		expect(wrapper.emitted('update:modelValue')).toBeUndefined()
		expect((input.element as HTMLInputElement).value).toBe('Alpha')

		// Clearing input then pressing Escape also reverts without clearing modelValue
		await input.trigger('focus')
		await input.setValue('')
		await nextTick()
		await input.trigger('keydown', { key: 'Escape' })
		await nextTick()

		expect(wrapper.emitted('update:modelValue')).toBeUndefined()
		expect((input.element as HTMLInputElement).value).toBe('Alpha')
	})
})
