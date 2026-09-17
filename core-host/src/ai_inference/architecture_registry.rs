use anyhow::Result;

use super::modelopt_nvfp4::ModelOptNvfp4Directory;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArchitectureKind {
    Qwen35MoeText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArchitectureMatch {
    pub(crate) kind: ArchitectureKind,
    pub(crate) profile: &'static str,
}

pub(crate) trait ArchitectureDescriptor: Sync {
    fn inspect(&self, model: &ModelOptNvfp4Directory) -> Result<Option<ArchitectureMatch>>;
}

pub(crate) struct ArchitectureRegistry {
    descriptors: &'static [&'static dyn ArchitectureDescriptor],
}

impl ArchitectureRegistry {
    pub(crate) const fn new(descriptors: &'static [&'static dyn ArchitectureDescriptor]) -> Self {
        Self { descriptors }
    }

    pub(crate) fn resolve(
        &self,
        model: &ModelOptNvfp4Directory,
    ) -> Result<Option<ArchitectureMatch>> {
        for descriptor in self.descriptors {
            if let Some(matched) = descriptor.inspect(model)? {
                return Ok(Some(matched));
            }
        }
        Ok(None)
    }
}
