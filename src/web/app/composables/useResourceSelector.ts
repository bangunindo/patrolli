// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { computed, onBeforeUnmount, ref, toValue, watch, type MaybeRefOrGetter } from 'vue'
import { useVirtualList, watchDebounced } from '@vueuse/core'

export interface ResourceSelectorItem {
	id: string
	name: string
	badge?: string
}

export interface UseResourceSelectorOptions<T extends ResourceSelectorItem> {
	isOpen: MaybeRefOrGetter<boolean>
	isInline: MaybeRefOrGetter<boolean>
	items: MaybeRefOrGetter<T[]>
	totalCount: MaybeRefOrGetter<number>
	sourceIds: MaybeRefOrGetter<string[]>
	loading: MaybeRefOrGetter<boolean>
	loadingMore: MaybeRefOrGetter<boolean>
	saving: MaybeRefOrGetter<boolean>
	minSelection: MaybeRefOrGetter<number>
	errorMessage: MaybeRefOrGetter<string | null>
	singularLabel: string
	pluralLabel: string
	searchHandler: MaybeRefOrGetter<((query: string) => Promise<void>) | undefined>
	loadMoreHandler: MaybeRefOrGetter<(() => Promise<void>) | undefined>
	resolveAllHandler: MaybeRefOrGetter<((searchQuery?: string) => Promise<string[]>) | undefined>
	onClearError: () => void
	onCommit: (ids: string[]) => void
}

