#![allow(dead_code)]

pub(crate) mod layer4 {
    pub(crate) const MODULE: &str = "network::layer4";
}

pub(crate) mod layer7 {
    pub(crate) const MODULE: &str = "network::layer7";
}

pub(crate) mod http3 {
    pub(crate) const MODULE: &str = "network::http3";
}

pub(crate) mod ebpf {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum EbpfFastPathStatus {
        Unsupported,
        NoRules,
    }

    pub(crate) fn init_ebpf_fastpath(route_count: usize) -> Result<EbpfFastPathStatus, String> {
        if route_count == 0 {
            return Ok(EbpfFastPathStatus::NoRules);
        }

        #[cfg(target_os = "linux")]
        {
            Err("eBPF/XDP fast-path requires an aya loader build; falling back to userspace L4 routing".to_owned())
        }

        #[cfg(not(target_os = "linux"))]
        {
            Ok(EbpfFastPathStatus::Unsupported)
        }
    }
}
