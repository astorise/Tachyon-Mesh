use super::*;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PeerPressureState {
    #[default]
    Idle,
    Caution,
    Saturated,
}
