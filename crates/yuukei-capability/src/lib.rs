use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use yuukei_protocol::{
    CapabilityInvocation, CapabilityRouteSummary, ExecutionLocation, ExtensionHealth, JsonMap,
};

pub const SPEECH_SYNTHESIS_CAPABILITY: &str = "speech.synthesis";
pub const DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID: &str = "yuukei.default-tts";

#[derive(Debug, Error)]
pub enum CapabilityError {
    #[error("capability extension already registered: {0}")]
    DuplicateExtension(String),
    #[error("capability extension must declare at least one capability: {0}")]
    EmptyCapabilities(String),
    #[error("unknown capability: {0}")]
    UnknownCapability(String),
    #[error("no healthy extension for capability: {0}")]
    NoHealthyExtension(String),
    #[error("missing permission {permission} for capability extension {extension_id}")]
    MissingPermission {
        extension_id: String,
        permission: String,
    },
    #[error("capability extension invocation failed: {0}")]
    Extension(String),
}

pub type Result<T> = std::result::Result<T, CapabilityError>;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegistration {
    pub extension_id: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
    pub location: ExecutionLocation,
    pub health: ExtensionHealth,
    pub enabled: bool,
    #[serde(default)]
    pub config_schema: JsonMap,
}

impl ProviderRegistration {
    pub fn summary(&self) -> CapabilityRouteSummary {
        CapabilityRouteSummary {
            extension_id: self.extension_id.clone(),
            capabilities: self.capabilities.clone(),
            location: self.location.clone(),
            health: self.health.clone(),
            enabled: self.enabled,
        }
    }

