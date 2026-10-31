// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { FailureKind, ItemState } from '@@/shared/generated/patrolliAPI.schemas.type'
import type { FailureKindMeta, StateMeta } from '@@/shared/types/item.type'

export const STATE_META_MAP: Record<ItemState, StateMeta> = {
	ok: {
		label: 'OK',
		sublabel: 'Active & Collecting',
		color: 'success',
		dotClass: 'bg-emerald-500',
		icon: 'check-circle'
	},
	ok_empty: {
		label: 'OK (Empty)',
		sublabel: 'Succeeded with 0 rows returned',
		color: 'success',
		dotClass: 'bg-emerald-400',
		icon: 'check-circle-outline'
	},
	retrying: {
		label: 'Retrying',
		sublabel: 'Transient failure, retries remaining',
		color: 'warning',
		dotClass: 'bg-amber-500 animate-pulse',
		icon: 'refresh'
	},
	failing: {
		label: 'Failing',
		sublabel: 'Retries exhausted, check configuration or target',
		color: 'error',
		dotClass: 'bg-rose-500',
		icon: 'alert-circle'
	},
	stale: {
		label: 'Stale',
		sublabel: 'Scheduler overdue (Patrolli node issue, target is fine)',
		color: 'info',
		dotClass: 'bg-[--b-color-info]',
		icon: 'clock-alert-outline'
	},
	never_ran: {
		label: 'Never Ran',
		sublabel: 'Registered but has not executed yet',
		color: 'secondary',
		dotClass: 'bg-slate-400',
		icon: 'clock-outline'
	},
	unknown: {
		label: 'Unknown',
		sublabel: 'No stats reported (disabled, unassigned, or node unreachable)',
		color: 'secondary',
		dotClass: 'bg-slate-400',
		icon: 'help-circle-outline'
	}
}

export const FAILURE_KIND_META_MAP: Record<FailureKind, FailureKindMeta> = {
	timeout: {
		label: 'Timeout',
		category: 'Target',
		hint: 'Deadline passed with no response. Raise the timeout in item configuration, or check if the target is overloaded / filtered.',
		color: 'text-amber-600 dark:text-amber-400 bg-amber-50 dark:bg-amber-950/40 border-amber-200 dark:border-amber-900/50'
	},
	unreachable: {
		label: 'Unreachable',
		category: 'Target',
		hint: 'Request never reached the target. Check network routing, firewall/ACL rules, DNS resolution, or host power state.',
		color: 'text-rose-600 dark:text-rose-400 bg-rose-50 dark:bg-rose-950/40 border-rose-200 dark:border-rose-900/50'
	},
	auth: {
		label: 'Authentication Failed',
		category: 'Target',
		hint: 'Credentials were rejected by target. Check community string, SNMPv3 user/auth/priv keys, or HTTP bearer token.',
		color: 'text-rose-600 dark:text-rose-400 bg-rose-50 dark:bg-rose-950/40 border-rose-200 dark:border-rose-900/50'
	},
	not_supported: {
		label: 'Not Supported',
		category: 'Target',
		hint: 'Feature or OID not supported by target. Check the item OID or device MIB / capability settings.',
		color: 'text-orange-600 dark:text-orange-400 bg-orange-50 dark:bg-orange-950/40 border-orange-200 dark:border-orange-900/50'
	},
	target_error: {
		label: 'Target Error',
		category: 'Target',
		hint: 'Target device returned an explicit error. Check device state or reduce bulk request repetition size.',
		color: 'text-rose-600 dark:text-rose-400 bg-rose-50 dark:bg-rose-950/40 border-rose-200 dark:border-rose-900/50'
	},
	malformed: {
		label: 'Malformed Response',
		category: 'Target',
		hint: 'Answer arrived unusable or unparseable. Check device agent/exporter output format or disable bulk queries.',
		color: 'text-amber-600 dark:text-amber-400 bg-amber-50 dark:bg-amber-950/40 border-amber-200 dark:border-amber-900/50'
	},
	config: {
		label: 'Config Error',
		category: 'Config',
		hint: 'Patrolli-side configuration error (missing endpoint, invalid parameters). Non-retryable — fix item or endpoint settings.',
		color: 'text-violet-600 dark:text-violet-400 bg-violet-50 dark:bg-violet-950/40 border-violet-200 dark:border-violet-900/50'
	},
	dropped: {
		label: 'Data Dropped',
		category: 'Patrolli',
		hint: 'Data collected successfully but dropped before transmission. Check Patrolli node capacity and queue thresholds.',
		color: 'text-purple-600 dark:text-purple-400 bg-purple-50 dark:bg-purple-950/40 border-purple-200 dark:border-purple-900/50'
	},
	cluster: {
		label: 'Cluster Node Error',
		category: 'Patrolli',
		hint: 'Could not reach the Patrolli cluster node managing this item. Check cluster peer health and interconnect connectivity.',
		color: 'text-indigo-600 dark:text-indigo-400 bg-indigo-50 dark:bg-indigo-950/40 border-indigo-200 dark:border-indigo-900/50'
	},
	internal: {
		label: 'Internal Patrolli Bug',
		category: 'Patrolli',
		hint: 'Internal Patrolli engine exception. Please inspect node logs and report this issue.',
		color: 'text-red-700 dark:text-red-400 bg-red-50 dark:bg-red-950/40 border-red-200 dark:border-red-900/50'
	}
}

export default {
	icon: {
		state: {
			ok: 'file-arrow-up-down-outline',
			ok_empty: 'file-alert-outline',
			never_ran: 'file-clock-outline',
			retrying: 'file-refresh-outline',
			failing: 'file-document-remove-outline',
			stale: 'file-hidden',
			unknown: 'file-question-outline'
		}
	},
	color: {
		state: {
			ok: 'success',
			ok_empty: 'success',
			never_ran: 'secondary',
			retrying: 'warning',
			failing: 'error',
			stale: 'info',
			unknown: 'tertiary'
		},
		class: {
			ok: 'text-[--b-color-success]',
			ok_empty: 'text-[--b-color-success]',
			never_ran: 'text-[--b-color-secondary]',
			retrying: 'text-[--b-color-warning]',
			failing: 'text-[--b-color-error]',
			stale: 'text-[--b-color-info]',
			unknown: 'text-[--b-color-tertiary]'
		}
	},
	stateMeta: STATE_META_MAP,
	failureKindMeta: FAILURE_KIND_META_MAP
}
