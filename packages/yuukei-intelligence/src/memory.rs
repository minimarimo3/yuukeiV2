use std::{
    cmp::Ordering,
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

const MAX_FACTS: usize = 50;
const EPISODE_HALF_LIFE_DAYS: f64 = 14.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Fact {
    id: String,
    text: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Episode {
    id: String,
    date: String,
    timestamp: String,
    text: String,
}

struct Store {
    facts_path: PathBuf,
    episodes_path: PathBuf,
}

pub fn index(input: &Value, summary: Option<Value>, mut metadata: Value) -> (Value, Value) {
    let events_empty = input
        .get("events")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty);
    if events_empty {
        return (
            json!({ "indexed": true, "noteCount": 0 }),
            json!({ "skipped": "empty-events" }),
        );
    }
    let Some(summary) = summary else {
        return (json!({ "indexed": false }), metadata);
    };
    match index_inner(input, &summary) {
        Ok(note_count) => (
            json!({ "indexed": true, "noteCount": note_count }),
            metadata,
        ),
        Err(error) => {
            eprintln!("yuukei-intelligence: memory index storage failed: {error:#}");
            add_reason(&mut metadata, "storage-error");
            (json!({ "indexed": false }), metadata)
        }
    }
}

pub fn list(input: &Value) -> (Value, Value) {
    match list_inner(input) {
        Ok((facts, episodes, total)) => (
            json!({ "facts": facts, "episodes": episodes, "episodeTotal": total }),
            json!({ "facts": facts.len(), "episodes": total }),
        ),
        Err(error) => {
            eprintln!("yuukei-intelligence: memory list failed: {error:#}");
            (
                json!({ "facts": [], "episodes": [], "episodeTotal": 0 }),
                json!({ "reason": "storage-error" }),
            )
        }
    }
}

pub fn retrieve(input: &Value) -> (Value, Value) {
    match retrieve_inner(input) {
        Ok((memories, facts, episodes)) => (
            json!({ "memories": memories }),
            json!({ "facts": facts, "episodes": episodes }),
        ),
        Err(error) => {
            eprintln!("yuukei-intelligence: memory retrieve failed: {error:#}");
            (
                json!({ "memories": [] }),
                json!({ "reason": "storage-error" }),
            )
        }
    }
}

pub fn update(input: &Value) -> (Value, Value) {
    let text = input
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let id = input.get("id").and_then(Value::as_str).unwrap_or("");
    if input.get("kind").and_then(Value::as_str) != Some("fact")
        || text.is_empty()
        || text.chars().count() > 500
        || id.is_empty()
    {
        return (
            json!({ "updated": false }),
            json!({ "reason": "invalid-input" }),
        );
    }
    match update_inner(input, id, text) {
        Ok(true) => (json!({ "updated": true }), json!({})),
        Ok(false) => (
            json!({ "updated": false }),
            json!({ "reason": "not-found" }),
        ),
        Err(error) => {
            eprintln!("yuukei-intelligence: memory update failed: {error:#}");
            (
                json!({ "updated": false }),
                json!({ "reason": "storage-error" }),
            )
        }
    }
}

pub fn forget(input: &Value) -> (Value, Value) {
    match forget_inner(input) {
        Ok((facts, episodes)) => (
            json!({ "removedFacts": facts, "removedEpisodes": episodes }),
            json!({}),
        ),
        Err(error) => {
            eprintln!("yuukei-intelligence: memory forget failed: {error:#}");
            (
                json!({ "removedFacts": 0, "removedEpisodes": 0 }),
                json!({ "reason": "storage-error" }),
            )
        }
    }
}

fn index_inner(input: &Value, summary: &Value) -> Result<usize> {
    let store = open_store(input)?;
    let diary = summary
        .get("diary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let date = input.get("date").and_then(Value::as_str).unwrap_or("");
    if !date.is_empty() && !diary.is_empty() {
        upsert_episode(&store, date, diary)?;
    }
    let new_facts = summary
        .get("newFacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    merge_facts(&store, &new_facts)
}

fn list_inner(input: &Value) -> Result<(Vec<Fact>, Vec<Value>, usize)> {
    let store = open_store(input)?;
    let (facts, facts_changed) = read_facts(&store.facts_path)?;
    let (mut episodes, episodes_changed) = read_episodes(&store.episodes_path)?;
    if facts_changed {
        write_facts(&store.facts_path, &facts)?;
    }
    if episodes_changed {
        write_episodes(&store.episodes_path, &episodes)?;
    }
    episodes.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
    let offset = non_negative_integer(input.get("episodeOffset"), 0);
    let limit = positive_integer(input.get("episodeLimit"), 50);
    let total = episodes.len();
    let episodes = episodes
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|episode| {
            json!({
                "id": episode.id,
                "text": episode.text,
                "timestamp": episode.timestamp
            })
        })
        .collect();
    Ok((facts, episodes, total))
}

fn retrieve_inner(input: &Value) -> Result<(Vec<Value>, usize, usize)> {
    let store = open_store(input)?;
    let (facts, _) = read_facts(&store.facts_path)?;
    let (episodes, _) = read_episodes(&store.episodes_path)?;
    let query = input
        .pointer("/query/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    let facts_limit = positive_integer(input.pointer("/limits/facts"), 10);
    let episodes_limit = positive_integer(input.pointer("/limits/episodes"), 5);
    let mut memories = rank_facts(&facts, query)
        .into_iter()
        .take(facts_limit)
        .collect::<Vec<_>>();
    memories.extend(
        rank_episodes(&episodes, query)
            .into_iter()
            .take(episodes_limit),
    );
    Ok((memories, facts.len(), episodes.len()))
}

fn update_inner(input: &Value, id: &str, text: &str) -> Result<bool> {
    let store = open_store(input)?;
    let (mut facts, changed) = read_facts(&store.facts_path)?;
    let Some(fact) = facts.iter_mut().find(|fact| fact.id == id) else {
        if changed {
            write_facts(&store.facts_path, &facts)?;
        }
        return Ok(false);
    };
    fact.text = text.to_string();
    fact.updated_at = Utc::now().to_rfc3339();
    write_facts(&store.facts_path, &facts)?;
    Ok(true)
}

fn forget_inner(input: &Value) -> Result<(usize, usize)> {
    let store = open_store(input)?;
    let (mut facts, _) = read_facts(&store.facts_path)?;
    let (mut episodes, _) = read_episodes(&store.episodes_path)?;
    let previous_facts = facts.len();
    let previous_episodes = episodes.len();
    if input.get("all") == Some(&Value::Bool(true)) {
        facts.clear();
        episodes.clear();
    } else {
        let entries = input
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let fact_ids = entry_ids(&entries, "fact");
        let episode_ids = entry_ids(&entries, "episode");
        facts.retain(|fact| !fact_ids.contains(&fact.id));
        episodes.retain(|episode| !episode_ids.contains(&episode.id));
    }
    write_facts(&store.facts_path, &facts)?;
    write_episodes(&store.episodes_path, &episodes)?;
    Ok((
        previous_facts - facts.len(),
        previous_episodes - episodes.len(),
    ))
}

fn open_store(input: &Value) -> Result<Store> {
    let data_dir = env::var_os("YUUKEI_EXTENSION_DATA_DIR")
        .context("YUUKEI_EXTENSION_DATA_DIR is not configured")?;
    let root = PathBuf::from(data_dir)
        .join("memory")
        .join(safe_segment(
            input.get("worldPackId").and_then(Value::as_str),
        ))
        .join(safe_segment(
            input.get("residentId").and_then(Value::as_str),
        ));
    fs::create_dir_all(&root)?;
    Ok(Store {
        facts_path: root.join("facts.json"),
        episodes_path: root.join("episodes.jsonl"),
    })
}

fn upsert_episode(store: &Store, date: &str, text: &str) -> Result<()> {
    let (mut episodes, _) = read_episodes(&store.episodes_path)?;
    let existing_id = episodes
        .iter()
        .find(|episode| episode.date == date)
        .map(|episode| episode.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    episodes.retain(|episode| episode.date != date);
    episodes.push(Episode {
        id: existing_id,
        date: date.to_string(),
        timestamp: date.to_string(),
        text: text.to_string(),
    });
    episodes.sort_by(|left, right| left.date.cmp(&right.date));
    write_episodes(&store.episodes_path, &episodes)
}

fn merge_facts(store: &Store, new_facts: &[&str]) -> Result<usize> {
    let (mut facts, _) = read_facts(&store.facts_path)?;
    let now = Utc::now().to_rfc3339();
    for fact_text in new_facts
        .iter()
        .map(|fact| fact.trim())
        .filter(|fact| !fact.is_empty())
    {
        let normalized = normalize_duplicate(fact_text);
        if normalized.is_empty() {
            continue;
        }
        if let Some(duplicate) = facts.iter_mut().find(|fact| {
            let existing = normalize_duplicate(&fact.text);
            existing == normalized
                || existing.contains(&normalized)
                || normalized.contains(&existing)
        }) {
            duplicate.updated_at = now.clone();
        } else {
            facts.push(Fact {
                id: Uuid::new_v4().to_string(),
                text: fact_text.to_string(),
                created_at: now.clone(),
                updated_at: now.clone(),
            });
        }
    }
    facts.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    facts.truncate(MAX_FACTS);
    write_facts(&store.facts_path, &facts)?;
    Ok(facts.len())
}

fn read_facts(path: &Path) -> Result<(Vec<Fact>, bool)> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), false))
        }
        Err(error) => return Err(error.into()),
    };
    let values: Vec<Value> = serde_json::from_str(&raw)?;
    let mut changed = false;
    let facts = values
        .into_iter()
        .filter_map(|value| {
            let text = value.get("text")?.as_str()?.to_string();
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    changed = true;
                    Uuid::new_v4().to_string()
                });
            Some(Fact {
                id,
                text,
                created_at: value
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                updated_at: value
                    .get("updatedAt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect();
    Ok((facts, changed))
}

fn read_episodes(path: &Path) -> Result<(Vec<Episode>, bool)> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), false))
        }
        Err(error) => return Err(error.into()),
    };
    let mut changed = false;
    let mut episodes = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        let Some(text) = value.get("text").and_then(Value::as_str) else {
            continue;
        };
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .filter(|timestamp| !timestamp.trim().is_empty())
            .or_else(|| value.get("date").and_then(Value::as_str));
        let Some(timestamp) = timestamp else {
            continue;
        };
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                changed = true;
                Uuid::new_v4().to_string()
            });
        if value.get("timestamp").and_then(Value::as_str) != Some(timestamp) {
            changed = true;
        }
        episodes.push(Episode {
            id,
            date: timestamp.chars().take(10).collect(),
            timestamp: timestamp.to_string(),
            text: text.to_string(),
        });
    }
    Ok((episodes, changed))
}

