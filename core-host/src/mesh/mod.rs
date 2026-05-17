#![allow(dead_code)]

pub(crate) mod gossip {
    pub(crate) const MODULE: &str = "mesh::gossip";
}

pub(crate) mod migration;

pub(crate) mod overlay {
    pub(crate) const MODULE: &str = "mesh::overlay";
}
