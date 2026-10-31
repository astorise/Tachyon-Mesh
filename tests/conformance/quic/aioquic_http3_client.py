#!/usr/bin/env python3
"""Exercise Tachyon HTTP/3 with aioquic and a verified test CA."""
import argparse
import asyncio
import os
import ssl
import sys

from aioquic.asyncio.client import connect
from aioquic.asyncio.protocol import QuicConnectionProtocol
from aioquic.h3.connection import H3_ALPN, H3Connection
from aioquic.h3.events import DataReceived, HeadersReceived
from aioquic.quic.configuration import QuicConfiguration
from aioquic.quic.events import ConnectionTerminated, ProtocolNegotiated


class Http3Client(QuicConnectionProtocol):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.http = H3Connection(self._quic)
        self.responses = {}
        self.alpn = None

    async def get(self, authority, path):
        stream = self._quic.get_next_available_stream_id()
        future = asyncio.get_running_loop().create_future()
        self.responses[stream] = {"future": future, "headers": [], "body": bytearray()}
        self.http.send_headers(stream, [
            (b":method", b"GET"), (b":scheme", b"https"),
            (b":authority", authority.encode()), (b":path", path.encode()),
        ], end_stream=True)
        self.transmit()
        return await asyncio.wait_for(future, timeout=30)

    def quic_event_received(self, event):
        if os.getenv("TACHYON_INTEROP_DEBUG"):
            print(f"QUIC event: {event!r}", file=sys.stderr)
        if isinstance(event, ProtocolNegotiated):
            self.alpn = event.alpn_protocol
        if isinstance(event, ConnectionTerminated):
            for response in self.responses.values():
                if not response["future"].done():
                    response["future"].set_exception(ConnectionError(event.reason_phrase))
        for http_event in self.http.handle_event(event):
            if os.getenv("TACHYON_INTEROP_DEBUG"):
                print(f"HTTP/3 event: {http_event!r}", file=sys.stderr)
            response = self.responses.get(http_event.stream_id)
            if response is None:
                continue
            if isinstance(http_event, HeadersReceived):
                response["headers"].extend(http_event.headers)
            elif isinstance(http_event, DataReceived):
                response["body"].extend(http_event.data)
            if getattr(http_event, "stream_ended", False) and not response["future"].done():
                response["future"].set_result((response["headers"], bytes(response["body"])))


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--ca", required=True)
    parser.add_argument("--expected", required=True)
    args = parser.parse_args()
    configuration = QuicConfiguration(is_client=True, alpn_protocols=H3_ALPN, server_name="localhost")
    configuration.verify_mode = ssl.CERT_REQUIRED
    configuration.load_verify_locations(cafile=args.ca)
    async with connect("127.0.0.1", args.port, configuration=configuration,
                       create_protocol=Http3Client) as client:
        if client.alpn != "h3":
            raise RuntimeError(f"unexpected ALPN: {client.alpn!r}")
        responses = await asyncio.gather(*(
            client.get(f"localhost:{args.port}", "/api/guest-example") for _ in range(3)
        ))
        for headers, body in responses:
            if (b":status", b"200") not in headers or body.decode() != args.expected:
                raise RuntimeError(f"unexpected HTTP/3 response: {headers!r}, {body!r}")
    print("aioquic: verified TLS, negotiated h3 and received three concurrent responses")


if __name__ == "__main__":
    asyncio.run(main())
