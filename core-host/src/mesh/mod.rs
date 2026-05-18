pub(crate) mod gossip {
    #[cfg(feature = "experimental")]
    pub(crate) const MODULE: &str = "mesh::gossip";
}

pub(crate) mod migration;

pub(crate) mod overlay {
    #[cfg(feature = "experimental")]
    pub(crate) const MODULE: &str = "mesh::overlay";
}
