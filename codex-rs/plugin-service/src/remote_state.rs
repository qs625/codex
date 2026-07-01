#[derive(Debug, Clone, PartialEq)]
pub struct RemoteInstalledPlugin {
    pub marketplace_name: String,
    pub id: String,
    pub name: String,
    pub enabled: bool,
}
