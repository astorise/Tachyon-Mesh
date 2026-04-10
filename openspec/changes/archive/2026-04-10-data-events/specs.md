# Specifications: Data Events Architecture

## 1. Schema Update v11 (integrity.lock)
Define the system modules responsible for handling data streams.

    {
        "targets": [
            {
                "name": "db-outbox-poller",
                "module": "system-faas-cdc.wasm",
                "role": "system",
                "env": {
                    "DB_URL": "postgres://user:pass@host/db",
                    "OUTBOX_TABLE": "events_outbox",
                    "TARGET_ROUTE": "http://mesh/process-db-event"
                }
            },
            {
                "name": "s3-upload-proxy",
                "module": "system-faas-s3-proxy.wasm",
                "role": "system",
                "env": {
                    "REAL_S3_BUCKET": "https://my-bucket.s3.amazonaws.com",
                    "TARGET_ROUTE": "http://mesh/on-image-upload"
                }
            }
        ]
    }

## 2. The CDC / Outbox Component
Compiled as a background Reactor or Command.
- Connects to the database using the provided `DB_URL`.
- Executes `SELECT id, payload FROM events_outbox ORDER BY created_at ASC LIMIT 50 FOR UPDATE SKIP LOCKED` (to ensure safe concurrent polling).
- For each row, makes a `POST` to `TARGET_ROUTE`.
- If the Mesh returns HTTP 200, executes `DELETE FROM events_outbox WHERE id = ?`.
- This ensures "At-Least-Once" delivery without blocking the main application transactions.

## 3. The S3 Smart Proxy Component
Compiled as an HTTP Reactor intercepting external traffic.
- Intercepts a `PUT /upload/filename.jpg` request.
- Uses the `tachyon:http/outbound` capability to forward the raw bytes to the actual S3 bucket, signing the request with AWS V4 signatures (handled securely by the System FaaS).
- Upon receiving a 200 OK from AWS S3, it makes an internal `POST` to the `TARGET_ROUTE` with JSON metadata: `{"file": "filename.jpg", "size": 1024}`.
- Returns 200 OK to the original client.