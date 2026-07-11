use super::*;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityUsageState {
    pub extensions: Vec<ExtensionCapabilityUsage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCapabilityUsage {
    pub extension_id: String,
    pub capabilities: Vec<CapabilityUsageByCapability>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityUsageByCapability {
    pub capability: String,
    pub models: Vec<ModelCapabilityUsage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilityUsage {
    pub provider: String,
    pub model: String,
    pub all_time: TokenUsageTotals,
    pub last_7_days: TokenUsageTotals,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageTotals {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub(crate) fn capability_usage_from_records(
    records: &[EventLogRecord],
    now: DateTime<Utc>,
) -> CapabilityUsageState {
    let cutoff = now - chrono::Duration::days(7);
    let mut usage_by_key: BTreeMap<
        (String, String, String, String),
        (TokenUsageTotals, TokenUsageTotals),
    > = BTreeMap::new();

    for record in records {
        if record.kind != "capability.invocation.result" {
            continue;
        }
        let Some(usage) = record
            .payload
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("usage"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(extension_id) = record.payload.get("extensionId").and_then(Value::as_str) else {
            continue;
        };
        let Some(capability) = record.payload.get("capability").and_then(Value::as_str) else {
            continue;
        };
        let Some(provider) = usage.get("provider").and_then(Value::as_str) else {
            continue;
        };
        let Some(model) = usage.get("model").and_then(Value::as_str) else {
            continue;
        };
        let Some(input_tokens) = usage.get("inputTokens").and_then(Value::as_u64) else {
            continue;
        };
        let Some(output_tokens) = usage.get("outputTokens").and_then(Value::as_u64) else {
            continue;
        };

        let timestamp = DateTime::parse_from_rfc3339(&record.timestamp)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .ok();
        let entry = usage_by_key
            .entry((
                extension_id.to_string(),
                capability.to_string(),
                provider.to_string(),
                model.to_string(),
            ))
            .or_default();
        add_usage(&mut entry.0, input_tokens, output_tokens);
        if timestamp.is_some_and(|timestamp| timestamp >= cutoff) {
            add_usage(&mut entry.1, input_tokens, output_tokens);
        }
    }

    let mut extension_map: BTreeMap<String, BTreeMap<String, Vec<ModelCapabilityUsage>>> =
        BTreeMap::new();
    for ((extension_id, capability, provider, model), (all_time, last_7_days)) in usage_by_key {
        extension_map
            .entry(extension_id)
            .or_default()
            .entry(capability)
            .or_default()
            .push(ModelCapabilityUsage {
                provider,
                model,
                all_time,
                last_7_days,
            });
    }

    CapabilityUsageState {
        extensions: extension_map
            .into_iter()
            .map(|(extension_id, capabilities)| ExtensionCapabilityUsage {
                extension_id,
                capabilities: capabilities
                    .into_iter()
                    .map(|(capability, models)| CapabilityUsageByCapability { capability, models })
                    .collect(),
            })
            .collect(),
    }
}

fn add_usage(totals: &mut TokenUsageTotals, input_tokens: u64, output_tokens: u64) {
    totals.requests += 1;
    totals.input_tokens += input_tokens;
    totals.output_tokens += output_tokens;
}
