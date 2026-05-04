# Design: Storage & State Data Model

## 1. The GitOps YAML Specification
This file configures the data layer of the Edge node.

    api_version: storage.tachyon.io/v1alpha1
    kind: StorageAndState
    metadata:
      name: edge-global-storage
      environment: production
      
    spec:
      # 1. WASI VOLUMES (Local persistent or ephemeral storage)
      volumes:
        - name: "ai-models-cache"
          type: local_disk
          host_path: "/var/lib/tachyon/models"
          guest_path: "/models"
          read_only: true
          garbage_collection:
            enabled: true
            max_size_mb: 10240 # 10 GB
            
        - name: "tmp-processing"
          type: memory_tmpfs
          guest_path: "/tmp"
          max_size_mb: 512

      # 2. S3 BACKENDS (Object Storage routing)
      s3_backends:
        - name: "corporate-blob-store"
          endpoint: "https://s3.eu-west-1.amazonaws.com"
          bucket: "tachyon-edge-sync-prod"
          region: "eu-west-1"
          # Credentials are not here. They are resolved via identity/secrets manager.

      # 3. TURBOQUANT KV (Distributed Embedded Database)
      kv_partitions:
        - name: "rate-limit-counters"
          persistence: memory_only
          replication_factor: 3 # Syncs to 3 local peers via Gossip
          
        - name: "edge-auth-sessions"
          persistence: disk_backed
          sync_to_s3_backend_ref: "corporate-blob-store"

## 2. The WIT Contract (`wit/config-storage.wit`)
This interface safely validates storage intents before mounting physical resources.

    interface config-storage {
        enum volume-type { local-disk, memory-tmpfs }
        enum kv-persistence { memory-only, disk-backed }

        record volume-gc-policy {
            enabled: bool,
            max-size-mb: u32,
        }

        record wasi-volume {
            name: string,
            vol-type: volume-type,
            host-path: option<string>,
            guest-path: string,
            read-only: bool,
            gc-policy: option<volume-gc-policy>,
        }

        record s3-backend {
            name: string,
            endpoint: string,
            bucket: string,
            region: string,
        }

        record kv-partition {
            name: string,
            persistence: kv-persistence,
            replication-factor: u8,
            sync-to-s3-ref: option<string>,
        }

        record storage-configuration {
            volumes: list<wasi-volume>,
            s3-backends: list<s3-backend>,
            kv-partitions: list<kv-partition>,
        }

        /// Validation (Zero-Panic)
        validate-storage-config: func(config: storage-configuration) -> result<_, string>;

        /// CRUD Operations for Tachyon-UI / MCP
        get-storage-config: func() -> result<storage-configuration, string>;
        apply-wasi-volume: func(vol: wasi-volume) -> result<_, string>;
        apply-s3-backend: func(s3: s3-backend) -> result<_, string>;
        apply-kv-partition: func(kv: kv-partition) -> result<_, string>;
    }