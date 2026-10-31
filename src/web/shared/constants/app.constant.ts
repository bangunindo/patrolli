// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export default {
	// AGPL-3.0 section 13: users interacting over the network must be offered
	// the source. Point this at your fork if you run a modified version.
	sourceUrl: 'https://github.com/bangunindo/patrolli',
	colors: ['primary', 'secondary', 'tertiary', 'info', 'success', 'error', 'warning'],
	chartDarkMode: {
		version: 1,
		themeName: 'chart-dark',
		theme: {
			backgroundColor: '#000',
			color: ['#1abc9c', '#3498db', '#9b59b6', '#f1c40f', '#e67e22', '#e74c3c'], // Custom color palette
			titleColor: 'var(--b-color-white)', // Light title color
			markTextColor: '#000',
			axisLine: {
				lineStyle: {
					color: 'var(--b-color-white)' // Axis line color
				}
			},
			axisLabel: {
				textStyle: {
					color: 'var(--b-color-white)' // Axis label color
				}
			},
			splitLine: {
				lineStyle: {
					color: 'var(--b-color-white)' // Split line color
				}
			}
			// Add more style properties as needed
		}
	},
	menuNav: [
		{
			key: 'pipeline',
			label: 'Pipeline',
			to: '/pipeline'
		},
		{
			key: 'configuration',
			label: 'Configuration',
			to: '/configuration'
		},
		{
			key: 'cluster',
			label: 'Cluster Monitoring',
			to: '/cluster'
		},
		{
			key: 'user-management',
			label: 'User Management',
			to: '/user-management'
		}
	],
	menuNavConfiguration: [
		{
			key: 'host',
			label: 'Hosts',
			to: '/configuration/host'
		},
		{
			key: 'endpoint',
			label: 'Endpoints',
			to: '/configuration/endpoint'
		},
		{
			key: 'listener',
			label: 'Listeners',
			to: '/configuration/listener'
		},
		{
			key: 'item',
			label: 'Items',
			to: '/configuration/item'
		},
		{
			key: 'sink',
			label: 'Sinks',
			to: '/configuration/sink'
		},
		{
			key: 'template',
			label: 'Templates',
			to: '/configuration/template'
		}
	]
}
