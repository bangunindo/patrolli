// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { validateOplSyntax } from '@/app/helpers/opl.language'

describe('validateOplSyntax', () => {
	describe('Valid OPL scripts', () => {
		it('should validate standard logs filter and set pipeline', () => {
			const opl = 'logs | where severity_number >= 17 | set attributes["alert"] = true'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(true)
			expect(result.error).toBeUndefined()
		})

		it('should validate spans filter and error tagging', () => {
			const opl =
				'spans | where attributes["http.status_code"] >= 500 | set attributes["error"] = true'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(true)
		})

		it('should validate metrics with nested apply scope', () => {
			const opl =
				'metrics | where name == "http_requests_total" | apply data_points { set attributes["env"] = "prod" }'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(true)
		})

		it('should validate rename and remove operators', () => {
			const opl =
				'logs | rename attributes["old_key"] as "new_key" | remove attributes["temp_id"]'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(true)
		})

		it('should validate drop operator', () => {
			const opl = 'logs | where attributes["debug"] == true | drop'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(true)
		})

		it('should validate script with line and block comments', () => {
			const opl = `
				// Filter critical errors
				logs
				/* Multi-line comment explaining
				   the logic below */
				| where severity_number >= 17
				| set attributes["alert"] = true
			`
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(true)
		})

		it('should handle pipe characters inside string literals correctly', () => {
			const opl = 'logs | set attributes["delimited"] = "part1 | part2"'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(true)
		})
	})

	describe('Invalid OPL scripts', () => {
		it('should reject empty or whitespace-only code', () => {
			expect(validateOplSyntax('')).toEqual({
				isValid: false,
				error: 'Kode OPL tidak boleh kosong.'
			})
			expect(validateOplSyntax('   \n\t  ')).toEqual({
				isValid: false,
				error: 'Kode OPL tidak boleh kosong.'
			})
			expect(validateOplSyntax('// only comments\n/* here */')).toEqual({
				isValid: false,
				error: 'Kode OPL tidak boleh kosong.'
			})
		})

		it('should reject script not starting with a valid signal', () => {
			const opl = 'where severity_number >= 17 | drop'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain('harus diawali dengan target signal')
		})

		it('should reject unknown target signal', () => {
			const opl = 'unknown_signal | where severity >= 10'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Ditemukan 'unknown_signal'")
		})

		it('should reject signal without pipe before next instruction', () => {
			const opl = 'logs where severity >= 10'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain('harus dipisahkan dengan tanda pipe (|)')
		})

		it('should reject empty pipe segments', () => {
			const opl = 'logs | | where severity >= 10'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain('segmen pipe (|) kosong')
		})

		it('should reject trailing pipe segments', () => {
			const opl = 'logs | where severity >= 10 |'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain('segmen pipe (|) kosong')
		})

		it('should reject unbalanced parentheses', () => {
			const opl = 'logs | where (severity >= 10'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Tanda kurung '(' belum tertutup")
		})

		it('should reject unbalanced brackets', () => {
			const opl = 'logs | set attributes["key" = "value"'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Tanda kurung '[' belum tertutup")
		})

		it('should reject unclosed string quotes', () => {
			const opl = 'logs | set attributes["key"] = "unclosed string'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain('Tanda kutip string belum tertutup')
		})

		it('should reject unknown operators', () => {
			const opl = 'logs | calculate attributes["x"] = 1'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Operator 'calculate' pada segmen ke-2 tidak dikenali")
		})

		it('should reject where operator without expression', () => {
			const opl = 'logs | where'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Operator 'where' membutuhkan ekspresi kondisi")
		})

		it('should reject set operator without assignment', () => {
			const opl = 'logs | set attributes["key"]'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Operator 'set' harus memiliki tanda assignment '='")
		})

		it('should reject rename operator without as clause', () => {
			const opl = 'logs | rename attributes["old"] attributes["new"]'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Operator 'rename' harus menggunakan klausa 'as'")
		})

		it('should reject remove operator without target', () => {
			const opl = 'logs | remove'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Operator 'remove' membutuhkan target atribut")
		})

		it('should reject apply operator without scope or braces', () => {
			const opl = 'metrics | apply data_points'
			const result = validateOplSyntax(opl)
			expect(result.isValid).toBe(false)
			expect(result.error).toContain("Operator 'apply' harus memiliki blok kurung kurawal")
		})
	})
})
