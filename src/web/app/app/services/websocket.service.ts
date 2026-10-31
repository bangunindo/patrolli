// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import tokenHelper from '../helpers/token.helper'

/* global AddEventListenerOptions, BufferSource */
interface AppWebSocketOptions {
	protocols?: string | string[]
	reconnectInterval?: number // Initial reconnect interval (ms)
	maxReconnectInterval?: number // Maximum reconnect interval (ms)
	reconnectDecay?: number // Exponential backoff decay factor
	maxRetries?: number // Maximum reconnect attempts
}

export function normalizeWebSocketUrl(rawUrl: string): string {
	if (typeof window === 'undefined') return rawUrl

	const isPageHttps = window.location.protocol === 'https:'
	const defaultWsProto = isPageHttps ? 'wss:' : 'ws:'

	let finalUrl = rawUrl

	// 1. Relative path: '/ws/...' -> 'ws(s)://hostname:port/ws/...'
	if (rawUrl.startsWith('/')) {
		finalUrl = `${defaultWsProto}//${window.location.host}${rawUrl}`
	} else if (rawUrl.startsWith('http://') || rawUrl.startsWith('https://')) {
		// 2. HTTP/HTTPS URL -> WS/WSS URL
		try {
			const parsed = new URL(rawUrl)
			parsed.protocol = isPageHttps || rawUrl.startsWith('https://') ? 'wss:' : 'ws:'
			finalUrl = parsed.toString()
		} catch {
			finalUrl = rawUrl.replace(/^http:/, 'ws:').replace(/^https:/, 'wss:')
		}
	} else if (isPageHttps && rawUrl.startsWith('ws://')) {
		// 3. Upgrade ws:// to wss:// if current page is served over HTTPS (Mixed Content prevention)
		finalUrl = rawUrl.replace(/^ws:\/\//, 'wss://')
	}

	// 4. Attach access token query parameter if authenticated and not already present
	try {
		const token = tokenHelper.getAccessToken()
		if (token) {
			const parsed = new URL(finalUrl)
			if (!parsed.searchParams.has('token') && !parsed.searchParams.has('access_token')) {
				parsed.searchParams.set('token', token)
				finalUrl = parsed.toString()
			}
		}
	} catch {
		// Ignore any URL parse failures
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

	constructor(url: string, options: AppWebSocketOptions = {}) {
		super()
		this.url = normalizeWebSocketUrl(url)
		this.protocols = options.protocols
		this.reconnectInterval = options.reconnectInterval ?? 1000
		this.maxReconnectInterval = options.maxReconnectInterval ?? 30000
		this.reconnectDecay = options.reconnectDecay ?? 1.5
		this.maxRetries = options.maxRetries ?? Infinity

		this.connect()
	}

	private connect(): void {
		this.ws = new WebSocket(this.url, this.protocols)

		this.ws.onopen = () => {
			this.retries = 0 // Reset counter upon successful connection
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
			// 1. Dispatch 'close' event first
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

			// 2. Perform cleanup AFTER event is dispatched
			this.cleanup()

			// 3. Automatic reconnect logic
			if (!this.isExplicitlyClosed && this.retries < this.maxRetries) {
				const timeout = Math.min(
					this.reconnectInterval * Math.pow(this.reconnectDecay, this.retries),
					this.maxReconnectInterval
				)

				this.retries++
				console.warn(
					`[AppWebSocket] Connection lost. Reconnecting in ${Math.round(timeout)}ms...`
				)

				setTimeout(() => this.connect(), timeout)
			}
		}
	}

	// --- Type Safety Overloads for addEventListener ---
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

	/**
	 * Send data to WebSocket server
	 */
	public send(data: string | Blob | BufferSource): void {
		if (this.ws && this.ws.readyState === WebSocket.OPEN) {
			this.ws.send(data)
		} else {
			console.warn('[AppWebSocket] Cannot send message: WebSocket is not OPEN.')
		}
	}

	/**
	 * Close connection permanently (without reconnecting)
	 */
	public close(code?: number, reason?: string): void {
		this.isExplicitlyClosed = true
		if (this.ws) {
			this.ws.close(code, reason)
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

		if (this.ws) {
			this.cleanup()
			this.ws.close()
			this.ws = null
		}

		this.connect()
	}
}
