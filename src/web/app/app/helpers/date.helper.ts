// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { format, formatDistanceToNow, differenceInMinutes, addMinutes, isValid } from 'date-fns'

/**
 * Parses any incoming date string / timestamp / Date object and ensures
 * it is treated as UTC if no timezone is specified.
 * When converted into a standard JavaScript `Date` instance, standard formatting
 * functions will automatically display it in the user's browser local timezone.
 */
export function parseUtcDate(input: string | number | Date | null | undefined): Date | null {
	if (!input && input !== 0) return null

	if (input instanceof Date) {
		return isValid(input) ? input : null
	}

	if (typeof input === 'number') {
		const d = new Date(input)
		return isValid(d) ? d : null
	}

	if (typeof input === 'string') {
		const trimmed = input.trim()
		if (!trimmed) return null

		// If numeric string timestamp (e.g., "1692600000000")
		if (/^\d+$/.test(trimmed)) {
			const d = new Date(Number(trimmed))
			return isValid(d) ? d : null
		}

		// Normalize space separator to ISO 'T'
		let isoStr =
			trimmed.includes(' ') && !trimmed.includes('T') ? trimmed.replace(' ', 'T') : trimmed

		// If timezone offset or 'Z' is missing, treat the input as UTC by appending 'Z'
		const hasTimezone = /Z|[+-]\d{2}(:?\d{2})?$/i.test(isoStr)
		if (!hasTimezone) {
			isoStr = `${isoStr}Z`
		}

		const d = new Date(isoStr)
		return isValid(d) ? d : null
	}

	return null
}

/**
 * Formats a UTC date string/timestamp into local browser time.
 *
 * @param date UTC date input
 * @param formatPattern date-fns format string (default: 'dd MMM yyyy HH:mm:ss')
 * @param fallback Fallback string if date is null or invalid (default: '-')
 */
export function formatUtcDate(
	date: string | number | Date | null | undefined,
	formatPattern = 'dd MMM yyyy HH:mm:ss',
	fallback = '-'
): string {
	const parsed = parseUtcDate(date)
	if (!parsed) return fallback
	return format(parsed, formatPattern)
}

/**
 * Formats a UTC date into relative time in local browser perspective (e.g. "5 minutes ago").
 *
 * @param date UTC date input
 * @param fallback Fallback string if date is null or invalid (default: '')
 */
export function formatUtcRelative(
	date: string | number | Date | null | undefined,
	fallback = ''
): string {
	const parsed = parseUtcDate(date)
	if (!parsed) return fallback
	return formatDistanceToNow(parsed, { addSuffix: true })
}

/**
 * Formats a UTC date with both local date-time and relative time in parentheses.
 * Example: "21 Aug 2026, 12:11:00 (5 minutes ago)"
 *
 * @param date UTC date input
 * @param formatPattern date-fns format string (default: 'dd MMM yyyy, HH:mm:ss')
 * @param fallback Fallback string if date is null or invalid (default: '—')
 */
export function formatUtcDateTimeWithRelative(
	date: string | number | Date | null | undefined,
	formatPattern = 'dd MMM yyyy, HH:mm:ss',
	fallback = '—'
): string {
	const parsed = parseUtcDate(date)
	if (!parsed) return fallback
	return `${format(parsed, formatPattern)} (${formatDistanceToNow(parsed, { addSuffix: true })})`
}

/**
 * Formats a UTC date into time only (e.g., "14:30:00").
 *
 * @param date UTC date input
 * @param formatPattern date-fns format string (default: 'HH:mm:ss')
 * @param fallback Fallback string if date is null or invalid (default: '—')
 */
export function formatUtcTime(
	date: string | number | Date | null | undefined,
	formatPattern = 'HH:mm:ss',
	fallback = '—'
): string {
	const parsed = parseUtcDate(date)
	if (!parsed) return fallback
	return format(parsed, formatPattern)
}

/**
 * Calculates the difference in minutes between two UTC dates.
 */
export function diffUtcInMinutes(
	dateLeft: string | number | Date | null | undefined,
	dateRight: string | number | Date | null | undefined
): number {
	const left = parseUtcDate(dateLeft)
	const right = parseUtcDate(dateRight)
	if (!left || !right) return 0
	return differenceInMinutes(left, right)
}

/**
 * Adds minutes to a UTC date and returns a local Date object.
 */
export function addUtcMinutes(
	date: string | number | Date | null | undefined,
	amount: number
): Date | null {
	const parsed = parseUtcDate(date)
	if (!parsed) return null
	return addMinutes(parsed, amount)
}
