# Proposal: Geo-Pinning & Dynamic Data Locality

## Why
Tachyon-Mesh distributes compute (FaaS) dynamically across edge nodes. However, if the underlying RedDB Key-Value pairs remain statically provisioned on their original ingestion nodes, compute instances will suffer from severe cross-region network latency (e.g., a user in Europe fetching data stored in the US).

1. **Speed of Light Bottleneck:** Cross-continental network calls impose a hard physical floor on minimum latency (often > 100ms).
2. **Bandwidth Costs:** Repeatedly transferring the same records across the mesh backbone incurs unnecessary egress/ingress costs and network congestion.
3. **Static Sharding:** Traditional databases use static sharding keys (e.g., `tenant_id % node_count`), which do not adapt to nomadic users or changing access patterns.

## What Changes
Implement an autonomous "Geo-Pinning" engine within the core-host that migrates RedDB data shards closer to their center of gravity (the nodes requesting them most frequently).
1. **Access Telemetry:** The core-host maintains a lightweight, probabilistic tracker (e.g., Count-Min Sketch) recording the `peer-id` origin of read requests for specific key prefixes (Subspaces).
2. **Threshold Trigger:** If a specific subspace is heavily requested by a remote peer and rarely accessed locally, the host initiates a shard migration.
3. **Zero-Downtime Migration:** The data is asynchronously replicated to the target node via QUIC. Once synced, the mesh's Gossip routing table is updated to point to the new primary node, and the original copy is either downgraded to a read-replica or purged.

## Impact
- **Microsecond Latency:** Hot data is pinned locally on the NVMe of the node executing the FaaS, dropping retrieval latency to virtually zero.
- **Self-Optimizing Topology:** The mesh organically adapts to user movement (e.g., a user traveling from Europe to Asia will have their profile data "follow" them across the mesh after a few reads).
