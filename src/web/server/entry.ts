// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// @ts-ignore
import '#nitro-internal-pollyfills'
import { Server as HttpServer } from 'node:http'
import { Server as HttpsServer } from 'node:https'
// @ts-ignore
import wsAdapter from 'crossws/adapters/node'
import destr from 'destr'
import { toNodeListener } from 'h3'
import { useNitroApp, useRuntimeConfig } from 'nitropack/runtime'
// @ts-ignore
import { loadSslConfig } from '@@/server/services/ssl.service'
import {
	setupGracefulShutdown,
	startScheduleRunner,
	trapUnhandledNodeErrors
} from 'nitropack/runtime/internal'

const ssl = loadSslConfig()
const cert = ssl?.cert || process.env.NITRO_SSL_CERT
const key = ssl?.key || process.env.NITRO_SSL_KEY

const nitroApp = useNitroApp()
const server =
	cert && key
		? new HttpsServer({ key, cert }, toNodeListener(nitroApp.h3App))
		: new HttpServer(toNodeListener(nitroApp.h3App))

const port = destr(process.env.NITRO_PORT || process.env.PORT) || 3000
const host = process.env.NITRO_HOST || process.env.HOST
const socketPath = process.env.NITRO_UNIX_SOCKET

const listener = server.listen(socketPath ? { path: socketPath } : { port, host }, (err?: any) => {
	if (err) {
		console.error(err)
		process.exit(1)
	}
	const protocol = cert && key ? 'https' : 'http'
	const addressInfo = listener.address()
	if (typeof addressInfo === 'string') {
		console.log(`Listening on unix socket ${addressInfo}`)
		return
	}
	const baseURL = (useRuntimeConfig().app.baseURL || '').replace(/\/$/, '')
	const addr =
		addressInfo && (addressInfo as any).family === 'IPv6'
			? `[${addressInfo.address}]`
			: (addressInfo as any)?.address
	const listenPort = (addressInfo as any)?.port
	console.log(`Listening on ${protocol}://${addr}:${listenPort}${baseURL}`)
})

server.on('clientError', (err: any, socket) => {
	if (err.code === 'ECONNRESET' || !socket.writable) {
		return
	}
	socket.end('HTTP/1.1 400 Bad Request\r\n\r\n')
})

trapUnhandledNodeErrors()
setupGracefulShutdown(listener, nitroApp)

if (import.meta._websocket || (nitroApp.h3App as any)?.websocket) {
	try {
		const { handleUpgrade } = wsAdapter((nitroApp.h3App as any)?.websocket || {})
		server.on('upgrade', (req, socket, head) => {
			handleUpgrade(req, socket, head)
		})
	} catch (err) {
		console.warn('[Patrolli-FE WS] Failed to initialize WebSocket upgrade handler:', err)
	}
}

if (import.meta._tasks) {
	startScheduleRunner()
}

export default {}
