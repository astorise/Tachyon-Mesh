# Specifications: UDS & Local Discovery Architecture

## 1. The Discovery Directory
Tachyon hosts on the same node must share a common directory (e.g., mounted via K8s hostPath or emptyDir).
- At startup, each host generates a unique HostID.
- It creates a Unix Domain Socket at `/var/run/tachyon/peers/<HostID>.sock`.
- It writes a small metadata file `<HostID>.json` containing its network IP and supported protocols.

## 2. Fast-Path Connection Logic
When a FaaS wants to talk to a peer at `10.0.0.5`:
1. The Mesh Router scans the `/var/run/tachyon/peers/` directory.
2. It looks for a metadata JSON file where the `ip` matches `10.0.0.5`.
3. If found:
   - It attempts to connect to the corresponding `.sock` file.
   - If the connection succeeds, it uses this UDS stream for the mTLS/H2 handshake.
4. If not found or connection fails:
   - It initiates a standard TCP connection to `10.0.0.5:PORT`.

## 3. Security & Permissions
- UDS socket files must have restricted permissions (0660) to ensure only the Tachyon service user can communicate.
- Even over UDS, the mTLS handshake (Change 029) should still be performed to ensure identity verification between the two instances, though encryption overhead is negligible on a local socket.