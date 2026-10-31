// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export const cookieHelper = {
	get(name: string): string | null {
		if (typeof document === 'undefined') return null
		const match = document.cookie.match(new RegExp('(^|;\\s*)(' + name + ')=([^;]*)'))
		return match ? decodeURIComponent(match[3]) : null
	},

	set(name: string, value: string, days?: number, path = '/'): void {
		if (typeof document === 'undefined') return
		let expires = ''
		if (days) {
			const date = new Date()
			date.setTime(date.getTime() + days * 24 * 60 * 60 * 1000)
			expires = '; expires=' + date.toUTCString()
		}
		const isSecure = typeof window !== 'undefined' && window.location?.protocol === 'https:'
		const secFlag = isSecure ? '; Secure' : ''
		document.cookie = `${name}=${encodeURIComponent(value || '')}${expires}; path=${path}; SameSite=Lax${secFlag}`
	},

	remove(name: string, path = '/'): void {
		if (typeof document === 'undefined') return
		document.cookie = `${name}=; expires=Thu, 01 Jan 1970 00:00:00 UTC; path=${path};`
	},

	clearAll(paths: string[] = ['/', '']): void {
		if (typeof document === 'undefined') return
		const cookies = document.cookie.split(';')

		for (const cookie of cookies) {
			const eqPos = cookie.indexOf('=')
			const name = eqPos > -1 ? cookie.substring(0, eqPos).trim() : cookie.trim()
			if (name) {
				for (const path of paths) {
					document.cookie = `${name}=; expires=Thu, 01 Jan 1970 00:00:00 UTC; path=${path};`
				}
			}
		}
	}
}

export default cookieHelper
