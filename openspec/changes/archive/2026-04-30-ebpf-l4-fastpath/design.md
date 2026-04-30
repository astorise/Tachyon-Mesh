# Design: Aya-rs Integration



## 1. The eBPF Probe (`ebpf-probes/src/l4_router.rs`)
Compiled separately to `bpfel-unknown-none`.
```rust
#[xdp(name="tachyon_l4_router")]
pub fn tachyon_l4_router(ctx: XdpContext) -> u32 {
    let packet = ctx.data();
    let port = extract_dst_port(packet);
    
    // Lookup in the shared kernel map
    if let Some(target) = ROUTING_TABLE.get(&port) {
        rewrite_packet_headers(packet, target.ip, target.port);
        return xdp_action::XDP_TX; // Bounce straight back out the NIC
    }
    xdp_action::XDP_PASS // Send to Tachyon userspace if no rule matches
}
```

## 2. The Host Control Plane (`core-host/src/network/ebpf.rs`)
Tachyon loads the probe and updates the map.
```rust
use aya::Bpf;

pub fn init_ebpf(interface: &str, routes: &HashMap<u16, SocketAddr>) -> Result<()> {
    let mut bpf = Bpf::load(include_bytes_aligned!(...))?;
    let program: &mut Xdp = bpf.program_mut("tachyon_l4_router").unwrap().try_into()?;
    program.attach(interface, XdpFlags::default())?;
    
    // Sync the routing table into the kernel
    update_ebpf_maps(&mut bpf, routes)?;
    Ok(())
}
```