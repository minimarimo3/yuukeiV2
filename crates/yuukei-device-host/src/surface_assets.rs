use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceAssetCatalog {
    pub world_pack_id: String,
    pub actors: Vec<ActorSurfaceAsset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceAsset {
    pub actor_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<ActorSurfaceRendererAsset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceRendererAsset {
    pub kind: ActorSurfaceRendererKind,
    pub model: String,
    #[serde(default)]
    pub motions: BTreeMap<String, String>,
    #[serde(default)]
    pub hit_zones: Vec<ActorSurfaceHitZoneDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorSurfaceRendererKind {
    Vrm,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceHitZoneDefinition {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub source: ActorSurfaceHitZoneSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ActorSurfaceHitZoneShape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorSurfaceHitZoneSource {
    HumanoidBone,
    NodeName,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorSurfaceHitZoneShape {
    Auto,
    Mesh,
}

pub(crate) fn actor_surface_asset_catalog(world: &WorldPack) -> ActorSurfaceAssetCatalog {
    ActorSurfaceAssetCatalog {
        world_pack_id: world.id.clone(),
        actors: world
            .actors
            .iter()
            .map(|actor| ActorSurfaceAsset {
                actor_id: actor.id.clone(),
                display_name: actor.display_name.clone(),
                renderer: actor
                    .renderer
                    .as_ref()
                    .map(|renderer| ActorSurfaceRendererAsset {
                        kind: match renderer.kind {
                            ActorRendererKind::Vrm => ActorSurfaceRendererKind::Vrm,
                        },
                        model: renderer.model.clone(),
                        motions: renderer.motions.clone(),
                        hit_zones: renderer
                            .hit_zones
                            .iter()
                            .map(|hit_zone| ActorSurfaceHitZoneDefinition {
                                id: hit_zone.id.clone(),
                                label: hit_zone.label.clone(),
                                source: match hit_zone.source {
                                    ActorHitZoneSource::HumanoidBone => {
                                        ActorSurfaceHitZoneSource::HumanoidBone
                                    }
                                    ActorHitZoneSource::NodeName => {
                                        ActorSurfaceHitZoneSource::NodeName
                                    }
                                },
                                bones: hit_zone.bones.clone(),
                                nodes: hit_zone.nodes.clone(),
                                shape: hit_zone.shape.map(|shape| match shape {
                                    ActorHitZoneShape::Auto => ActorSurfaceHitZoneShape::Auto,
                                    ActorHitZoneShape::Mesh => ActorSurfaceHitZoneShape::Mesh,
                                }),
                                events: hit_zone.events.clone(),
                                priority: hit_zone.priority,
                            })
                            .collect(),
                    }),
            })
            .collect(),
    }
}
