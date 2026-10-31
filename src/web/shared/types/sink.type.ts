// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import {
	CreateSinkBody,
	GetSinkResponse,
	ListSinksQueryParams,
	ListSinksResponse
} from '@@/shared/generated/sinks.type'
import {
	Format,
	type Clickhouse as ISinkClickhouseConfig,
	type DiskChannel,
	type File as ISinkFileConfig,
	type Kafka as ISinkKafkaConfig,
	type OpenTelemetry as ISinkOpenTelemetryConfig,
	type MemoryChannel
} from '@@/shared/generated/patrolliAPI.schemas.type'
import type z from 'zod'
import type { IBaseResponse, IFilterParam } from '@@/shared/types/api.type'

export type SinkChanelMap = {
	memory: MemoryChannel & { type: 'memory' }
	disk: DiskChannel & { type: 'disk' }
}

export type SinkConfigMap = {
	file: ISinkFileConfig
	kafka: ISinkKafkaConfig
	open_telemetry: ISinkOpenTelemetryConfig
	clickhouse: ISinkClickhouseConfig
}

export type SinkConfigFileFormat = Format

export const ZodSink = GetSinkResponse.shape.data.unwrap()
export const ZodSinks = ListSinksResponse.shape.data.unwrap().shape.sinks.element
const ZodSinkConfig = CreateSinkBody.shape.config
const ZodSinkChanel = CreateSinkBody.shape.channel

export type SinkConfigType = z.infer<typeof ZodSinkConfig>['type']

export const SinkConfigType = ZodSinkConfig.options.reduce(
	(acc, option) => {
		const typeValue = option.shape.type.options[0] as SinkConfigType

		const uppercaseKey = typeValue.toUpperCase() as Uppercase<SinkConfigType>

		acc[uppercaseKey] = typeValue

		return acc
	},
	{} as Record<Uppercase<SinkConfigType>, SinkConfigType>
)

export type SinkChanelType = z.infer<typeof ZodSinkChanel>['type']

export const SinkChanelType = ZodSinkChanel.options.reduce(
	(acc, option) => {
		const typeValue = option._def.right.shape.type._def.values[0] as SinkChanelType

		const uppercaseKey = typeValue.toUpperCase() as Uppercase<SinkChanelType>

		acc[uppercaseKey] = typeValue

		return acc
	},
	{} as Record<Uppercase<SinkChanelType>, SinkChanelType>
)

const compressionRaw = ZodSinkChanel.options[1]._def.left.shape.compression
const compression = compressionRaw.unwrap()
export type SinkChanelCompression = z.infer<typeof compression>

export const SinkChanelCompression = compression.options.reduce(
	(acc, option) => {
		const typeValue = option
		const uppercaseKey = typeValue.toUpperCase() as Uppercase<SinkChanelCompression>

		acc[uppercaseKey] = typeValue

		return acc
	},
	{} as Record<Uppercase<SinkChanelCompression>, SinkChanelCompression>
)

export interface ISink extends Partial<z.infer<typeof ZodSink>> {
	cluster_url?: string
	cluster_name?: string
}

export interface ISinks extends z.infer<typeof ZodSinks> {
	cluster_url?: string
	cluster_name?: string
}

export interface ISinkForm extends z.infer<typeof CreateSinkBody> {}

export interface ISinkListParams
	extends Omit<Partial<z.infer<typeof ListSinksQueryParams>>, keyof IFilterParam>, IFilterParam {}

export interface ISinkListResponse extends IBaseResponse {
	data: {
		sinks: ISinks[]
		total: number
	}
}

export interface ISinkBatch {
	batch_size: number
	batch_timeout: number
}

export interface ISinkChannel {
	compression: SinkChanelCompression
	max_disk_size: number
	max_memtable_size: number
	non_blocking: boolean
	type: SinkChanelType
	worker_count: number
}

export interface ISinkResponse extends IBaseResponse {
	data?: ISink
}
