// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import {
	CreateEndpointBody,
	ListEndpointsQueryParams,
	ListEndpointsResponse
} from '@@/shared/generated/endpoints.type'
import {
	type Http,
	type SnmpV1,
	type SnmpV2C,
	type SnmpV3
} from '@@/shared/generated/patrolliAPI.schemas.type'
import z from 'zod'
import type { IBaseResponse, IFilterParam } from '@@/shared/types/api.type'

export const ZodEndpoint = ListEndpointsResponse.shape.data.unwrap().shape.endpoints.element
const ZodEndpointConfig = ZodEndpoint.shape.config

export type EndpointConfigMap = {
	snmp_v1: SnmpV1
	snmp_v2c: SnmpV2C
	snmp_v3: SnmpV3
	http: Http
}

export type EndpointType = z.infer<typeof ZodEndpointConfig>['type']

export const EndpointType = ZodEndpointConfig.options.reduce(
	(acc, option) => {
		const typeValue = option.shape.type.options[0] as EndpointType

		const uppercaseKey = typeValue.toUpperCase() as Uppercase<EndpointType>

		acc[uppercaseKey] = typeValue

		return acc
	},
	{} as Record<Uppercase<EndpointType>, EndpointType>
)

export interface IEndpoint extends Omit<z.infer<typeof ZodEndpoint>, 'config'> {
	config?: z.infer<typeof CreateEndpointBody>['config']
	cluster_url?: string
	cluster_name?: string
}

export const EndpointFormSchema = CreateEndpointBody

export type IEndpointForm = z.infer<typeof EndpointFormSchema>

export interface IEndpointListParams
	extends
		Omit<Partial<z.infer<typeof ListEndpointsQueryParams>>, keyof IFilterParam>,
		IFilterParam {}

export interface IEndpointListResponse extends IBaseResponse {
	data: {
		endpoints: IEndpoint[]
		total: number
	}
}

export interface IEndpointResponse extends IBaseResponse {
	data?: IEndpoint
}

export interface IEndpointSelect extends IEndpoint {
	label: string
	value: string
}