fn write_facts(path: &Path, facts: &[Fact]) -> Result<()> {
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(facts)?))?;
    Ok(())
}

fn write_episodes(path: &Path, episodes: &[Episode]) -> Result<()> {
    let lines = episodes
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(
        path,
        if lines.is_empty() {
            lines
        } else {
            format!("{lines}\n")
        },
    )?;
    Ok(())
}

fn rank_facts(facts: &[Fact], query: &str) -> Vec<Value> {
    let mut ranked = facts
        .iter()
        .map(|fact| {
            (
                bigram_score(query, &fact.text),
                fact.updated_at.clone(),
                json!({ "text": fact.text, "kind": "fact" }),
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.1.cmp(&left.1))
    });
    ranked.into_iter().map(|(_, _, value)| value).collect()
}

fn rank_episodes(episodes: &[Episode], query: &str) -> Vec<Value> {
    let today = Utc::now().date_naive();
    let mut ranked = episodes
        .iter()
        .filter_map(|episode| {
            let relevance = bigram_score(query, &episode.text);
            if relevance <= 0.0 {
                return None;
            }
            let recency = NaiveDate::parse_from_str(&episode.date, "%Y-%m-%d")
                .ok()
                .map(|date| {
                    let age_days = (today - date).num_days().max(0) as f64;
                    0.5f64.powf(age_days / EPISODE_HALF_LIFE_DAYS)
                })
                .unwrap_or(0.0);
            Some((
                relevance + 0.1 * recency,
                episode.date.clone(),
                json!({
                    "text": episode.text,
                    "kind": "episode",
                    "date": episode.date
                }),
            ))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.1.cmp(&left.1))
    });
    ranked.into_iter().map(|(_, _, value)| value).collect()
}

