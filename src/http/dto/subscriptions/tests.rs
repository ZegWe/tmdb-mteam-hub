use serde_json::json;

use super::cursor::MAX_CURSOR_TOKEN_LEN;
use super::{
    CursorCodecError, ListCursorScope, OpaqueListCursor, SubscriptionDetailDto,
    SubscriptionListResponse,
};
use crate::subscription::repository::payload;
use crate::subscription::repository::{
    ListCursor, ListSubscriptionsCommand, Revision, SnapshotId, SubscriptionDetail,
    SubscriptionHead, SubscriptionKey, SubscriptionListFilter, SubscriptionListPage,
    SubscriptionProjection, SubscriptionSummary,
};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind,
};

const ACCOUNT: &str = "account-secret";
const SUBJECT: &str = "subject-9";
const SECRET_DOWNLOAD_URL: &str = "https://tracker.test/download?passkey=must-not-leak";
const SECRET_DIAGNOSTIC: &str = "provider-token-must-not-leak";

fn redact_diagnostic(value: &str) -> String {
    value.replace(SECRET_DIAGNOSTIC, "[REDACTED]")
}

fn source() -> payload::WantedSourcePayload {
    payload::WantedSourcePayload {
        title: "Fixture Movie".to_string(),
        release_year: Some(2026),
        poster_url: "https://images.test/poster.jpg".to_string(),
        cover_url: "https://images.test/cover.jpg".to_string(),
        original_title: Some("Fixture Original".to_string()),
        languages: vec!["zh".to_string()],
        genres: vec!["Drama".to_string()],
        directors: vec!["Director".to_string()],
        actors: vec!["Actor".to_string()],
        summary: Some("Fixture synopsis".to_string()),
        rating_value: Some(8.5),
        rating_count: Some(42),
        category_text: Some("movie".to_string()),
        douban_sort_time: Some(123),
        douban_return_order: Some(7),
        ..payload::WantedSourcePayload::default()
    }
}

fn summary(source: &payload::WantedSourcePayload) -> SubscriptionSummary {
    SubscriptionSummary {
        head: SubscriptionHead {
            key: SubscriptionKey::try_new(ACCOUNT, SUBJECT).unwrap(),
            revision: Revision::try_new(3).unwrap(),
            active: true,
            inactive_at: None,
            last_seen_snapshot_id: Some(SnapshotId::try_new("snapshot-3").unwrap()),
            media_kind: SubscriptionMediaKind::Movie,
            schedulable: true,
            blocked_reason: None,
            lifecycle_state: SubscriptionLifecycleState::Downloading,
            execution_state: SubscriptionExecutionState::Idle,
            next_attempt_at: Some(50),
            retry_count: 2,
            max_retries: 5,
            retry_blocked: false,
            force_eligible_once: true,
            updated_at: 40,
        },
        projection: SubscriptionProjection::from_source(source).unwrap(),
        attention_tags: vec![SubscriptionAttentionTag::WaitingRelease],
    }
}

