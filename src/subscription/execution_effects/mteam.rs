use std::collections::HashSet;

use serde_json::Value;

use super::super::execution::{ExecutionTorrentMatchRule, ExecutionTorrentRuleMatchMode};
use super::super::repository::payload::{
    CandidateMatchPayload, CandidatePayload, CandidateRuleEvaluationPayload,
};

const SEARCH_PAGE_SIZE: u32 = 100;

pub(super) fn search_body(field: &str, value: &str) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("pageNumber".to_string(), Value::from(1));
    body.insert("pageSize".to_string(), Value::from(SEARCH_PAGE_SIZE));
    body.insert("sortField".to_string(), Value::from("SEEDERS"));
    body.insert("sortDirection".to_string(), Value::from("DESC"));
    body.insert(field.to_string(), Value::from(value));
    Value::Object(body)
}

pub(super) fn append_candidates(
    output: &mut Vec<CandidatePayload>,
    seen: &mut HashSet<String>,
    candidates: Vec<CandidatePayload>,
) {
    for candidate in candidates {
        let identity = if candidate.torrent_id.is_empty() {
            format!("title:{}", candidate.title)
        } else {
            format!("id:{}", candidate.torrent_id)
        };
        if seen.insert(identity) {
            output.push(candidate);
        }
    }
}

pub(super) fn candidates_from_response(
    response: &Value,
    source: &str,
    query: &str,
) -> Vec<CandidatePayload> {
    let mut values = Vec::new();
    collect_candidate_objects(response, &mut values);
    values
        .into_iter()
        .filter_map(|value| candidate_from_value(value, source, query))
        .collect()
}

fn collect_candidate_objects<'a>(value: &'a Value, output: &mut Vec<&'a Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_candidate_objects(item, output);
            }
        }
        Value::Object(map) => {
            let identity = ["id", "torrentId", "torrent_id", "tid"]
                .iter()
                .any(|key| map.contains_key(*key));
            let title = ["name", "title", "smallDescr", "small_descr"]
                .iter()
                .any(|key| map.contains_key(*key));
            if identity && title {
                output.push(value);
            } else {
                for nested in map.values() {
                    collect_candidate_objects(nested, output);
                }
            }
        }
        _ => {}
    }
}

fn candidate_from_value(value: &Value, source: &str, query: &str) -> Option<CandidatePayload> {
    Some(CandidatePayload {
        torrent_id: first_string(value, &["id", "torrentId", "torrent_id", "tid"])
            .unwrap_or_default(),
        title: first_string(value, &["name", "title", "smallDescr", "small_descr"])?,
        subtitle: first_string(
            value,
            &[
                "smallDescr",
                "small_descr",
                "description",
                "descr",
                "subTitle",
                "subtitle",
            ],
        )
        .unwrap_or_default(),
        source: source.to_string(),
        search_query: query.to_string(),
        size: first_string(value, &["size", "sizeStr", "size_str"]),
        seeders: first_u64(value, &["seeders", "seeder", "seed", "status.seeders"]),
        leechers: first_u64(value, &["leechers", "leecher", "leech", "status.leechers"]),
        uploaded_at: first_string(value, &["createdDate", "created_date", "added", "date"]),
    })
}

