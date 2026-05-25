use crate::manifest::ProviderProtocolFamily;
use crate::registry::AgentIdentity;

pub struct ProviderConfig {
    pub provider_name: String,
    pub api_key: String, // Plaintext, decrypted by the Vault before use
}

#[derive(Clone, Debug)]
pub struct ProviderRoute {
    pub provider_name: String,
    pub model: String,
    pub protocol_family: ProviderProtocolFamily,
}

pub struct Router;

impl Router {
    pub fn new() -> Self {
        Self
    }

    /// Resolve the specific Provider and Model for a given Agent,
    /// allowing for dynamic overrides by essentially returning a tuple.
    pub fn resolve_route(
        &self,
        agent: &AgentIdentity,
        override_model: Option<&str>,
    ) -> ProviderRoute {
        let model = match override_model {
            Some(m) => m.to_string(),
            None => agent.default_model.clone(),
        };

        ProviderRoute {
            provider_name: agent.default_provider.clone(),
            model,
            protocol_family: ProviderProtocolFamily::from_provider_name(&agent.default_provider),
        }
    }
}
