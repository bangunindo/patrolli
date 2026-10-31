// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export function formatSchedule(schedule: string | number | undefined | null): string {
	if (schedule === undefined || schedule === null) return '-'

	const str = String(schedule).trim()
	if (!str) return '-'
	if (str.toLowerCase() === 'never') return 'Never'

	// If numeric value (seconds)
	if (/^\d+$/.test(str)) {
		const sec = parseInt(str, 10)
		if (sec === 0) return 'Never'
		if (sec % 86400 === 0) return `Every ${sec / 86400} Day${sec / 86400 > 1 ? 's' : ''}`
		if (sec % 3600 === 0) return `Every ${sec / 3600} Hour${sec / 3600 > 1 ? 's' : ''}`
		if (sec % 60 === 0) return `Every ${sec / 60} Minute${sec / 60 > 1 ? 's' : ''}`
		return `Every ${sec} Second${sec > 1 ? 's' : ''}`
	}

	// For cron strings like "0 */21 * * * ?"
	const parts = str.split(/\s+/)
	if (parts.length >= 5) {
		const [secOrMin, minOrHour, hourOrDay] = parts

		// Pattern: 0 */X * * * ? -> Every X Minutes
		if (secOrMin === '0' && minOrHour.startsWith('*/')) {
			const mins = minOrHour.replace('*/', '')
			return `Every ${mins} Minute${parseInt(mins) > 1 ? 's' : ''}`
		}

		// Pattern: 0 0 */X * * ? -> Every X Hours
		if (secOrMin === '0' && minOrHour === '0' && hourOrDay.startsWith('*/')) {
			const hours = hourOrDay.replace('*/', '')
			return `Every ${hours} Hour${parseInt(hours) > 1 ? 's' : ''}`
		}
	}

	return str
}