fn bigram_score(query: &str, text: &str) -> f64 {
    let query_bigrams = bigrams(query);
    if query_bigrams.is_empty() {
        return 0.0;
    }
    let text_bigrams = bigrams(text);
    query_bigrams
        .iter()
        .filter(|bigram| text_bigrams.contains(*bigram))
        .count() as f64
        / query_bigrams.len() as f64
}

fn bigrams(value: &str) -> BTreeSet<String> {
    let characters = normalize_search(value).chars().collect::<Vec<_>>();
    match characters.as_slice() {
        [] => BTreeSet::new(),
        [character] => BTreeSet::from([character.to_string()]),
        _ => characters
            .windows(2)
            .map(|pair| pair.iter().collect::<String>())
            .collect(),
    }
}

fn normalize_search(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if ('ァ'..='ヶ').contains(&character) {
                char::from_u32(character as u32 - 0x60).unwrap_or(character)
            } else {
                character
            }
        })
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn normalize_duplicate(value: &str) -> String {
    normalize_search(value)
        .chars()
        .filter(|character| !"。、，,.!?！？".contains(*character))
        .collect()
}

fn entry_ids(entries: &[Value], kind: &str) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|entry| entry.get("kind").and_then(Value::as_str) == Some(kind))
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn positive_integer(value: Option<&Value>, fallback: usize) -> usize {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number > 0.0)
        .map(|number| number.trunc() as usize)
        .unwrap_or(fallback)
}

fn non_negative_integer(value: Option<&Value>, fallback: usize) -> usize {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number.trunc() as usize)
        .unwrap_or(fallback)
}

fn safe_segment(value: Option<&str>) -> String {
    let value = value.unwrap_or("").trim();
    if value.is_empty() {
        return "default".to_string();
    }
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn add_reason(metadata: &mut Value, reason: &str) {
    if !metadata.is_object() {
        *metadata = json!({});
    }
    if let Some(metadata) = metadata.as_object_mut() {
        metadata.insert("reason".to_string(), json!(reason));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_bigrams_are_normalized() {
        assert!(bigram_score("アンパン", "今日はあんぱんを食べた") > 0.0);
    }

    #[test]
    fn safe_segments_cannot_escape_the_data_directory() {
        assert_eq!(safe_segment(Some("../../resident")), ".._.._resident");
    }

    #[test]
    fn existing_iso_dates_parse_for_recency() {
        assert!(chrono::DateTime::parse_from_rfc3339("2026-07-28T00:00:00Z").is_ok());
    }
}
