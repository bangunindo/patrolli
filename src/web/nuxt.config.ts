// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { fileURLToPath } from 'node:url'
import { defineNuxtConfig } from 'nuxt/config'
import { loadSslConfig } from './server/services/ssl.service'

const sslConfig = loadSslConfig()

// https://nuxt.com/docs/api/configuration/nuxt-config
export default defineNuxtConfig({
	compatibilityDate: '2026-10-21',
	devtools: { enabled: process.env.NODE_ENV !== 'production' },
	ssr: true,
	srcDir: 'app',

	devServer: {
		https: sslConfig
			? {
					key: sslConfig.keyPath,
					cert: sslConfig.certPath
				}
			: false
	},

	modules: ['@pinia/nuxt', 'shadcn-nuxt'],

	shadcn: {
		prefix: 'Ui',
		componentDir: './app/components/ui'
	},

	components: [
		{
			path: '~/components',
			pathPrefix: true
		}
	],

	css: ['vue3-json-viewer/dist/vue3-json-viewer.css', '@/assets/styles/main.scss'],

	postcss: {
		plugins: {
			'tailwindcss/nesting/index.js': {},
			tailwindcss: {},
			autoprefixer: {}
		}
	},

	app: {
		head: {
			title: 'Patrolli',
			htmlAttrs: {
				lang: 'en'
			},
			meta: [
				{ charset: 'utf-8' },
				{ name: 'viewport', content: 'width=device-width, initial-scale=1.0' }
			],
			link: [
				{ rel: 'icon', type: 'image/x-icon', href: '/favicon.ico' },
				{ rel: 'preconnect', href: 'https://fonts.googleapis.com' },
				{ rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' },
				{
					rel: 'stylesheet',
					href: 'https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:ital,wght@0,100..700;1,100..700&family=IBM+Plex+Sans:ital,wght@0,100..700;1,100..700&display=swap'
				}
			]
		}
	},

	alias: {
		'entities/lib/decode.js': 'entities',
		'entities/lib/decode': 'entities'
	},

	nitro: {
		preset: 'node-server',
		imports: {
			dirs: ['server/services', 'server/services/**']
		},
		experimental: {
			websocket: true
		},
		...(process.env.NODE_ENV === 'production'
			? { entry: fileURLToPath(new URL('./server/entry.ts', import.meta.url)) }
			: {}),
		externals: {
			inline: [
				'vue',
				'@vue/shared',
				'@vue/compiler-dom',
				'entities',
				'@vue/runtime-dom',
				'@vue/runtime-core',
				'@vue/reactivity',
				'@vue/server-renderer',
				'@vue/compiler-ssr'
			]
		},
		publicAssets: [
			{
				dir: fileURLToPath(new URL('./public', import.meta.url)),
				maxAge: 0
			}
		]
	},

	vite: {
		css: {
			preprocessorOptions: {
				scss: {
					api: 'modern-compiler'
				}
			}
		},

		server: {},

		build: {
			sourcemap: false
		}
	}
})
