// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { z } from 'zod'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'

describe('auto-adapter', () => {
	const testSchema = z.object({
		name: z.string().min(1, 'Name is required'),
		retries: z.number().int().min(0).optional(),
		timeout: z.number().int().default(1000),
		description: z.string().optional(),
		override_val: z.string().nullable().optional()
	})

	describe('toRequest - Sanitasi & Coercion', () => {
		it('mengonversi string kosong ke undefined untuk field opsional dan angka', () => {
			const rawDraft = {
				name: 'Test Endpoint',
				retries: '',
				description: ''
			}

			const result = AutoFormAdapter.toRequest(testSchema, rawDraft)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.retries).toBeUndefined()
				expect(result.data.description).toBeUndefined()
			}
		})

		it('mengonversi string angka menjadi number (coercion)', () => {
			const rawDraft = {
				name: 'Test Endpoint',
				retries: '5',
				timeout: '5000'
			}

			const result = AutoFormAdapter.toRequest(testSchema, rawDraft)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.retries).toBe(5)
				expect(result.data.timeout).toBe(5000)
			}
		})

		it('mengonversi string kosong ke null jika emptyPolicy diatur ke null', () => {
			const rawDraft = {
				name: 'Test Item',
				override_val: ''
			}
			const hints = {
				override_val: { emptyPolicy: 'null' as const }
			}

			const result = AutoFormAdapter.toRequest(testSchema, rawDraft, hints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.override_val).toBeNull()
			}
		})

		it('mengembalikan dictionary issues saat validasi gagal', () => {
			const rawDraft = {
				name: '', // Field wajib
				retries: -1 // Min 0
			}

			const result = AutoFormAdapter.toRequest(testSchema, rawDraft)
			expect(result.success).toBe(false)
			if (!result.success) {
				expect(result.issues['name']).toBeDefined()
				expect(result.issues['retries']).toBeDefined()
			}
		})
	})

	describe('toDraft', () => {
		it('mengisi nilai default dari Zod schema saat draft awal kosong', () => {
			const draft = AutoFormAdapter.toDraft(testSchema)
			expect(draft.timeout).toBe(1000)
			expect(draft.name).toBeUndefined()
		})

		it('mempertahankan nilai entitas yang ada', () => {
			const entity = { name: 'Existing Endpoint', timeout: 3000 }
			const draft = AutoFormAdapter.toDraft(testSchema, entity)

			expect(draft.name).toBe('Existing Endpoint')
			expect(draft.timeout).toBe(3000)
		})
	})
})
