// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { useAuthStore } from '@/stores/auth.store'

/* global AddEventListenerOptions, BufferSource */
interface AppWebSocketOptions {
	protocols?: string | string[]
	reconnectInterval?: number
	maxReconnectInterval?: number
	reconnectDecay?: number
	maxRetries?: number
}

export function normalizeWebSocketUrl(rawUrl: string): string {
	if (typeof window === 'undefined') return rawUrl

	const isPageHttps = window.location.protocol === 'https:'
	const defaultWsProto = isPageHttps ? 'wss:' : 'ws:'

	let finalUrl = rawUrl

	if (rawUrl.startsWith('/')) {
		finalUrl = `${defaultWsProto}//${window.location.host}${rawUrl}`
	} else if (rawUrl.startsWith('http://') || rawUrl.startsWith('https://')) {
		try {
			const parsed = new URL(rawUrl)
			parsed.protocol = isPageHttps || rawUrl.startsWith('https://') ? 'wss:' : 'ws:'
			finalUrl = parsed.toString()
		} catch {
			finalUrl = rawUrl.replace(/^http:/, 'ws:').replace(/^https:/, 'wss:')
		}
	} else if (isPageHttps && rawUrl.startsWith('ws://')) {
		// Upgrade ws:// to wss:// if current page is served over HTTPS (Mixed Content prevention)
		finalUrl = rawUrl.replace(/^ws:\/\//, 'wss://')
	}

	return finalUrl
}

export class AppWebSocket extends EventTarget {
	private url: string
	private protocols?: string | string[]
	private ws: WebSocket | null = null

	private reconnectInterval: number
	private maxReconnectInterval: number
	private reconnectDecay: number
	private maxRetries: number
	private retries = 0
	private isExplicitlyClosed = false
	private connectAttemptId = 0
	private reconnectTimeoutId: ReturnType<typeof setTimeout> | null = null

	constructor(url: string, options: AppWebSocketOptions = {}) {
		super()
		this.url = normalizeWebSocketUrl(url)
		this.protocols = options.protocols
		this.reconnectInterval = options.reconnectInterval ?? 1000
		this.maxReconnectInterval = options.maxReconnectInterval ?? 30000
		this.reconnectDecay = options.reconnectDecay ?? 1.5
		this.maxRetries = options.maxRetries ?? Infinity

		void this.connect()
	}

	private clearReconnectTimeout(): void {
		if (this.reconnectTimeoutId !== null) {
			clearTimeout(this.reconnectTimeoutId)
			this.reconnectTimeoutId = null
		}
	}

	private async connect(): Promise<void> {
		this.clearReconnectTimeout()
		const attemptId = ++this.connectAttemptId

		if (import.meta.client || typeof window !== 'undefined') {
			try {
				await useAuthStore().ensureValidAccessToken()
			} catch {
				// Proceed with WebSocket connection even if pre-flight check fails
			}
			if (this.isExplicitlyClosed || attemptId !== this.connectAttemptId) return
		}

		this.ws = new WebSocket(this.url, this.protocols)

		this.ws.onopen = () => {
			this.retries = 0
			this.dispatchEvent(new Event('open'))
		}

		this.ws.onmessage = (event: MessageEvent) => {
			console.log('[AppWebSocket Message from Cluster]:', event.data)
			this.dispatchEvent(new MessageEvent('message', { data: event.data }))
		}

		this.ws.onerror = () => {
			this.dispatchEvent(new Event('error'))
		}

		this.ws.onclose = (event: CloseEvent) => {
			try {
				this.dispatchEvent(
					new CloseEvent('close', {
						code: event.code,
						reason: event.reason,
						wasClean: event.wasClean
					})
				)
			} catch {
				// Fallback if environment does not support CloseEvent constructor
				this.dispatchEvent(new Event('close'))
			}

			// Cleanup handlers only after 'close' listeners have executed
			this.cleanup()

			if (!this.isExplicitlyClosed && this.retries < this.maxRetries) {
				const timeout = Math.min(
					this.reconnectInterval * Math.pow(this.reconnectDecay, this.retries),
					this.maxReconnectInterval
				)

				this.retries++
				console.warn(
					`[AppWebSocket] Connection lost. Reconnecting in ${Math.round(timeout)}ms...`
				)

				this.reconnectTimeoutId = setTimeout(() => {
					this.reconnectTimeoutId = null
					void this.connect()
				}, timeout)
			}
		}
	}

	public override addEventListener(
		type: 'open' | 'error',
		listener: (event: Event) => void,
		options?: boolean | AddEventListenerOptions
	): void
	public override addEventListener(
		type: 'message',
		listener: (event: MessageEvent) => void,
		options?: boolean | AddEventListenerOptions
	): void
	public override addEventListener(
		type: 'close',
		listener: (event: CloseEvent) => void,
		options?: boolean | AddEventListenerOptions
	): void
	public override addEventListener(
		type: string,
		listener: any,
		options?: boolean | AddEventListenerOptions
	): void {
		super.addEventListener(type, listener, options)
	}

	public send(data: string | Blob | BufferSource): void {
		if (this.ws && this.ws.readyState === WebSocket.OPEN) {
			this.ws.send(data)
		} else {
			console.warn('[AppWebSocket] Cannot send message: WebSocket is not OPEN.')
		}
	}

	public close(code?: number, reason?: string): void {
		this.isExplicitlyClosed = true
		this.connectAttemptId++
		this.clearReconnectTimeout()
		if (this.ws) {
			this.cleanup()
			this.ws.close(code, reason)
			this.ws = null
		}
	}

	private cleanup(): void {
		if (this.ws) {
			this.ws.onopen = null
			this.ws.onmessage = null
			this.ws.onerror = null
			this.ws.onclose = null
		}
	}

	public get readyState(): number {
		return this.ws ? this.ws.readyState : WebSocket.CLOSED
	}

	public reconnect(): void {
		this.isExplicitlyClosed = false
		this.retries = 0
		this.connectAttemptId++
		this.clearReconnectTimeout()

		if (this.ws) {
			this.cleanup()
			this.ws.close()
			this.ws = null
		}

		void this.connect()
	}
}
