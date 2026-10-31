// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { format } from 'date-fns'

declare global {
	interface String {
		capitalize(): string
		titlecase(): string
		formatDate(fmt: string): string
		generateSecureString(length?: number): string
		includeTexts(searchElements: string[]): boolean
	}

	interface Date {
		formatDate(fmt: string): string
	}
}

String.prototype.capitalize = function (this: string): string {
	if (!this) return ''

	return this.split(' ')
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
		.join(' ')
}

String.prototype.titlecase = function (this: string): string {
	if (!this) return ''

	return this.replace(/_id$/, '').replaceAll('_', ' ').replaceAll('-', ' ').capitalize()
}

String.prototype.formatDate = function (this: string, fmt: string = 'dd MMM yyyy - HH:mm'): string {
	const date = new Date(this)

	if (isNaN(date.getTime())) {
		return 'Invalid Date'
	}

	const formatted = format(date, fmt)
	return formatted
}

String.prototype.generateSecureString = function (length: number = 12): string {
	const characters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'
	const charactersLength = characters.length

	if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') {
		const randomValues = new Uint32Array(length)
		crypto.getRandomValues(randomValues)

		let result = ''
		for (let i = 0; i < length; i++) {
			result += characters.charAt(randomValues[i] % charactersLength)
		}
		return result
	}

	let result = ''
	for (let i = 0; i < length; i++) {
		result += characters.charAt(Math.floor(Math.random() * charactersLength))
	}
	return result
}

if (!String.prototype.includeTexts) {
	String.prototype.includeTexts = function (searchElements: string[]): boolean {
		return searchElements.some((element) => this.includes(element))
	}
}

export {}
