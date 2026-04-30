#![no_std]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct L4Key {
    pub protocol: L4Protocol,
    pub destination_port: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum L4Protocol {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct L4Target {
    pub destination_ipv4: [u8; 4],
    pub destination_port: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketRewrite {
    pub key: L4Key,
    pub target: L4Target,
}

impl PacketRewrite {
    pub fn new(protocol: L4Protocol, destination_port: u16, target: L4Target) -> Self {
        Self {
            key: L4Key {
                protocol,
                destination_port,
            },
            target,
        }
    }
}

pub fn lookup_rewrite<'a>(
    protocol: L4Protocol,
    destination_port: u16,
    rules: &'a [PacketRewrite],
) -> Option<&'a L4Target> {
    rules
        .iter()
        .find(|rule| rule.key.protocol == protocol && rule.key.destination_port == destination_port)
        .map(|rule| &rule.target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_rewrite_matches_protocol_and_port() {
        let rules = [PacketRewrite::new(
            L4Protocol::Tcp,
            8080,
            L4Target {
                destination_ipv4: [10, 0, 0, 5],
                destination_port: 9000,
            },
        )];

        assert_eq!(
            lookup_rewrite(L4Protocol::Tcp, 8080, &rules),
            Some(&rules[0].target)
        );
        assert_eq!(lookup_rewrite(L4Protocol::Udp, 8080, &rules), None);
    }
}
