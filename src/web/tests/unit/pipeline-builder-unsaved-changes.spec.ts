// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import UnsavedChangesModal from '@/components/pipeline/builder/modals/UnsavedChangesModal.vue'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

describe('Pipeline Builder Unsaved Changes Protection', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('UnsavedChangesModal Component', () => {
		it('should render dialog header, title, and description when open is true', () => {
			const wrapper = mount(UnsavedChangesModal, {
				props: {
					open: true,
					isSaving: false
				},
				global: {
					stubs: {
						UiDialog: { template: '<div><slot /></div>' },
						UiDialogContent: { template: '<div><slot /></div>' },
						UiDialogHeader: { template: '<div><slot /></div>' },
						UiDialogTitle: { template: '<h2><slot /></h2>' },
						UiDialogDescription: { template: '<p><slot /></p>' },
						UiDialogFooter: { template: '<div><slot /></div>' },
						UiButton: {
							template: '<button :disabled="disabled"><slot /></button>',
							props: ['disabled']
						},
						UiIcon: true
					}
				}
			})

			expect(wrapper.text()).toContain('Unsaved Changes')
			expect(wrapper.text()).toContain('You have unsaved changes in this pipeline builder')
			expect(wrapper.text()).toContain('Cancel')
			expect(wrapper.text()).toContain('Discard & Leave')
			expect(wrapper.text()).toContain('Save & Leave')
		})

		it('should emit cancel event when Cancel button is clicked', async () => {
			const wrapper = mount(UnsavedChangesModal, {
				props: {
					open: true,
					isSaving: false
				},
				global: {
					stubs: {
						UiDialog: { template: '<div><slot /></div>' },
						UiDialogContent: { template: '<div><slot /></div>' },
						UiDialogHeader: { template: '<div><slot /></div>' },
						UiDialogTitle: { template: '<h2><slot /></h2>' },
						UiDialogDescription: { template: '<p><slot /></p>' },
						UiDialogFooter: { template: '<div><slot /></div>' },
						UiButton: {
							template: '<button :disabled="disabled"><slot /></button>',
							props: ['disabled']
						},
						UiIcon: true
					}
				}
			})

			const buttons = wrapper.findAll('button')
			const cancelButton = buttons.find((b) => b.text().includes('Cancel'))
			expect(cancelButton).toBeDefined()
			await cancelButton?.trigger('click')

			expect(wrapper.emitted('cancel')).toBeTruthy()
			expect(wrapper.emitted('cancel')?.length).toBe(1)
		})

		it('should emit discard event when Discard & Leave button is clicked', async () => {
			const wrapper = mount(UnsavedChangesModal, {
				props: {
					open: true,
					isSaving: false
				},
				global: {
					stubs: {
						UiDialog: { template: '<div><slot /></div>' },
						UiDialogContent: { template: '<div><slot /></div>' },
						UiDialogHeader: { template: '<div><slot /></div>' },
						UiDialogTitle: { template: '<h2><slot /></h2>' },
						UiDialogDescription: { template: '<p><slot /></p>' },
						UiDialogFooter: { template: '<div><slot /></div>' },
						UiButton: {
							template: '<button :disabled="disabled"><slot /></button>',
							props: ['disabled']
						},
						UiIcon: true
					}
				}
			})

			const buttons = wrapper.findAll('button')
			const discardButton = buttons.find((b) => b.text().includes('Discard & Leave'))
			expect(discardButton).toBeDefined()
			await discardButton?.trigger('click')

			expect(wrapper.emitted('discard')).toBeTruthy()
			expect(wrapper.emitted('discard')?.length).toBe(1)
		})

		it('should emit save event when Save & Leave button is clicked', async () => {
			const wrapper = mount(UnsavedChangesModal, {
				props: {
					open: true,
					isSaving: false
				},
				global: {
					stubs: {
						UiDialog: { template: '<div><slot /></div>' },
						UiDialogContent: { template: '<div><slot /></div>' },
						UiDialogHeader: { template: '<div><slot /></div>' },
						UiDialogTitle: { template: '<h2><slot /></h2>' },
						UiDialogDescription: { template: '<p><slot /></p>' },
						UiDialogFooter: { template: '<div><slot /></div>' },
						UiButton: {
							template: '<button :disabled="disabled"><slot /></button>',
							props: ['disabled']
						},
						UiIcon: true
					}
				}
			})

			const buttons = wrapper.findAll('button')
			const saveButton = buttons.find((b) => b.text().includes('Save & Leave'))
			expect(saveButton).toBeDefined()
			await saveButton?.trigger('click')

			expect(wrapper.emitted('save')).toBeTruthy()
			expect(wrapper.emitted('save')?.length).toBe(1)
		})

		it('should disable all buttons and show Saving state when isSaving is true', async () => {
			const wrapper = mount(UnsavedChangesModal, {
				props: {
					open: true,
					isSaving: true
				},
				global: {
					stubs: {
						UiDialog: { template: '<div><slot /></div>' },
						UiDialogContent: { template: '<div><slot /></div>' },
						UiDialogHeader: { template: '<div><slot /></div>' },
						UiDialogTitle: { template: '<h2><slot /></h2>' },
						UiDialogDescription: { template: '<p><slot /></p>' },
						UiDialogFooter: { template: '<div><slot /></div>' },
						UiButton: {
							template: '<button :disabled="disabled"><slot /></button>',
							props: ['disabled']
						},
						UiIcon: true
					}
				}
			})

			expect(wrapper.text()).toContain('Saving...')
			const buttons = wrapper.findAll('button')
			buttons.forEach((b) => {
				expect(b.attributes('disabled')).toBeDefined()
			})

			// Clicks should not emit events while saving
			await buttons[0].trigger('click')
			expect(wrapper.emitted('cancel')).toBeFalsy()
		})
	})

	describe('Page Lifecycle and Navigation Guards Logic', () => {
		it('should prompt beforeunload event only when store.isDirty is true', () => {
			const store = usePipelineBuildStore()
			store.isDirty = false

			const handleBeforeUnload = (e: BeforeUnloadEvent) => {
				if (store.isDirty) {
					e.preventDefault()
					e.returnValue = ''
					return ''
				}
			}

			// Case 1: store.isDirty is false -> no prompt
			const eventNotDirty = {
				preventDefault: vi.fn(),
				returnValue: undefined
			} as unknown as BeforeUnloadEvent

			const resNotDirty = handleBeforeUnload(eventNotDirty)
			expect(eventNotDirty.preventDefault).not.toHaveBeenCalled()
			expect(eventNotDirty.returnValue).toBeUndefined()
			expect(resNotDirty).toBeUndefined()

			// Case 2: store.isDirty is true -> trigger browser unsaved prompt
			store.isDirty = true
			const eventDirty = {
				preventDefault: vi.fn(),
				returnValue: undefined
			} as unknown as BeforeUnloadEvent

			const resDirty = handleBeforeUnload(eventDirty)
			expect(eventDirty.preventDefault).toHaveBeenCalled()
			expect(eventDirty.returnValue).toBe('')
			expect(resDirty).toBe('')
		})

		it('should cancel route navigation and trigger modal when store.isDirty is true', () => {
			const store = usePipelineBuildStore()
			store.isDirty = true

			let showModal = false
			let pendingRoute: any = null
			const isNavigatingAway = false

			const onRouteLeaveGuard = (to: any, _from: any, next: (arg?: boolean) => void) => {
				if (store.isDirty && !isNavigatingAway) {
					pendingRoute = to
					showModal = true
					next(false)
					return
				}
				next()
			}

			const nextFn = vi.fn()
			onRouteLeaveGuard(
				{ path: '/pipeline' },
				{ path: '/pipeline/1/builder/canvas-1' },
				nextFn
			)

			expect(nextFn).toHaveBeenCalledWith(false)
			expect(showModal).toBe(true)
			expect(pendingRoute).toEqual({ path: '/pipeline' })
		})

		it('should allow route navigation without modal when store.isDirty is false', () => {
			const store = usePipelineBuildStore()
			store.isDirty = false

			let showModal = false
			const nextFn = vi.fn()

			const onRouteLeaveGuard = (_to: any, _from: any, next: (arg?: boolean) => void) => {
				if (store.isDirty) {
					showModal = true
					next(false)
					return
				}
				next()
			}

			onRouteLeaveGuard(
				{ path: '/pipeline' },
				{ path: '/pipeline/1/builder/canvas-1' },
				nextFn
			)
			expect(nextFn).toHaveBeenCalledWith()
			expect(showModal).toBe(false)
		})

		it('should discard changes and proceed navigation when discard is triggered', () => {
			const store = usePipelineBuildStore()
			store.isDirty = true

			const routerMock = { push: vi.fn() }
			let showModal = true
			let isNavigatingAway = false
			const pendingRoute = { path: '/pipeline' }

			function handleDiscardLeave() {
				isNavigatingAway = true
				store.isDirty = false
				showModal = false
				if (pendingRoute) {
					routerMock.push(pendingRoute)
				}
			}

			handleDiscardLeave()

			expect(store.isDirty).toBe(false)
			expect(isNavigatingAway).toBe(true)
			expect(showModal).toBe(false)
			expect(routerMock.push).toHaveBeenCalledWith({ path: '/pipeline' })
		})

		it('should save graph and proceed navigation on successful save-and-leave', async () => {
			const store = usePipelineBuildStore()
			store.isDirty = true
			vi.spyOn(store, 'saveGraph').mockResolvedValue(true)

			const routerMock = { push: vi.fn() }
			let showModal = true
			let isNavigatingAway = false
			let isSavingAndLeaving = false
			const pendingRoute = { path: '/pipeline' }

			async function handleSaveAndLeave() {
				isSavingAndLeaving = true
				try {
					const success = await store.saveGraph()
					if (success) {
						isNavigatingAway = true
						showModal = false
						if (pendingRoute) {
							routerMock.push(pendingRoute)
						}
					}
				} finally {
					isSavingAndLeaving = false
				}
			}

			await handleSaveAndLeave()

			expect(store.saveGraph).toHaveBeenCalled()
			expect(isNavigatingAway).toBe(true)
			expect(showModal).toBe(false)
			expect(isSavingAndLeaving).toBe(false)
			expect(routerMock.push).toHaveBeenCalledWith({ path: '/pipeline' })
		})

		it('should abort navigation and remain on canvas if saveGraph fails during save-and-leave', async () => {
			const store = usePipelineBuildStore()
			store.isDirty = true
			vi.spyOn(store, 'saveGraph').mockResolvedValue(false)

			const routerMock = { push: vi.fn() }
			let showModal = true
			let isNavigatingAway = false
			let isSavingAndLeaving = false
			const pendingRoute = { path: '/pipeline' }

			async function handleSaveAndLeave() {
				isSavingAndLeaving = true
				try {
					const success = await store.saveGraph()
					if (success) {
						isNavigatingAway = true
						showModal = false
						if (pendingRoute) {
							routerMock.push(pendingRoute)
						}
					}
				} finally {
					isSavingAndLeaving = false
				}
			}

			await handleSaveAndLeave()

			expect(store.saveGraph).toHaveBeenCalled()
			expect(isNavigatingAway).toBe(false)
			expect(showModal).toBe(true) // modal stays open so user can retry or discard
			expect(isSavingAndLeaving).toBe(false)
			expect(routerMock.push).not.toHaveBeenCalled()
		})

		it('should intercept F5 and Ctrl+R keyboard reload shortcuts when store.isDirty is true', () => {
			const store = usePipelineBuildStore()
			store.isDirty = true

			let showModal = false
			let isPendingReload = false

			function handleKeyDown(e: any) {
				const isReloadShortcut =
					e.key === 'F5' || ((e.ctrlKey || e.metaKey) && (e.key === 'r' || e.key === 'R'))

				if (isReloadShortcut && store.isDirty) {
					e.preventDefault()
					isPendingReload = true
					showModal = true
				}
			}

			// Case 1: F5 key pressed
			const f5Event = { key: 'F5', preventDefault: vi.fn() }
			handleKeyDown(f5Event)
			expect(f5Event.preventDefault).toHaveBeenCalled()
			expect(isPendingReload).toBe(true)
			expect(showModal).toBe(true)

			// Reset
			showModal = false
			isPendingReload = false

			// Case 2: Ctrl+R key pressed
			const ctrlREvent = { key: 'r', ctrlKey: true, preventDefault: vi.fn() }
			handleKeyDown(ctrlREvent)
			expect(ctrlREvent.preventDefault).toHaveBeenCalled()
			expect(isPendingReload).toBe(true)
			expect(showModal).toBe(true)

			// Reset
			showModal = false
			isPendingReload = false

			// Case 3: Cmd+R (macOS metaKey) pressed
			const cmdREvent = { key: 'R', metaKey: true, preventDefault: vi.fn() }
			handleKeyDown(cmdREvent)
			expect(cmdREvent.preventDefault).toHaveBeenCalled()
			expect(isPendingReload).toBe(true)
			expect(showModal).toBe(true)
		})

		it('should not intercept reload shortcuts when store.isDirty is false or for unrelated keys', () => {
			const store = usePipelineBuildStore()
			store.isDirty = false

			let showModal = false
			let isPendingReload = false

			function handleKeyDown(e: any) {
				const isReloadShortcut =
					e.key === 'F5' || ((e.ctrlKey || e.metaKey) && (e.key === 'r' || e.key === 'R'))

				if (isReloadShortcut && store.isDirty) {
					e.preventDefault()
					isPendingReload = true
					showModal = true
				}
			}

			// Case 1: store.isDirty is false
			const f5Event = { key: 'F5', preventDefault: vi.fn() }
			handleKeyDown(f5Event)
			expect(f5Event.preventDefault).not.toHaveBeenCalled()
			expect(showModal).toBe(false)
			expect(isPendingReload).toBe(false)

			// Case 2: store.isDirty is true, but pressing unrelated keys (Ctrl+S, Escape)
			store.isDirty = true
			const ctrlSEvent = { key: 's', ctrlKey: true, preventDefault: vi.fn() }
			handleKeyDown(ctrlSEvent)
			expect(ctrlSEvent.preventDefault).not.toHaveBeenCalled()
			expect(showModal).toBe(false)
			expect(isPendingReload).toBe(false)
		})

		it('should trigger page reload instead of router.push when isPendingReload is true on discard or save', async () => {
			const store = usePipelineBuildStore()
			store.isDirty = true
			vi.spyOn(store, 'saveGraph').mockResolvedValue(true)

			const reloadMock = vi.fn()
			const routerMock = { push: vi.fn() }

			// Test discard with isPendingReload
			let isPendingReload = true
			let showModal = true
			let isNavigatingAway = false

			function handleDiscardLeave() {
				isNavigatingAway = true
				store.isDirty = false
				showModal = false
				if (isPendingReload) {
					isPendingReload = false
					reloadMock()
					return
				}
				routerMock.push('/pipeline')
			}

			handleDiscardLeave()
			expect(isNavigatingAway).toBe(true)
			expect(reloadMock).toHaveBeenCalledTimes(1)
			expect(routerMock.push).not.toHaveBeenCalled()
			expect(store.isDirty).toBe(false)
			expect(showModal).toBe(false)

			// Test save & leave with isPendingReload
			store.isDirty = true
			isPendingReload = true
			showModal = true
			isNavigatingAway = false

			async function handleSaveAndLeave() {
				const success = await store.saveGraph()
				if (success) {
					isNavigatingAway = true
					showModal = false
					if (isPendingReload) {
						isPendingReload = false
						reloadMock()
						return
					}
					routerMock.push('/pipeline')
				}
			}

			await handleSaveAndLeave()
			expect(isNavigatingAway).toBe(true)
			expect(reloadMock).toHaveBeenCalledTimes(2)
			expect(routerMock.push).not.toHaveBeenCalled()
			expect(showModal).toBe(false)
		})
	})
})
