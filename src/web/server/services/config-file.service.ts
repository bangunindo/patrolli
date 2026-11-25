// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import YAML from 'yaml'

interface ICachedYamlEntry {
	mtimeMs: number
	size: number
	checkedAt: number
	parsed: any
	raw: string
}

const DEFAULT_CACHE_CHECK_TTL_MS = 2000

const yamlConfigCache = new Map<string, ICachedYamlEntry>()
let lastResolvedDefaultPath: { filePath: string; checkedAt: number } | null = null

export function getDefaultCacheCheckTtlMs(): number {
	return Boolean(process.env.VITEST) || process.env.NODE_ENV === 'test'
		? 0
		: DEFAULT_CACHE_CHECK_TTL_MS
}

export function clearConfigFileCache(): void {
	yamlConfigCache.clear()
	lastResolvedDefaultPath = null
}

function evictCachedConfigPath(filePath: string): void {
	yamlConfigCache.delete(filePath)
	if (lastResolvedDefaultPath?.filePath === filePath) {
		lastResolvedDefaultPath = null
	}
}

export function getCandidateConfigFilePaths(configPath?: string): string[] {
	const explicitPath = configPath || process.env.CONFIG_FILE

	return explicitPath
		? [explicitPath]
		: [
				path.resolve(process.cwd(), 'config-web.yaml'),
				path.resolve(process.cwd(), 'config-web.yml'),
				'/app/config-web.yaml',
				'/config-web.yaml'
			]
}

export function readCachedYamlConfigFile(
	configPath?: string,
	statTtlMs: number = getDefaultCacheCheckTtlMs()
): { filePath: string; parsed: any; raw: string } | null {
	const now = Date.now()
	const explicitPath = configPath || process.env.CONFIG_FILE

	if (statTtlMs > 0) {
		const fastPath = explicitPath || lastResolvedDefaultPath?.filePath
		if (fastPath) {
			const cached = yamlConfigCache.get(fastPath)
			if (cached && now - cached.checkedAt < statTtlMs) {
				return { filePath: fastPath, parsed: cached.parsed, raw: cached.raw }
			}
		}
	}

	const candidatePaths = getCandidateConfigFilePaths(configPath)

	for (const filePath of candidatePaths) {
		try {
			if (!fs.existsSync(filePath)) {
				evictCachedConfigPath(filePath)
				continue
			}

			let stat: fs.Stats | null = null
			try {
				stat = fs.statSync(filePath)
			} catch {
				// Fall back to uncached read when statSync is unavailable (e.g. virtual fs mocks)
				stat = null
			}

			if (stat) {
				const cached = yamlConfigCache.get(filePath)
				if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size) {
					cached.checkedAt = now
					if (!explicitPath) {
						lastResolvedDefaultPath = { filePath, checkedAt: now }
					}
					return { filePath, parsed: cached.parsed, raw: cached.raw }
				}
			}

			const raw = fs.readFileSync(filePath, 'utf-8')
			if (!raw.trim()) {
				evictCachedConfigPath(filePath)
				continue
			}

			const parsed = YAML.parse(raw)
			if (stat) {
				yamlConfigCache.set(filePath, {
					mtimeMs: stat.mtimeMs,
					size: stat.size,
					checkedAt: now,
					parsed,
					raw
				})
				if (!explicitPath) {
					lastResolvedDefaultPath = { filePath, checkedAt: now }
				}
			}

			return { filePath, parsed, raw }
		} catch {
			evictCachedConfigPath(filePath)
		}
	}

	return null
}
