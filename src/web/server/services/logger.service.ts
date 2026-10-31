// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Unified Logger for Client API Calls and Cluster Forwarding (Server-Side).
 */

export function formatErrorDetail(data: any): string {
	if (!data) return ''
	if (data instanceof Error) {
		const cause = (data as any).cause
			? ` | Cause: ${formatErrorDetail((data as any).cause)}`
			: ''
		const code = (data as any).code ? ` [Code: ${(data as any).code}]` : ''
		return `${data.name}: ${data.message}${code}${cause}`
	}
	if (typeof data === 'string') {
		return data
	}
	if (typeof data === 'object') {
		const parts: string[] = []

		if (data.error) {
			if (typeof data.error === 'string') {
				parts.push(data.error)
			} else if (typeof data.error === 'object') {
				if (data.error.title) parts.push(data.error.title)
				if (data.error.detail) parts.push(String(data.error.detail))
				if (data.error.message && data.error.message !== data.error.detail) {
					parts.push(String(data.error.message))
				}
				if (data.error.description) parts.push(String(data.error.description))
				if (data.error.code) parts.push(`Code: ${data.error.code}`)
				if (data.error.errors && Array.isArray(data.error.errors)) {
					const fieldErrors = data.error.errors
						.map((e: any) =>
							typeof e === 'string'
								? e
								: `${e.field || e.param || e.property || 'field'}: ${e.message || e.detail || JSON.stringify(e)}`
						)
						.join(', ')
					if (fieldErrors) parts.push(`Validation: [${fieldErrors}]`)
				}
			}
		}

		if (data.error_description) parts.push(String(data.error_description))
		if (data.detail && !parts.includes(String(data.detail))) parts.push(String(data.detail))
		if (data.message && !parts.includes(String(data.message))) parts.push(String(data.message))
		if (data.errors && Array.isArray(data.errors)) {
			const errList = data.errors
				.map((e: any) =>
					typeof e === 'string'
						? e
						: `${e.field || e.param || e.property || 'field'}: ${e.message || e.detail || JSON.stringify(e)}`
				)
				.join(', ')
			if (errList) parts.push(`Errors: [${errList}]`)
		}

		if (parts.length > 0) {
			return parts.join(' - ')
		}

		try {
			return JSON.stringify(data)
		} catch {
			return String(data)
		}
	}
	return String(data)
}

export function logApiIncoming(
	method: string,
	path: string,
	clusterHeader?: string | null,
	query?: Record<string, any>
) {
	const timestamp = new Date().toISOString()
	const targetDesc = clusterHeader && clusterHeader.trim() !== '' ? clusterHeader : 'all'
	let queryString = ''
	if (query && Object.keys(query).length > 0) {
		const qParams = new URLSearchParams()
		Object.entries(query).forEach(([k, v]) => {
			if (v !== undefined && v !== null) {
				if (Array.isArray(v)) {
					v.forEach((val) => qParams.append(k, String(val)))
				} else {
					qParams.set(k, String(v))
				}
			}
		})
		const qs = qParams.toString()
		if (qs) {
			queryString = `?${qs}`
		}
	}

	console.log(
		`[API ${timestamp}] [CLIENT CALL] --> ${method.toUpperCase()} ${path}${queryString} (Target: ${targetDesc})`
	)
}

export function logApiResponse(method: string, path: string, status: number, durationMs: number) {
	if (status >= 400) {
		console.error(
			`[API] <-- [CLIENT ERROR ${status}] ${method.toUpperCase()} ${path} (${durationMs}ms)`
		)
	} else {
		console.log(
			`[API] <-- [CLIENT SUCCESS ${status}] ${method.toUpperCase()} ${path} (${durationMs}ms)`
		)
	}
}

export function logClusterForward(method: string, clusterName: string, targetUrl: string) {
	console.log(`[API]   ├── [${clusterName}] Forwarding ${method.toUpperCase()} -> ${targetUrl}`)
}

export function logClusterResponse(
	clusterName: string,
	method: string,
	targetUrl: string,
	status: number,
	durationMs: number,
	errorData?: any
) {
	if (status >= 400) {
		const detail = formatErrorDetail(errorData)
		const detailSuffix = detail ? ` | Error Detail: ${detail}` : ''
		console.warn(
			`[API]   └── [${clusterName}] Response ${status} (${durationMs}ms) <- ${method.toUpperCase()} ${targetUrl}${detailSuffix}`
		)
		if (errorData && typeof errorData === 'object') {
			try {
				const jsonStr = JSON.stringify(errorData, null, 2)
				if (jsonStr !== '{}' && jsonStr.length > 2) {
					console.warn(`[API]   └── [${clusterName}] Response Body:\n${jsonStr}`)
				}
			} catch {
				// Safely ignore if errorData contains circular references or non-serializable content
			}
		}
	} else {
		console.log(
			`[API]   └── [${clusterName}] Response ${status} (${durationMs}ms) <- ${method.toUpperCase()} ${targetUrl}`
		)
	}
}

export function logClusterError(
	clusterName: string,
	method: string,
	targetUrl: string,
	error: string | Error | any,
	durationMs: number
) {
	const detail = formatErrorDetail(error)
	console.error(
		`[API]   └── [${clusterName}] Network/Fetch ERROR: ${detail || error} (${durationMs}ms) <- ${method.toUpperCase()} ${targetUrl}`
	)
	if (error instanceof Error && error.stack) {
		console.error(`[API]   └── [${clusterName}] Stack: ${error.stack}`)
	}
}

export function logApiError(
	method: string,
	path: string,
	status: number,
	error: any,
	extraDetail?: any
) {
	const detail = formatErrorDetail(error)
	const extra = extraDetail ? ` | ${formatErrorDetail(extraDetail)}` : ''
	console.error(
		`[API] <-- [CLIENT ERROR ${status}] ${method.toUpperCase()} ${path}${detail ? ` | ${detail}` : ''}${extra}`
	)

	if (error && typeof error === 'object') {
		if (error instanceof Error && error.stack) {
			console.error(`[API]   └── Stack: ${error.stack}`)
		} else {
			try {
				const jsonStr = JSON.stringify(error, null, 2)
				if (jsonStr !== '{}' && jsonStr.length > 2) {
					console.error(`[API]   └── Full Error Payload:\n${jsonStr}`)
				}
			} catch {
				// Safely ignore if error payload contains circular references or non-serializable content
			}
		}
	}
}