    fn is_healthy_for(&self, capability: &str) -> bool {
        self.enabled
            && self.health == ExtensionHealth::Ready
            && self
                .capabilities
                .iter()
                .any(|declared| declared == capability)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityResult {
    pub invocation_id: String,
    pub extension_id: String,
    pub capability: String,
    pub output: JsonMap,
    #[serde(default)]
    pub metadata: JsonMap,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventLogReadGrant {
    pub extension_id: String,
    pub resident_id: String,
    #[serde(default)]
    pub event_types: Vec<String>,
    #[serde(default)]
    pub privacy_categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_after_sequence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_timestamp: Option<String>,
    pub max_records: usize,
    pub allow_payloads: bool,
    pub allow_references: bool,
    pub expires_at: String,
    pub purpose: String,
}

#[async_trait]
pub trait CapabilityProvider: Send + Sync {
    fn registration(&self) -> ProviderRegistration;
    async fn invoke(&self, invocation: CapabilityInvocation) -> Result<CapabilityResult>;
}

#[derive(Clone, Default)]
pub struct CapabilityRouter {
    providers: BTreeMap<String, Arc<dyn CapabilityProvider>>,
    registrations: BTreeMap<String, ProviderRegistration>,
    defaults: BTreeMap<String, String>,
    permission_grants: BTreeMap<String, BTreeSet<String>>,
}

impl CapabilityRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P>(&mut self, provider: P) -> Result<()>
    where
        P: CapabilityProvider + 'static,
    {
        let registration = provider.registration();
        if registration.capabilities.is_empty() {
            return Err(CapabilityError::EmptyCapabilities(
                registration.extension_id,
            ));
        }
        if self.providers.contains_key(&registration.extension_id) {
            return Err(CapabilityError::DuplicateExtension(
                registration.extension_id,
            ));
        }
        let extension_id = registration.extension_id.clone();
        let capabilities = registration.capabilities.clone();
        self.providers
            .insert(extension_id.clone(), Arc::new(provider));
        self.registrations
            .insert(extension_id.clone(), registration);
        for capability in capabilities {
            self.defaults
                .entry(capability)
                .or_insert_with(|| extension_id.clone());
        }
        Ok(())
    }

    pub fn grant_permission(
        &mut self,
        extension_id: impl Into<String>,
        permission: impl Into<String>,
    ) {
        self.permission_grants
            .entry(extension_id.into())
            .or_default()
            .insert(permission.into());
    }

    pub fn set_default_extension(
        &mut self,
        capability: impl Into<String>,
        extension_id: impl Into<String>,
    ) {
        self.defaults.insert(capability.into(), extension_id.into());
    }

    pub fn summaries(&self) -> BTreeMap<String, CapabilityRouteSummary> {
        self.registrations
            .iter()
            .map(|(id, registration)| (id.clone(), registration.summary()))
            .collect()
    }

    pub fn has_healthy_provider(&self, capability: &str) -> bool {
        self.registrations
            .values()
            .any(|registration| registration.is_healthy_for(capability))
    }

    pub async fn invoke(&self, invocation: CapabilityInvocation) -> Result<CapabilityResult> {
        let extension_id = self.select_provider(&invocation.capability)?;
        let registration = self
            .registrations
            .get(&extension_id)
            .ok_or_else(|| CapabilityError::UnknownCapability(invocation.capability.clone()))?;
        self.ensure_permissions(registration)?;
        let provider = self
            .providers
            .get(&extension_id)
            .ok_or_else(|| CapabilityError::UnknownCapability(invocation.capability.clone()))?;
        provider.invoke(invocation).await
    }

    fn select_provider(&self, capability: &str) -> Result<String> {
        if let Some(extension_id) = self.defaults.get(capability) {
            if self
                .registrations
                .get(extension_id)
                .is_some_and(|registration| registration.is_healthy_for(capability))
            {
                return Ok(extension_id.clone());
            }
        }

        let mut found_capability = false;
        for registration in self.registrations.values() {
            if registration
                .capabilities
                .iter()
                .any(|declared| declared == capability)
            {
                found_capability = true;
                if registration.is_healthy_for(capability) {
                    return Ok(registration.extension_id.clone());
                }
            }
        }

        if found_capability {
            Err(CapabilityError::NoHealthyExtension(capability.to_string()))
        } else {
            Err(CapabilityError::UnknownCapability(capability.to_string()))
        }
    }

    fn ensure_permissions(&self, registration: &ProviderRegistration) -> Result<()> {
        let grants = self.permission_grants.get(&registration.extension_id);
        for permission in &registration.required_permissions {
            if !grants.is_some_and(|grants| grants.contains(permission)) {
                return Err(CapabilityError::MissingPermission {
                    extension_id: registration.extension_id.clone(),
                    permission: permission.clone(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct StubSpeechSynthesisProvider;

#[async_trait]
impl CapabilityProvider for StubSpeechSynthesisProvider {
    fn registration(&self) -> ProviderRegistration {
        ProviderRegistration {
            extension_id: DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID.to_string(),
            capabilities: vec![SPEECH_SYNTHESIS_CAPABILITY.to_string()],
            methods: vec!["synthesize".to_string()],
            required_permissions: Vec::new(),
            location: ExecutionLocation::ResidentHome,
            health: ExtensionHealth::Ready,
            enabled: true,
            config_schema: JsonMap::new(),
        }
    }

    async fn invoke(&self, invocation: CapabilityInvocation) -> Result<CapabilityResult> {
        let text = invocation
            .input
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let display_command_id = invocation
            .input
            .get("displayCommandId")
            .and_then(Value::as_str)
            .unwrap_or("display-command");
        let duration_ms = (text.chars().count() as u64).max(1) * 90;

        Ok(CapabilityResult {
            invocation_id: invocation.id,
            extension_id: DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID.to_string(),
            capability: SPEECH_SYNTHESIS_CAPABILITY.to_string(),
            output: JsonMap::from([
                (
                    "speechRef".to_string(),
                    json!(format!("yuukei-default-tts://{display_command_id}")),
                ),
                ("durationMs".to_string(), json!(duration_ms)),
                (
                    "segments".to_string(),
                    json!([{ "startMs": 0, "endMs": duration_ms, "text": text }]),
                ),
                ("visemes".to_string(), json!([])),
            ]),
            metadata: JsonMap::from([("binaryAudio".to_string(), json!(false))]),
        })
    }
}

#[cfg(test)]
mod tests {
    use yuukei_protocol::{new_id, CapabilityInvocation};

    use super::*;

    #[tokio::test]
    async fn registry_routes_to_stub_speech_provider() -> Result<()> {
        let mut router = CapabilityRouter::new();
        router.register(StubSpeechSynthesisProvider)?;

        let result = router
            .invoke(CapabilityInvocation {
                id: new_id("cap"),
                capability: "speech.synthesis".to_string(),
                method: "synthesize".to_string(),
                resident_id: "resident-default".to_string(),
                actor_id: Some("yuukei".to_string()),
                input: JsonMap::from([
                    ("text".to_string(), json!("hello")),
                    ("displayCommandId".to_string(), json!("cmd_1")),
                ]),
                context: None,
            })
            .await?;

        assert_eq!(result.extension_id, "yuukei.default-tts");
        assert_eq!(result.output["speechRef"], "yuukei-default-tts://cmd_1");
        Ok(())
    }

    #[tokio::test]
    async fn router_rejects_unknown_capability() -> Result<()> {
        let router = CapabilityRouter::new();
        let error = router
            .invoke(CapabilityInvocation {
                id: new_id("cap"),
                capability: "dialogue.generate".to_string(),
                method: "generate".to_string(),
                resident_id: "resident-default".to_string(),
                actor_id: None,
                input: JsonMap::new(),
                context: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, CapabilityError::UnknownCapability(_)));
        Ok(())
    }

    #[derive(Clone)]
    struct PermissionedProvider;

    #[async_trait]
    impl CapabilityProvider for PermissionedProvider {
        fn registration(&self) -> ProviderRegistration {
            ProviderRegistration {
                extension_id: "memory-extension".to_string(),
                capabilities: vec!["memory.retrieve".to_string()],
                methods: vec!["retrieve".to_string()],
                required_permissions: vec!["event-log:read".to_string()],
                location: ExecutionLocation::ResidentHome,
                health: ExtensionHealth::Ready,
                enabled: true,
                config_schema: JsonMap::new(),
            }
        }

        async fn invoke(&self, invocation: CapabilityInvocation) -> Result<CapabilityResult> {
            Ok(CapabilityResult {
                invocation_id: invocation.id,
                extension_id: "memory-extension".to_string(),
                capability: "memory.retrieve".to_string(),
                output: JsonMap::new(),
                metadata: JsonMap::new(),
            })
        }
    }

    #[tokio::test]
    async fn router_checks_permissions() -> Result<()> {
        let mut router = CapabilityRouter::new();
        router.register(PermissionedProvider)?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: "memory.retrieve".to_string(),
            method: "retrieve".to_string(),
            resident_id: "resident-default".to_string(),
            actor_id: None,
            input: JsonMap::new(),
            context: None,
        };
        let error = router.invoke(invocation.clone()).await.unwrap_err();
        assert!(matches!(error, CapabilityError::MissingPermission { .. }));

        router.grant_permission("memory-extension", "event-log:read");
        router.invoke(invocation).await?;
        Ok(())
    }

    #[derive(Clone)]
    struct UserSpeechExtension;

    #[async_trait]
    impl CapabilityProvider for UserSpeechExtension {
        fn registration(&self) -> ProviderRegistration {
            ProviderRegistration {
                extension_id: "user.custom-tts".to_string(),
                capabilities: vec!["speech.synthesis".to_string()],
                methods: vec!["synthesize".to_string()],
                required_permissions: Vec::new(),
                location: ExecutionLocation::ResidentHome,
                health: ExtensionHealth::Ready,
                enabled: true,
                config_schema: JsonMap::new(),
            }
        }

        async fn invoke(&self, invocation: CapabilityInvocation) -> Result<CapabilityResult> {
            Ok(CapabilityResult {
                invocation_id: invocation.id,
                extension_id: "user.custom-tts".to_string(),
                capability: "speech.synthesis".to_string(),
                output: JsonMap::from([("speechRef".to_string(), json!("user-tts://cmd_1"))]),
                metadata: JsonMap::new(),
            })
        }
    }

    #[tokio::test]
    async fn router_can_switch_capability_between_default_and_user_extension() -> Result<()> {
        let mut router = CapabilityRouter::new();
        router.register(StubSpeechSynthesisProvider)?;
        router.register(UserSpeechExtension)?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: "speech.synthesis".to_string(),
            method: "synthesize".to_string(),
            resident_id: "resident-default".to_string(),
            actor_id: None,
            input: JsonMap::from([("displayCommandId".to_string(), json!("cmd_1"))]),
            context: None,
        };

        let default_result = router.invoke(invocation.clone()).await?;
        assert_eq!(default_result.extension_id, "yuukei.default-tts");

        router.set_default_extension("speech.synthesis", "user.custom-tts");
        let user_result = router.invoke(invocation).await?;
        assert_eq!(user_result.extension_id, "user.custom-tts");
        assert_eq!(user_result.output["speechRef"], "user-tts://cmd_1");
        Ok(())
    }
}