fn rich_detail() -> SubscriptionDetail {
    let source = source();
    let download_id = payload::stable_download_artifact_key(ACCOUNT, SUBJECT, "torrent-1");
    let link_id = payload::stable_resolved_link_artifact_key(ACCOUNT, SUBJECT, &download_id);
    let detail_payload = payload::SubscriptionPayload {
        source: source.clone(),
        observation: payload::ObservationPayload {
            created_at: 10,
            first_seen_at: 20,
            last_seen_at: 30,
        },
        issues: vec![payload::IssuePayload {
            owner: payload::IssueOwnerPayload::Artifact {
                artifact_kind: payload::ArtifactKindPayload::Download,
                artifact_id: download_id.clone(),
            },
            operation: Some(format!("progress:{SECRET_DIAGNOSTIC}")),
            error_type: Some(format!("upstream:{SECRET_DIAGNOSTIC}")),
            message: format!("temporary failure: {SECRET_DIAGNOSTIC}"),
            occurred_at: Some(35),
        }],
        skip_reason: None,
        candidates: vec![payload::CandidateMatchPayload {
            candidate: payload::CandidatePayload {
                torrent_id: "torrent-1".to_string(),
                title: "Fixture.Movie.2026.1080p".to_string(),
                subtitle: "Fixture subtitle".to_string(),
                source: "mteam".to_string(),
                search_query: "storage-only query".to_string(),
                seeders: Some(10),
                leechers: Some(2),
                ..payload::CandidatePayload::default()
            },
            selected: true,
            matched_rule_name: Some("1080p".to_string()),
            matched_priority: Some(10),
            matched_keywords: vec!["1080p".to_string()],
            rule_evaluations: vec![payload::CandidateRuleEvaluationPayload {
                rule_name: "storage-only-rule-evaluation".to_string(),
                ..payload::CandidateRuleEvaluationPayload::default()
            }],
            ..payload::CandidateMatchPayload::default()
        }],
        tv: None,
        artifacts: payload::ArtifactPayload {
            downloads: vec![payload::DownloadArtifactPayload {
                idempotency_key: download_id.clone(),
                torrent_id: "torrent-1".to_string(),
                torrent_title: "Fixture.Movie.2026.1080p".to_string(),
                qb_server_id: "qb-1".to_string(),
                qb_server_name: Some("Primary qB".to_string()),
                qb_category: "movies".to_string(),
                qb_save_dir_name: "fixture-movie".to_string(),
                qb_identifier: Some("storage-only-qb-identifier".to_string()),
                qb_hash: Some("abc123".to_string()),
                qb_name: Some("Fixture Movie".to_string()),
                qb_state: Some("uploading".to_string()),
                torrent_download_url: Some(SECRET_DOWNLOAD_URL.to_string()),
                mteam_torrent_url: Some("https://tracker.test/detail/1".to_string()),
                state: payload::DownloadArtifactStatePayload::Downloaded,
                progress: Some(1.0),
                total_size: Some(1024),
                files: vec![payload::DownloadFilePayload {
                    name: "Fixture.Movie.mkv".to_string(),
                    size: 1024,
                    progress: 1.0,
                    priority: 7,
                    season_number: None,
                    episode_number: None,
                    episode_end_number: None,
                    episode_label: None,
                }],
                pushed_at: Some(31),
                checked_at: Some(32),
                completed_at: Some(33),
            }],
            links: vec![payload::LinkArtifactPayload {
                idempotency_key: link_id,
                download: payload::LinkDownloadRefPayload {
                    artifact_id: download_id,
                },
                state: payload::LinkArtifactStatePayload::Failed,
                source_path: Some("/downloads/Fixture.Movie.mkv".to_string()),
                target_dir: Some("/media/movies/Fixture Movie (2026)".to_string()),
                checked_at: 34,
                completed_at: None,
                files: vec![payload::LinkFilePayload {
                    source_path: "/downloads/Fixture.Movie.mkv".to_string(),
                    target_path: "/media/movies/Fixture Movie (2026)/Fixture.Movie.mkv".to_string(),
                    size: 1024,
                    outcome: payload::LinkFileOutcomePayload::Failed,
                    season_number: None,
                    episode_number: None,
                    episode_end_number: None,
                    episode_label: None,
                    error: Some(format!("link failed: {SECRET_DIAGNOSTIC}")),
                }],
            }],
        },
    };

    SubscriptionDetail::try_new(summary(&source), detail_payload).unwrap()
}

fn list_command(
    filter: SubscriptionListFilter,
    cursor: Option<ListCursor>,
    limit: u32,
) -> ListSubscriptionsCommand {
    ListSubscriptionsCommand::try_new(ACCOUNT, filter, cursor, limit).unwrap()
}

