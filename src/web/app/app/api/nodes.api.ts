// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { IClusterNodeListResponse } from '@@/shared/types/cluster.type'
import { httpService } from '@/app/services/http.service'

const nodesApi = {
	path: '/api/nodes',

	async getNodes(): Promise<IClusterNodeListResponse | undefined> {
		return httpService.get(this.path)
	}
}

export default nodesApi
