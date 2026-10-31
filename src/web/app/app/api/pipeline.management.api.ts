// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	CreatePipelineManagementRequest,
	ResponseDataGetPipelineManagementGraphResponse,
	ResponseDataGetPipelineManagementNodeResponse,
	ResponseDataListPipelineManagementResponse,
	ResponseDataPipelineManagementDetail,
	ResponseDataTupleUnit,
	UpdatePipelineManagementGraphRequest,
	UpdatePipelineManagementNodeRequest,
	UpdatePipelineManagementRequest
} from '@@/shared/generated/patrolliAPI.schemas.type'
import type {
	IGetPipelineManagementParams,
	IPipelineManagementQueryParams
} from '@@/shared/types/pipeline.management.type'
import { httpService } from '@/app/services/http.service'

const pipelineManagementApi = {
	path: '/api/pipeline_management',

	async getPipelines(
		query?: IPipelineManagementQueryParams,
		clusterURLs?: string[]
	): Promise<ResponseDataListPipelineManagementResponse | undefined> {
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
	): Promise<ResponseDataPipelineManagementDetail | undefined> {
		return httpService.get(`${this.path}/${id}`, undefined, clusterURLs)
	},

	async createPipeline(
		payload: CreatePipelineManagementRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataPipelineManagementDetail | undefined> {
		return httpService.post(this.path, payload, undefined, clusterURLs)
	},

	async updatePipeline(
		id: string,
		payload: UpdatePipelineManagementRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataPipelineManagementDetail | undefined> {
		return httpService.put(`${this.path}/${id}`, payload, undefined, clusterURLs)
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
		params: IGetPipelineManagementParams & { canvas_id: string },
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineManagementGraphResponse | undefined> {
		return httpService.get(
			`${this.path}/${params.pipeline_management_id}/canvases/${params.canvas_id}/graph`,
			undefined,
			clusterURLs
		)
	},

	async updatePipelineGraph(
		params: IGetPipelineManagementParams & { canvas_id: string },
		payload: UpdatePipelineManagementGraphRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineManagementGraphResponse | undefined> {
		return httpService.put(
			`${this.path}/${params.pipeline_management_id}/canvases/${params.canvas_id}/graph`,
			payload,
			undefined,
			clusterURLs
		)
	},

	async getPipelineNode(
		params: IGetPipelineManagementParams & {
			canvas_id: string
			node_id: string
			query?: { name?: string; limit?: number; offset?: number }
		},
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineManagementNodeResponse | undefined> {
		return httpService.get(
			`${this.path}/${params.pipeline_management_id}/canvases/${params.canvas_id}/nodes/${params.node_id}`,
			params.query ? { params: params.query } : undefined,
			clusterURLs
		)
	},

	async updatePipelineNode(
		params: IGetPipelineManagementParams & { canvas_id: string; node_id: string },
		payload: UpdatePipelineManagementNodeRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataGetPipelineManagementNodeResponse | undefined> {
		return httpService.put(
			`${this.path}/${params.pipeline_management_id}/canvases/${params.canvas_id}/nodes/${params.node_id}`,
			payload,
			undefined,
			clusterURLs
		)
	}
}

export default pipelineManagementApi
