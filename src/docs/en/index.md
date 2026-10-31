---
# https://vitepress.dev/reference/default-theme-home-page
layout: home

hero:
    name: "Patrolli"
    text: "Modernizing your legacy monitoring system"
    actions:
        - theme: brand
          text: Get Started
          link: /getting-started/deployment/single-node-without-backend
        - theme: alt
          text: What is Patrolli?
          link: /introduction/what-is-patrolli

features:
    - icon: 📡
      title: Listen and poll
      details: Receive syslog, NetFlow, sFlow, SNMP traps, and OpenTelemetry. Poll devices with SNMP and Prometheus scrape.
    - icon: 🔀
      title: Flexible pipelines
      details: Filter, enrich, and transform data with processors. Send one stream to many destinations.
    - icon: 🚚
      title: Deliver anywhere
      details: Send data to a file, Kafka, ClickHouse, or any OpenTelemetry endpoint.
---
