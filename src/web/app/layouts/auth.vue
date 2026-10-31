<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appConstant from '@@/shared/constants/app.constant'
</script>

<template>
	<div
		class="auth-layout min-h-screen w-full overflow-x-hidden bg-background text-foreground flex items-center justify-center"
	>
		<div
			id="main-content"
			class="relative min-h-screen w-full bg-[#F8FAFC] dark:bg-slate-950 overflow-x-hidden flex items-center justify-center selection:bg-sky-500 selection:text-white"
		>
			<!-- Network Topology Mesh from Figma -->
			<div
				class="absolute inset-0 pointer-events-none bg-center bg-no-repeat bg-cover opacity-10 sm:opacity-12"
				style="background-image: url('/images/network-topology-mesh.svg')"
				aria-hidden="true"
			></div>

			<!-- Ambient Radial Glow -->
			<div
				class="absolute pointer-events-none w-[340px] h-[240px] sm:w-[600px] sm:h-[400px] lg:w-[800px] lg:h-[550px] rounded-full bg-primary opacity-[0.14] filter blur-[60px] sm:blur-[70px]"
				aria-hidden="true"
			></div>

			<!-- Concentric Radial Rings & Ambient Glow Nodes -->
			<div
				class="absolute inset-0 pointer-events-none flex items-center justify-center overflow-hidden"
				aria-hidden="true"
			>
				<!-- Radial Ring 1 -->
				<div
					class="absolute rounded-[50%] border border-primary opacity-[0.10] w-[340px] h-[220px] sm:w-[520px] sm:h-[340px] lg:w-[672px] lg:h-[420px]"
				></div>
				<!-- Radial Ring 2 -->
				<div
					class="absolute rounded-[50%] border border-primary opacity-[0.08] hidden sm:block sm:w-[760px] sm:h-[480px] lg:w-[992px] lg:h-[620px]"
				></div>
				<!-- Radial Ring 3 -->
				<div
					class="absolute rounded-[50%] border border-primary opacity-[0.08] hidden lg:block lg:w-[1312px] lg:h-[820px]"
				></div>

				<!-- Glow Node: Left Center -->
				<div
					class="absolute hidden sm:block w-2.5 h-2.5 rounded-full bg-primary shadow-[0_0_18px_rgba(37,99,235,0.45)] -translate-x-[360px] -translate-y-[130px] lg:-translate-x-[480px] lg:-translate-y-[150px]"
				></div>
				<!-- Glow Node: Right Center -->
				<div
					class="absolute hidden sm:block w-2.5 h-2.5 rounded-full bg-primary shadow-[0_0_18px_rgba(37,99,235,0.45)] translate-x-[360px] -translate-y-[130px] lg:translate-x-[480px] lg:-translate-y-[150px]"
				></div>
				<!-- Glow Node: Bottom Left -->
				<div
					class="absolute hidden sm:block w-2.5 h-2.5 rounded-full bg-primary shadow-[0_0_18px_rgba(37,99,235,0.45)] -translate-x-[340px] translate-y-[160px] lg:-translate-x-[460px] lg:translate-y-[200px]"
				></div>
				<!-- Glow Node: Bottom Right -->
				<div
					class="absolute hidden sm:block w-2.5 h-2.5 rounded-full bg-primary shadow-[0_0_18px_rgba(37,99,235,0.45)] translate-x-[340px] translate-y-[160px] lg:translate-x-[460px] lg:translate-y-[200px]"
				></div>
				<!-- Light Glow Node Top -->
				<div
					class="absolute hidden lg:block w-2 h-2 rounded-full bg-accent shadow-[0_0_10px_rgba(37,99,235,0.3)] -translate-x-[160px] -translate-y-[290px]"
				></div>
				<!-- Light Glow Node Bottom -->
				<div
					class="absolute hidden lg:block w-2 h-2 rounded-full bg-[#E0F2FE] shadow-[0_0_10px_rgba(37,99,235,0.3)] translate-x-[220px] translate-y-[290px]"
				></div>
			</div>

			<!-- Page Slot Content -->
			<div class="relative z-10 w-full flex items-center justify-center p-4 sm:p-6 lg:p-8">
				<slot />
			</div>

			<a
				:href="appConstant.sourceUrl"
				target="_blank"
				rel="noopener"
				class="absolute bottom-4 z-10 text-xs text-muted-foreground hover:text-foreground"
			>
				Source Code
			</a>
		</div>
	</div>
</template>