fn first_string(value: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .filter_map(|path| value_at_path(value, path))
        .find_map(|value| match value {
            Value::String(value) => non_empty(value),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
}

fn first_u64(value: &Value, paths: &[&str]) -> Option<u64> {
    paths
        .iter()
        .filter_map(|path| value_at_path(value, path))
        .find_map(|value| match value {
            Value::Number(value) => value.as_u64(),
            Value::String(value) => value.trim().parse().ok(),
            _ => None,
        })
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

pub(super) fn match_candidates(
    candidates: &[CandidatePayload],
    rules: &[ExecutionTorrentMatchRule],
) -> Vec<CandidateMatchPayload> {
    if rules.is_empty() {
        let mut selected = false;
        return candidates
            .iter()
            .cloned()
            .map(|candidate| {
                let eligible = !candidate.torrent_id.is_empty();
                let row_selected = eligible && !selected;
                selected |= row_selected;
                CandidateMatchPayload {
                    candidate,
                    selected: row_selected,
                    matched_rule_name: row_selected.then(|| "default_first_candidate".to_string()),
                    matched_priority: row_selected.then_some(0),
                    matched_keywords: Vec::new(),
                    excluded_reason: (!row_selected).then(|| {
                        if eligible {
                            "an earlier candidate was selected".to_string()
                        } else {
                            "torrent ID is missing".to_string()
                        }
                    }),
                    rule_evaluations: Vec::new(),
                }
            })
            .collect();
    }

    let mut sorted_rules = rules.to_vec();
    sorted_rules.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut matches = candidates
        .iter()
        .cloned()
        .map(|candidate| {
            let rule_evaluations = sorted_rules
                .iter()
                .map(|rule| evaluate_rule(&candidate, rule))
                .collect::<Vec<_>>();
            let matched = rule_evaluations
                .iter()
                .find(|evaluation| evaluation.matched);
            CandidateMatchPayload {
                excluded_reason: if candidate.torrent_id.is_empty() {
                    Some("torrent ID is missing".to_string())
                } else {
                    None
                },
                candidate,
                selected: false,
                matched_rule_name: matched.map(|evaluation| evaluation.rule_name.clone()),
                matched_priority: matched.map(|evaluation| evaluation.priority),
                matched_keywords: matched
                    .map(|evaluation| evaluation.matched_keywords.clone())
                    .unwrap_or_default(),
                rule_evaluations,
            }
        })
        .collect::<Vec<_>>();
    let best_priority = matches
        .iter()
        .filter(|candidate| !candidate.candidate.torrent_id.is_empty())
        .filter_map(|candidate| candidate.matched_priority)
        .max();
    if let Some(priority) = best_priority {
        if let Some(selected) = matches.iter_mut().find(|candidate| {
            !candidate.candidate.torrent_id.is_empty()
                && candidate.matched_priority == Some(priority)
        }) {
            selected.selected = true;
            selected.excluded_reason = None;
        }
    }
    for candidate in matches.iter_mut().filter(|candidate| !candidate.selected) {
        if candidate.excluded_reason.is_none() {
            candidate.excluded_reason = Some(if candidate.matched_priority.is_some() {
                "a higher-priority or earlier candidate was selected".to_string()
            } else {
                "no rule matched".to_string()
            });
        }
    }
    matches
}

fn evaluate_rule(
    candidate: &CandidatePayload,
    rule: &ExecutionTorrentMatchRule,
) -> CandidateRuleEvaluationPayload {
    let checks = rule_checks(rule);
    let searchable = format!(
        "{}\n{}\n{}\n{}",
        candidate.title, candidate.subtitle, candidate.source, candidate.search_query
    )
    .to_lowercase();
    let mut matched_keywords = Vec::new();
    let mut missing_keywords = Vec::new();
    for (label, value) in checks {
        if searchable.contains(&value.to_lowercase()) {
            matched_keywords.push(label);
        } else {
            missing_keywords.push(label);
        }
    }
    let matched = !matched_keywords.is_empty()
        && match rule.mode {
            ExecutionTorrentRuleMatchMode::All => missing_keywords.is_empty(),
            ExecutionTorrentRuleMatchMode::Any => true,
        };
    CandidateRuleEvaluationPayload {
        rule_name: rule.name.clone(),
        priority: rule.priority,
        mode: match rule.mode {
            ExecutionTorrentRuleMatchMode::All => "all",
            ExecutionTorrentRuleMatchMode::Any => "any",
        }
        .to_string(),
        matched,
        matched_keywords,
        missing_keywords,
        excluded_reason: (!matched).then(|| "rule keywords did not match".to_string()),
    }
}

fn rule_checks(rule: &ExecutionTorrentMatchRule) -> Vec<(String, String)> {
    let mut checks = Vec::new();
    for (prefix, values) in [
        ("title", &rule.title_keywords),
        ("resolution", &rule.resolution_keywords),
        ("source", &rule.source_keywords),
    ] {
        for value in values {
            if let Some(value) = non_empty(value) {
                checks.push((format!("{prefix}:{value}"), value));
            }
        }
    }
    checks
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn candidate(id: &str, title: &str, seeders: u64) -> CandidatePayload {
        CandidatePayload {
            torrent_id: id.to_string(),
            title: title.to_string(),
            subtitle: String::new(),
            source: "fixture".to_string(),
            search_query: "fixture".to_string(),
            size: None,
            seeders: Some(seeders),
            leechers: None,
            uploaded_at: None,
        }
    }

    #[test]
    fn body_uses_the_requested_dynamic_query_field() {
        let body = search_body("douban", "https://movie.douban.com/subject/1/");
        assert_eq!(
            body.get("douban").and_then(Value::as_str),
            Some("https://movie.douban.com/subject/1/")
        );
        assert!(body.get("field").is_none());
        assert_eq!(body.get("pageSize").and_then(Value::as_u64), Some(100));
    }

    #[test]
    fn parser_and_matching_select_highest_priority() {
        let response = json!({
            "data": [
                { "id": "1", "name": "Movie 1080p WEB-DL", "seeders": 10 },
                { "torrentId": "2", "title": "Movie 2160p BluRay", "status": { "seeders": 3 } }
            ]
        });
        let mut parsed = candidates_from_response(&response, "keyword", "Movie");
        parsed.sort_by(|left, right| {
            right
                .seeders
                .unwrap_or_default()
                .cmp(&left.seeders.unwrap_or_default())
        });
        let rules = vec![
            ExecutionTorrentMatchRule {
                name: "1080p".to_string(),
                priority: 1,
                mode: ExecutionTorrentRuleMatchMode::All,
                title_keywords: vec!["1080p".to_string()],
                resolution_keywords: Vec::new(),
                source_keywords: Vec::new(),
            },
            ExecutionTorrentMatchRule {
                name: "2160p".to_string(),
                priority: 10,
                mode: ExecutionTorrentRuleMatchMode::All,
                title_keywords: vec!["2160p".to_string()],
                resolution_keywords: Vec::new(),
                source_keywords: Vec::new(),
            },
        ];
        let matches = match_candidates(&parsed, &rules);
        let selected = matches.iter().find(|candidate| candidate.selected).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(selected.candidate.torrent_id, "2");
        assert_eq!(selected.matched_rule_name.as_deref(), Some("2160p"));
    }

    #[test]
    fn empty_rule_never_matches() {
        let matches = match_candidates(
            &[candidate("1", "Movie 1080p", 10)],
            &[ExecutionTorrentMatchRule {
                name: "empty".to_string(),
                priority: 100,
                mode: ExecutionTorrentRuleMatchMode::Any,
                title_keywords: Vec::new(),
                resolution_keywords: Vec::new(),
                source_keywords: Vec::new(),
            }],
        );
        assert!(!matches[0].selected);
        assert_eq!(
            matches[0].excluded_reason.as_deref(),
            Some("no rule matched")
        );
    }
}
