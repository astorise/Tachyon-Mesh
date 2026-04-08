# Proposal: Change 036 - Data Plane Events (CDC & Smart Storage Proxies)

## Context
Triggering Serverless functions based on data mutations (Database inserts, S3 uploads) is a core requirement. However, directly triggering an HTTP call from a Relational Database Trigger (UDF) tightly couples the database transaction to the network availability, risking cascading rollbacks. For Object Storage, relying on the client or the business FaaS to dual-write and dispatch events mixes infrastructure concerns with business logic. 

## Objective
Implement robust, asynchronous Data Plane eventing using System FaaS:
1. **The Outbox / CDC Pattern (For RDBMS):** A System FaaS (`system-faas-cdc`) acts as a background poller. It reads from an `outbox_events` table (or WAL stream) in the database, dispatches the event to the Mesh, and safely deletes the row, completely decoupling the DB transaction from the FaaS execution.
2. **The Smart Proxy Pattern (For S3/NoSQL):** A System FaaS (`system-faas-s3-proxy`) acts as a reverse proxy for storage. It intercepts the user's `PUT` request, securely writes the file to the actual S3 bucket using Host capabilities, and then asynchronously dispatches an event to the Mesh before returning a 200 OK to the client.

## Scope
- Update `integrity.lock` schema to define proxy routes and database connection strings for system components.
- Develop the `system-faas-cdc` module using standard DB connection libraries compiled to WASM.
- Develop the `system-faas-s3-proxy` module using Outbound HTTP to communicate with the AWS S3 API and the internal Mesh IPC.

## Success Metrics
- A database `INSERT` commits instantly (e.g., < 2ms). The `system-faas-cdc` detects the outbox row within 1 second and triggers the business FaaS, ensuring eventual consistency without transaction blocking.
- A user uploads a file to the Tachyon Proxy. The file lands in the AWS S3 bucket, and an internal FaaS is immediately triggered with the file metadata, while the business developers only wrote standard file processing logic.