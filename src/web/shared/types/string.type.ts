// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

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

export {}
