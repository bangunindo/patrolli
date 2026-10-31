---
description: A sink sends data to its final destination, such as a file, Kafka, ClickHouse, or an OpenTelemetry endpoint.
---

# Sink

A sink is where data leaves Patrolli. It takes the output of one or more [processors](./processor) and delivers it to a destination such as a database, a message queue, or another collector.

## Sink types

Patrolli has four sink types.

- `file` appends records to a file. The path can also be `stdout` or `stderr`, which helps while you build a pipeline, or `null`, which discards the data. The default format is OTLP JSON, and other JSON layouts and OTLP protobuf are also available.
- `kafka` sends records to a Kafka topic. You choose the record format, the compression, how many brokers must confirm a write, and whether the message key is the item, the host, or the listener.
- `open_telemetry` sends records over gRPC as OTLP or OTAP, to an OpenTelemetry collector or to the `open_telemetry` listener of another Patrolli node. With several destinations, it can send to all of them, rotate between them, use one and fall back to the next, or split the data between them by a key such as the host.
- `clickhouse` inserts records into ClickHouse over HTTP. The tables have the same layout as the tables of the ClickHouse exporter in the OpenTelemetry Collector, so dashboards built for that exporter also work with Patrolli data.

## What a sink receives

The `inputs` field of a sink lists the processors whose output it receives. A sink can read from many processors, and many sinks can read from the same processor. Each sink gets its own copy.

A sink with no inputs is a catch-all. It receives data from items with an empty `processors` list, and listener records that match no item. If no catch-all sink exists, Patrolli drops that data and counts it in its internal metrics. A catch-all sink that writes to a file is a good way to find the records that your items do not match yet.

This sink writes the output of one processor to standard output:

```yaml
sinks:
  - id: 01a09ddf-933d-70a2-bfba-4b1d246946b1
    name: debug-output
    type: file
    inputs:
      - 01a05abf-25ff-70a2-89df-0eb8f1ab10aa
    config:
      path: stdout
      format: json_nested_denormalized
```

## Batches, queues, and retries

A sink collects records into batches and sends one batch at a time. A batch goes out when it reaches `batch_size` records or after `batch_timeout` milliseconds, and both default to 1000. The `worker_count` field sets how many batches the sink sends at the same time, because each worker has one batch in flight.

The `channel` field sets the queue in front of the sink, and the default is a `memory` queue. When the queue is full, Patrolli waits, and the processors in front of the sink slow down. Set `non_blocking: true` to drop data instead of waiting.

A `disk` channel keeps its queue on disk. It can hold a much larger backlog, which matters when a destination is down for a long time. With `write_through: true`, Patrolli writes all data to disk before it accepts the data, so a crash loses nothing that Patrolli accepted. The cost is a disk write for all data, even when the destination keeps up.

The `retry` field sets what happens when a destination cannot take a batch, for example because it is down or overloaded. The worker sends the same batch again. The wait between attempts starts at `initial_interval` and grows by `multiplier` up to `max_interval`. By default, Patrolli retries forever. Set `max_elapsed` in milliseconds to give up on a batch after that long. When the destination rejects a batch for good, for example because a message is too large, Patrolli drops the batch and does not retry it.
