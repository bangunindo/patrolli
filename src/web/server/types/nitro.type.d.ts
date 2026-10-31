// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

declare module '#nitro-internal-pollyfills' {
	const polyfills: unknown
	export default polyfills
}

declare module 'nitropack/runtime/internal' {
	export function trapUnhandledNodeErrors(): void
	export function setupGracefulShutdown(listener: any, nitroApp: any): void
	export function startScheduleRunner(): void
}

declare module 'crossws/adapters/node' {
	const adapter: (websocket: any) => {
		handleUpgrade: (req: any, socket: any, head: any) => void
	}
	export default adapter
}
