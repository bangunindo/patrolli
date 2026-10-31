// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export default {
	configs: {
		null: null,
		transform_script: {
			script: 'metric:set_value("host_id", "abc")\nreturn metric'
		},
		filter_script: {
			script: 'metric:compare_value("netflow.flow_set_record.ipNextHopIPv4Address", "==", "10.0.0.1")',
			pass_on_error: false
		},
		populate_metadata: null
	}
}
