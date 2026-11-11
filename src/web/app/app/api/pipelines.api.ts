// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { AxiosRequestConfig } from 'axios'
import type {
	CreatePipelineRequest,
	ResponseDataGetPipelineGraphResponse,
	ResponseDataGetPipelineNodeResponse,
	ResponseDataListPipelineResponse,
	ResponseDataPipelineDetail,
	ResponseDataTupleUnit,
	UpdatePipelineGraphRequest,
	UpdatePipelineNodeRequest,
	UpdatePipelineRequest
} from '@@/shared/generated/patrolliAPI.schemas.type'
import type { IGetPipelineParams, IPipelineQueryParams } from '@@/shared/types/pipeline.type'
import { httpService } from '@/app/services/http.service'

const pipelinesApi = {
	path: '/api/pipelines',

	async getPipelines(
		query?: IPipelineQueryParams,
		clusterURLs?: string[]
	): Promise<ResponseDataListPipelineResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async getPipelineById(
		id: string,
		clusterURLs?: string[]
	): Promise<ResponseDataPipelineDetail | undefined> {
		return httpService.get(`${this.path}/${id}`, undefined, clusterURLs)
	},

	async createPipeline(
		payload: CreatePipelineRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataPipelineDetail | undefined> {
		return httpService.post(this.path, payload, undefined, clusterURLs)
	},

	async updatePipeline(
		id: string,
		payload: UpdatePipelineRequest,
		clusterURLs?: string[],
		config?: AxiosRequestConfig
	): Promise<ResponseDataPipelineDetail | undefined> {
		return httpService.put(`${this.path}/${id}`, payload, config, clusterURLs)
	},

	async deletePipeline(
		id: string,
		clusterURLs?: string[]
	): Promise<ResponseDataTupleUnit | undefined> {
		try {
			return await httpService.delete(`${this.path}/${id}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete pipeline')
		}
	},

	async getPipelineGraph(
		params: IGetPipelineParams & { canvas_id: string },
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineGraphResponse | undefined> {
		return httpService.get(
			`${this.path}/${params.pipeline_id}/canvases/${params.canvas_id}/graph`,
			undefined,
			clusterURLs
		)
	},

	async updatePipelineGraph(
		params: IGetPipelineParams & { canvas_id: string },
		payload: UpdatePipelineGraphRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineGraphResponse | undefined> {
		return httpService.put(
			`${this.path}/${params.pipeline_id}/canvases/${params.canvas_id}/graph`,
			payload,
			undefined,
			clusterURLs
		)
	},

	async getPipelineNode(
		params: IGetPipelineParams & {
			canvas_id: string
			node_id: string
			query?: { name?: string; limit?: number; offset?: number }
		},
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineNodeResponse | undefined> {
		return httpService.get(
			`${this.path}/${params.pipeline_id}/canvases/${params.canvas_id}/nodes/${params.node_id}`,
			params.query ? { params: params.query } : undefined,
			clusterURLs
		)
	},

	async updatePipelineNode(
		params: IGetPipelineParams & { canvas_id: string; node_id: string },
		payload: UpdatePipelineNodeRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineNodeResponse | undefined> {
		return httpService.put(
			`${this.path}/${params.pipeline_id}/canvases/${params.canvas_id}/nodes/${params.node_id}`,
			payload,
			undefined,
			clusterURLs
		)
	}
}

export default pipelinesApi
