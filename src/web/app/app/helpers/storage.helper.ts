// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

const storageHelper = {
	loadItem(key: string = ''): any {
		try {
			const storageData = localStorage.getItem(`btj_${key}`)
			if (storageData === null) {
				return null
			}
			return JSON.parse(storageData)
		} catch {
			throw new Error('Failed to load object from local storage')
		}
	},

	loadItemStr(key: string = ''): string | null {
		try {
			return localStorage.getItem(`btj_${key}`)
		} catch {
			throw new Error('Failed to load string from local storage')
		}
	},

	loadItemNoLable(key: string = ''): string | null {
		try {
			return localStorage.getItem(key)
		} catch {
			throw new Error('Failed to load string from local storage')
		}
	},

	saveItem(key: string = 'key', obj: any = {}): void {
		try {
			localStorage.setItem(`btj_${key}`, JSON.stringify(obj))
		} catch {
			throw new Error('Failed to save object to local storage')
		}
	},

	saveItemStr(key: string = 'key', str: string = ''): void {
		try {
			localStorage.setItem(`btj_${key}`, str)
		} catch {
			throw new Error('Failed to save string to local storage')
		}
	},

	removeItem(key: string = 'key'): void {
		try {
			localStorage.removeItem(`btj_${key}`)
		} catch {
			throw new Error('Failed to remove object from local storage')
		}
	},

	clearItems(): void {
		localStorage.clear()
	}
}
export default storageHelper