#[test]
fn list_response_contains_only_explicit_summary_fields_and_opaque_cursor() {
    let source = source();
    let page = SubscriptionListPage {
        items: vec![summary(&source)],
        next_cursor: Some(ListCursor::try_new(Some(123), SUBJECT).unwrap()),
    };
    let command = list_command(SubscriptionListFilter::default(), None, 25);
    let scope = ListCursorScope::from_command(&command);

    let response = SubscriptionListResponse::try_from_page(&page, &command).unwrap();
    let value = serde_json::to_value(response).unwrap();
    let encoded_cursor =
        OpaqueListCursor::try_from(value["next_cursor"].as_str().unwrap()).unwrap();
    assert_eq!(
        encoded_cursor.decode(scope).unwrap(),
        page.next_cursor.clone().unwrap()
    );
    assert_eq!(
        value["items"][0],
        json!({
            "subject_id": SUBJECT,
            "revision": 3,
            "active": true,
            "inactive_at": null,
            "last_seen_snapshot_id": "snapshot-3",
            "media_kind": "movie",
            "schedulable": true,
            "blocked_reason": null,
            "lifecycle_state": "downloading",
            "execution_state": "idle",
            "next_attempt_at": 50,
            "retry_count": 2,
            "max_retries": 5,
            "retry_blocked": false,
            "force_eligible_once": true,
            "updated_at": 40,
            "title": "Fixture Movie",
            "release_year": 2026,
            "poster_url": "https://images.test/poster.jpg",
            "category_text": "movie",
            "douban_sort_time": 123,
            "attention_tags": ["waiting_release"]
        })
    );
    let item = value["items"][0].as_object().unwrap();
    for forbidden in [
        "account_key",
        "record_json",
        "candidates",
        "downloads",
        "links",
    ] {
        assert!(!item.contains_key(forbidden), "summary leaked {forbidden}");
    }
}

#[test]
fn detail_response_exposes_required_heavy_views_without_storage_only_or_secret_fields() {
    let dto = SubscriptionDetailDto::from_detail(&rich_detail(), &redact_diagnostic);
    let value = serde_json::to_value(&dto).unwrap();
    let raw = serde_json::to_string(&dto).unwrap();

    assert_eq!(value["summary"]["subject_id"], SUBJECT);
    assert_eq!(value["source"]["original_title"], "Fixture Original");
    assert_eq!(value["source"]["synopsis"], "Fixture synopsis");
    assert_eq!(value["candidates"][0]["torrent_id"], "torrent-1");
    assert_eq!(value["downloads"][0]["state"], "downloaded");
    assert_eq!(value["links"][0]["state"], "failed");
    assert_eq!(value["issues"][0]["owner"], "download_artifact");
    assert!(value["issues"][0]["message"]
        .as_str()
        .unwrap()
        .contains("[REDACTED]"));
    assert!(value["links"][0]["files"][0]["error"]
        .as_str()
        .unwrap()
        .contains("[REDACTED]"));

    let candidate = value["candidates"][0].as_object().unwrap();
    for forbidden in ["search_query", "rule_evaluations"] {
        assert!(!candidate.contains_key(forbidden));
    }
    let download = value["downloads"][0].as_object().unwrap();
    for forbidden in [
        "idempotency_key",
        "qb_identifier",
        "torrent_download_url",
        "mteam_torrent_url",
    ] {
        assert!(
            !download.contains_key(forbidden),
            "download DTO leaked {forbidden}"
        );
    }
    assert!(!value["downloads"][0]["files"][0]
        .as_object()
        .unwrap()
        .contains_key("priority"));
    assert!(!raw.contains(SECRET_DOWNLOAD_URL));
    assert!(!raw.contains(SECRET_DIAGNOSTIC));
    assert!(raw.contains("[REDACTED]"));
    assert!(!raw.contains(ACCOUNT));
    assert!(!raw.contains("record_json"));
    assert!(!raw.contains("storage-only"));
}

#[test]
fn cursor_v2_is_deterministic_url_safe_and_round_trips_null_and_non_null_keys() {
    let scope = ListCursorScope::new(ACCOUNT, &SubscriptionListFilter::default());
    for cursor in [
        ListCursor::try_new(Some(123), SUBJECT).unwrap(),
        ListCursor::try_new(None, SUBJECT).unwrap(),
        ListCursor::try_new(Some(u64::MAX), "中文-9").unwrap(),
    ] {
        let encoded = OpaqueListCursor::encode(&cursor, scope).unwrap();
        assert_eq!(
            encoded,
            OpaqueListCursor::encode(&cursor, scope).unwrap(),
            "v2 encoding must remain deterministic"
        );
        assert!(encoded.as_str().starts_with("v2."));
        assert_eq!(encoded.decode(scope).unwrap(), cursor);
        assert!(!encoded.as_str().contains(&cursor.subject_id));
        assert!(encoded
            .as_str()
            .bytes()
            .all(|byte| byte == b'.' || byte.is_ascii_alphanumeric()));

        let json = serde_json::to_string(&encoded).unwrap();
        let decoded: OpaqueListCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, encoded);
    }
}

