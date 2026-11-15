// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'

Array.prototype.noDuplicate = function <T>(this: T[]): T[] {
	return [...new Set(this)]
}

Array.prototype.noDuplicateObject = function <T>(
	this: T[],
	keySelector?: keyof T | ((item: T) => any)
): T[] {
	if (this.length === 0) return []

	if (keySelector) {
		return [
			...new Map(
				this.map((item) => {
					const key =
						typeof keySelector === 'function' ? keySelector(item) : item[keySelector]
					return [key, item]
				})
			).values()
		]
	}

	return [...new Set(this.map((obj) => JSON.stringify(obj)))].map((str) => JSON.parse(str))
}
