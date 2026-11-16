// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

declare global {
	interface Array<T> {
		noDuplicate(): T[]
		noDuplicateObject(keySelector?: keyof T | ((item: T) => any)): T[]
	}
}

export {}
