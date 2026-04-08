# Specifications: UDP Datagram Routing

## 1. UDP Port Mapping (`integrity.lock`)
The `layer4` configuration block is expanded to differentiate between TCP and UDP.

    {
        "host": {
            "layer4": {
                "tcp": {
                    "2222": "ssh-gateway"
                },
                "udp": {
                    "53": "dns-server",
                    "51820": "vpn-node"
                }
            }
        }
    }

## 2. The UDP WIT Interface
Instead of standard I/O, the FaaS must export a specific function to handle inbound packets.

    interface udp-handler {
        record datagram {
            target-ip: string,
            target-port: u16,
            payload: list<u8>,
        }

        export on-packet: func(source-ip: string, source-port: u16, payload: list<u8>) -> result<list<datagram>, string>;
    }

## 3. The Event-Driven Lifecycle
Because UDP is connectionless, it behaves very much like an HTTP request:
1. The Host's `UdpSocket` receives a packet.
2. The Host identifies the target FaaS bound to that port.
3. The Host claims an instance from the `InstancePool`.
4. The Host invokes the `on-packet` exported function.
5. The FaaS returns a list of packets to send back (which the Host immediately dispatches via the `UdpSocket`).
6. The FaaS instance is returned to the pool (Scale-to-Zero after idle timeout).