export function useResourceSelector<T extends ResourceSelectorItem>(
	options: UseResourceSelectorOptions<T>
) {
	const search = ref('')
	const lastSearchedQuery = ref('')
	const draftSelected = ref<string[]>([])
	const selectAllMode = ref(false)
	const selectAllBaseTotal = ref(0)
	const excludedIds = ref<Set<string>>(new Set())
	const resolvingAll = ref(false)
	const localError = ref<string | null>(null)
	const unfilteredTotal = ref(0)

	let isUnmounted = false
	let sessionSeq = 0

	onBeforeUnmount(() => {
		isUnmounted = true
		sessionSeq++
	})

	const isBusy = computed(() => toValue(options.saving) || resolvingAll.value)
	const isSearchPending = computed(
		() =>
			Boolean(toValue(options.searchHandler)) &&
			search.value.trim() !== lastSearchedQuery.value
	)
	const displayError = computed(() => localError.value || toValue(options.errorMessage) || null)
	const draftSelectedSet = computed(() => new Set(draftSelected.value))
	const sourceSignature = computed(() => toValue(options.sourceIds).join('\0'))

	const effectiveTotal = computed(() =>
		Math.max(toValue(options.totalCount), toValue(options.items).length)
	)

	watch(
		[effectiveTotal, lastSearchedQuery, () => toValue(options.loading)] as const,
		([total, query, isLoading]) => {
			if (query === '' && !isLoading) {
				unfilteredTotal.value = total
				if (selectAllMode.value) {
					selectAllBaseTotal.value = total
				}
			}
		},
		{ immediate: true }
	)

	const selectedCount = computed(() => {
		if (selectAllMode.value) {
			return Math.max(0, selectAllBaseTotal.value - excludedIds.value.size)
		}
		return draftSelected.value.length
	})

	const badgeTotal = computed(() => {
		if (selectAllMode.value && search.value.trim() !== '') {
			return selectAllBaseTotal.value
		}
		if (
			toValue(options.searchHandler) &&
			lastSearchedQuery.value !== '' &&
			unfilteredTotal.value > 0
		) {
			return Math.max(unfilteredTotal.value, effectiveTotal.value)
		}
		return effectiveTotal.value
	})

	const hasMore = computed(
		() =>
			Boolean(toValue(options.loadMoreHandler)) &&
			toValue(options.items).length < effectiveTotal.value
	)

	function resetSelectionState(nextDraft: string[] = []) {
		selectAllMode.value = false
		selectAllBaseTotal.value = 0
		excludedIds.value = new Set()
		draftSelected.value = nextDraft
	}

	watch(
		[() => toValue(options.isOpen), () => toValue(options.isInline), sourceSignature] as const,
		([isOpen, isInline], prev) => {
			if (!isOpen && !isInline) {
				sessionSeq++
				resolvingAll.value = false
				return
			}
			resetSelectionState([...toValue(options.sourceIds)])
			resolvingAll.value = false
			const wasActive = prev ? Boolean(prev[0] || prev[1]) : false
			if (!wasActive) {
				sessionSeq++
				const prevQuery = lastSearchedQuery.value
				const isLoading = toValue(options.loading)
				const searchHandler = toValue(options.searchHandler)
				search.value = ''
				lastSearchedQuery.value = ''
				localError.value = null
				unfilteredTotal.value = isLoading ? 0 : effectiveTotal.value
				if (prevQuery !== '' && searchHandler && !isLoading) {
					void searchHandler('')
				}
			}
		},
		{ immediate: true }
	)

	const filteredItems = computed(() => {
		const currentItems = toValue(options.items)
		if (toValue(options.searchHandler)) return currentItems
		const q = search.value.toLowerCase().trim()
		if (q === '') return currentItems
		return currentItems.filter(
			(item) =>
				item.name.toLowerCase().includes(q) ||
				Boolean(item.badge && item.badge.toLowerCase().includes(q))
		)
	})

	const {
		list: virtualItems,
		containerProps,
		wrapperProps,
		scrollTo
	} = useVirtualList(filteredItems, {
		itemHeight: 36,
		overscan: 10
	})

	const renderedItems = computed(() => {
		if (virtualItems.value.length > 0) return virtualItems.value
		return filteredItems.value.slice(0, 20).map((data, index) => ({ data, index }))
	})

	watchDebounced(
		search,
		async (val) => {
			if (isUnmounted || (!toValue(options.isOpen) && !toValue(options.isInline))) return
			const trimmed = val.trim()
			scrollTo(0)
			const searchHandler = toValue(options.searchHandler)
			if (!searchHandler) return
			if (trimmed === lastSearchedQuery.value) return
			lastSearchedQuery.value = trimmed
			await searchHandler(trimmed)
			if (isUnmounted || (!toValue(options.isOpen) && !toValue(options.isInline))) return
			scrollTo(0)
		},
		{ debounce: 300 }
	)

	function handleListScroll(event: Event) {
		containerProps.onScroll()
		const loadMoreHandler = toValue(options.loadMoreHandler)
		if (
			!hasMore.value ||
			toValue(options.loading) ||
			toValue(options.loadingMore) ||
			!loadMoreHandler
		) {
			return
		}
		const el = event.target as HTMLElement | null
		if (!el) return
		if (el.scrollHeight - el.scrollTop - el.clientHeight < 72) {
			void loadMoreHandler()
		}
	}

	function isChecked(id: string): boolean {
		if (selectAllMode.value) {
			return !excludedIds.value.has(id)
		}
		return draftSelectedSet.value.has(id)
	}

	function setChecked(id: string, checked = !isChecked(id)) {
		if (isBusy.value) return
		localError.value = null
		options.onClearError()

		if (selectAllMode.value) {
			const nextExcluded = new Set(excludedIds.value)
			if (checked) nextExcluded.delete(id)
			else nextExcluded.add(id)
			excludedIds.value = nextExcluded
			return
		}

		const idx = draftSelected.value.indexOf(id)
		if (checked && idx === -1) {
			draftSelected.value = [...draftSelected.value, id]
		} else if (!checked && idx > -1) {
			draftSelected.value = draftSelected.value.filter((item) => item !== id)
		}
	}

	function toggleIdBatch(targetIds: string[]) {
		if (targetIds.length === 0) return

		if (selectAllMode.value) {
			const allChecked = targetIds.every((id) => !excludedIds.value.has(id))
			const nextExcluded = new Set(excludedIds.value)
			for (const id of targetIds) {
				if (allChecked) nextExcluded.add(id)
				else nextExcluded.delete(id)
			}
			excludedIds.value = nextExcluded
			return
		}

		const selectedSet = draftSelectedSet.value
		const allInDraft = targetIds.every((id) => selectedSet.has(id))
		if (allInDraft) {
			const targetSet = new Set(targetIds)
			draftSelected.value = draftSelected.value.filter((id) => !targetSet.has(id))
		} else {
			const toAdd = targetIds.filter((id) => !selectedSet.has(id))
			if (toAdd.length > 0) {
				draftSelected.value = [...draftSelected.value, ...toAdd]
			}
		}
	}

	async function resolveAndToggleSearchBatch(trimmedSearch: string) {
		const resolveAll = toValue(options.resolveAllHandler)
		if (!resolveAll) return
		const seq = sessionSeq
		resolvingAll.value = true
		try {
			const matchingIds = await resolveAll(trimmedSearch)
			if (
				isUnmounted ||
				seq !== sessionSeq ||
				(!toValue(options.isOpen) && !toValue(options.isInline))
			) {
				return
			}
			toggleIdBatch(matchingIds)
		} catch (err: any) {
			if (!isUnmounted && seq === sessionSeq) {
				localError.value =
					err?.message || `Failed to resolve matching ${options.pluralLabel}.`
			}
		} finally {
			if (seq === sessionSeq) {
				resolvingAll.value = false
			}
		}
	}

	async function selectAllFiltered() {
		if (isBusy.value || toValue(options.loading) || isSearchPending.value) return
		localError.value = null
		options.onClearError()

		const trimmedSearch = search.value.trim()
		const currentItems = toValue(options.items)
		const hasUnloadedItems = effectiveTotal.value > currentItems.length
		const hasResolveAll = Boolean(toValue(options.resolveAllHandler))

		if (trimmedSearch === '') {
			if (selectAllMode.value && excludedIds.value.size === 0) {
				resetSelectionState()
				return
			}
			if (hasUnloadedItems && hasResolveAll) {
				const allLoadedSelected =
					currentItems.length > 0 &&
					currentItems.every((item) => draftSelectedSet.value.has(item.id))
				const isAllSelected =
					!selectAllMode.value &&
					effectiveTotal.value > 0 &&
					draftSelectedSet.value.size >= effectiveTotal.value &&
					allLoadedSelected
				if (isAllSelected) {
					resetSelectionState()
				} else {
					selectAllMode.value = true
					selectAllBaseTotal.value = effectiveTotal.value
					excludedIds.value = new Set()
				}
				return
			}
		} else if (toValue(options.searchHandler) && hasUnloadedItems && hasResolveAll) {
			await resolveAndToggleSearchBatch(trimmedSearch)
			return
		}

		toggleIdBatch(filteredItems.value.map((item) => item.id))
	}

	function clearAll() {
		if (isBusy.value) return
		localError.value = null
		options.onClearError()
		resetSelectionState()
	}

	function validateMinSelection(count: number): boolean {
		const min = toValue(options.minSelection)
		if (min > 0 && count < min) {
			localError.value =
				min === 1
					? `At least one ${options.singularLabel} must be selected.`
					: `At least ${min} ${options.pluralLabel} must be selected.`
			return false
		}
		return true
	}

	async function applySelection() {
		if (isBusy.value) return
		localError.value = null

		if (!validateMinSelection(selectedCount.value)) return

		let finalIds: string[] = [...draftSelected.value]

		if (selectAllMode.value) {
			const resolveAll = toValue(options.resolveAllHandler)
			if (selectedCount.value === 0) {
				finalIds = []
			} else if (resolveAll) {
				const seq = sessionSeq
				resolvingAll.value = true
				try {
					const allIds = await resolveAll('')
					if (
						isUnmounted ||
						seq !== sessionSeq ||
						(!toValue(options.isOpen) && !toValue(options.isInline))
					) {
						return
					}
					const excluded = excludedIds.value
					finalIds = excluded.size > 0 ? allIds.filter((id) => !excluded.has(id)) : allIds
				} catch (err: any) {
					if (!isUnmounted && seq === sessionSeq) {
						localError.value =
							err?.message || `Failed to fetch all ${options.pluralLabel}.`
					}
					return
				} finally {
					if (seq === sessionSeq) {
						resolvingAll.value = false
					}
				}
			}

			if (!validateMinSelection(finalIds.length)) return
		}

		options.onCommit(finalIds)
	}

	return {
		search,
		isBusy,
		isSearchPending,
		resolvingAll,
		displayError,
		effectiveTotal,
		selectedCount,
		badgeTotal,
		hasMore,
		filteredItems,
		renderedItems,
		containerProps,
		wrapperProps,
		handleListScroll,
		isChecked,
		setChecked,
		selectAllFiltered,
		clearAll,
		applySelection
	}
}
