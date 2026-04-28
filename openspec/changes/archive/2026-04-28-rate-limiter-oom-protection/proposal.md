# Proposal: Rate Limiter OOM Protection (Bounded LRU)

## Context
The `core-host` implements a Rate Limiter (`core-host/src/rate_limit.rs`) to protect the Mesh from DDoS attacks. Currently, it tracks request counts per source IP address. If this tracking mechanism relies on an unbounded data structure (like a standard `HashMap` or `DashMap`), it introduces a fatal vulnerability: an attacker spoofing millions of random source IP addresses will force the router to allocate memory for each fake IP, leading to an immediate Out-Of-Memory (OOM) crash.

## Proposed Solution
We will refactor the Rate Limiter to use a **Strictly Bounded LRU (Least Recently Used) Cache**. 
1. The cache will have a hard limit on the number of concurrent IPs it tracks (e.g., `100,000` entries).
2. When the capacity is reached, the oldest (least recently active) IP entries are automatically evicted to make room for new ones.
3. This guarantees that the memory footprint of the Rate Limiter remains strictly constant `O(1)`, regardless of the volume or diversity of incoming traffic.

## Objectives
- Eliminate the memory exhaustion (OOM) vector via IP spoofing.
- Maintain fast `O(1)` access and mutation times for legitimate traffic rate limiting.
- Provide a predictable and constant memory footprint for the `core-host` networking stack.