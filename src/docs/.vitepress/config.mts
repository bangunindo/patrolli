/*
 * SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
 *
 * SPDX-License-Identifier: AGPL-3.0-only
 */

import { defineConfig } from "vitepress";

// https://vitepress.dev/reference/site-config
export default defineConfig({
    title: "Patrolli",
    description: "Modernizing your legacy monitoring system",
    locales: {
        root: {
            label: "English",
            lang: "en",
        },
    },
    themeConfig: {
        // https://vitepress.dev/reference/default-theme-config
        nav: [
            { text: "Home", link: "/" },
            { text: "Docs", link: "/introduction/what-is-patrolli" },
        ],

        sidebar: [
            {
                text: "Introduction",
                collapsed: false,
                items: [
                    { text: "What is Patrolli?", link: "/introduction/what-is-patrolli" },
                    {
                        text: "Concepts",
                        link: "/introduction/concepts/",
                        collapsed: true,
                        items: [
                            { text: "Item", link: "/introduction/concepts/item" },
                            { text: "Listener", link: "/introduction/concepts/listener" },
                            { text: "Endpoint", link: "/introduction/concepts/endpoint" },
                            { text: "Host", link: "/introduction/concepts/host" },
                            { text: "Processor", link: "/introduction/concepts/processor" },
                            { text: "Sink", link: "/introduction/concepts/sink" },
                        ],
                    },
                    { text: "Architecture", link: "/introduction/architecture" },
                ],
            },
            {
                text: "Getting Started",
                collapsed: false,
                items: [
                    {
                        text: "Deployment Guides",
                        collapsed: true,
                        items: [
                            { text: "Single Node without Backend", link: "/getting-started/deployment/single-node-without-backend" },
                            { text: "Single Node with Backend", link: "/getting-started/deployment/single-node-with-backend" },
                            { text: "Clustering", link: "/getting-started/deployment/clustering" },
                            { text: "Multiple Cluster", link: "/getting-started/deployment/multiple-cluster" },
                            { text: "Authentication", link: "/getting-started/deployment/authentication" },
                            { text: "Rotate Secret Keys", link: "/getting-started/deployment/rotate-secret-keys" },
                        ],
                    },
                    { text: "Creating Your First Pipeline", link: "/getting-started/usage/creating-your-first-pipeline" },
                ],
            },
        ],
    },
    srcDir: "./en",
});