#[test]
fn cursor_scope_binds_account_and_every_filter_but_not_limit_or_incoming_cursor() {
    let cursor = ListCursor::try_new(Some(123), SUBJECT).unwrap();
    let base_filter = SubscriptionListFilter::default();
    let scope = ListCursorScope::new(ACCOUNT, &base_filter);
    let encoded = OpaqueListCursor::encode(&cursor, scope).unwrap();

    assert_eq!(
        encoded.decode(ListCursorScope::new("other-account", &base_filter)),
        Err(CursorCodecError::ScopeMismatch)
    );
    for filter in [
        SubscriptionListFilter {
            active: Some(true),
            ..SubscriptionListFilter::default()
        },
        SubscriptionListFilter {
            media_kind: Some(SubscriptionMediaKind::Tv),
            ..SubscriptionListFilter::default()
        },
        SubscriptionListFilter {
            lifecycle_state: Some(SubscriptionLifecycleState::Completed),
            ..SubscriptionListFilter::default()
        },
        SubscriptionListFilter {
            attention_tag: Some(SubscriptionAttentionTag::Failed),
            ..SubscriptionListFilter::default()
        },
    ] {
        assert_eq!(
            encoded.decode(ListCursorScope::new(ACCOUNT, &filter)),
            Err(CursorCodecError::ScopeMismatch),
            "changed filter must invalidate the continuation cursor"
        );
    }

    let first = list_command(base_filter.clone(), None, 1);
    let second = list_command(
        base_filter,
        Some(ListCursor::try_new(None, "earlier-page").unwrap()),
        100,
    );
    assert_eq!(
        ListCursorScope::from_command(&first),
        ListCursorScope::from_command(&second),
        "limit and incoming cursor must not alter list scope"
    );
}

#[test]
fn cursor_try_from_and_deserialize_reject_invalid_or_oversized_tokens() {
    assert_eq!(
        OpaqueListCursor::try_from("v1.00"),
        Err(CursorCodecError::UnsupportedVersion {
            version: "v1".to_string()
        })
    );
    assert!(serde_json::from_value::<OpaqueListCursor>(json!("v1.00")).is_err());

    for token in ["", "v2", "v2.", "v2.0", "v2.gg"] {
        assert_eq!(
            OpaqueListCursor::try_from(token),
            Err(CursorCodecError::InvalidEncoding),
            "token {token:?}"
        );
        assert!(serde_json::from_value::<OpaqueListCursor>(json!(token)).is_err());
    }

    let scope = "00".repeat(32);
    for token in [
        format!("v2.{scope}"),
        format!("v2.{scope}00"),
        format!("v2.{scope}01"),
        format!("v2.{scope}02aa"),
        format!("v2.{scope}00ff"),
        format!("v2.{scope}002020"),
    ] {
        assert_eq!(
            OpaqueListCursor::try_from(token.as_str()),
            Err(CursorCodecError::InvalidPayload),
            "token {token:?}"
        );
        assert!(serde_json::from_value::<OpaqueListCursor>(json!(token)).is_err());
    }

    let oversized = format!("v2.{}", "0".repeat(MAX_CURSOR_TOKEN_LEN));
    assert_eq!(
        OpaqueListCursor::try_from(oversized.as_str()),
        Err(CursorCodecError::Oversized {
            max_length: MAX_CURSOR_TOKEN_LEN
        })
    );
    assert!(serde_json::from_value::<OpaqueListCursor>(json!(oversized)).is_err());
}

#[test]
fn detail_top_level_shape_is_explicit_and_does_not_embed_the_repository_payload() {
    let value = serde_json::to_value(SubscriptionDetailDto::from_detail(
        &rich_detail(),
        &redact_diagnostic,
    ))
    .unwrap();
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(
        keys,
        [
            "candidates",
            "downloads",
            "issues",
            "links",
            "observation",
            "skip_reason",
            "source",
            "summary",
            "tv",
        ]
    );
}
