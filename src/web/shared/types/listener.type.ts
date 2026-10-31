// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import {
	CreateListenerBody,
	ListListenersQueryParams,
	ListListenersResponse
} from '@@/shared/generated/listeners.type'
import { z } from 'zod'
import type { IBaseResponse, IFilterParam } from '@@/shared/types/api.type'

export const ZodListener = ListListenersResponse.shape.data.unwrap().shape.listeners.element
const ZodListenerConfig = ZodListener.shape.config

export type ListenerType = z.infer<typeof ZodListenerConfig>['type']

export const ListenerType = ZodListenerConfig.options.reduce(
	(acc, option) => {
		const typeValue = option.shape.type.options[0] as ListenerType

		const uppercaseKey = typeValue.toUpperCase() as Uppercase<ListenerType>

		acc[uppercaseKey] = typeValue

		return acc
	},
	{} as Record<Uppercase<ListenerType>, ListenerType>
)

import type {
	DummyListener,
	Netflow,
	OpenTelemetryListener,
	SnmpTrap,
	Syslog
} from '@@/shared/generated/patrolliAPI.schemas.type'

export type ListenerConfigMap = {
	dummy: DummyListener
	netflow: Netflow
	snmp_trap: SnmpTrap
	syslog: Syslog
	open_telemetry: OpenTelemetryListener
}

export interface IListener extends z.infer<typeof ZodListener> {
	cluster_url?: string
	cluster_name?: string
}

export const ListenerFormSchema = CreateListenerBody.extend({
	name: z.string().min(1, 'Name is required'),
	config: ZodListenerConfig
})

export interface IListenerForm extends z.infer<typeof ListenerFormSchema> {}

export interface IListenerListParams
	extends
		Omit<Partial<z.infer<typeof ListListenersQueryParams>>, keyof IFilterParam>,
		IFilterParam {}

export interface IListenerListResponse extends IBaseResponse {
	data: {
		listeners: IListener[]
		total: number
	}
}

export interface IListenerResponse extends IBaseResponse {
	data?: IListener
}

export interface IListenerSelect extends IListener {
	label: string
	value: string
}
