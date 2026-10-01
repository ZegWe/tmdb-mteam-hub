const OPENAPI_DOCUMENT: &str = include_str!("openapi.json");

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::OPENAPI_DOCUMENT;

    const SUMMARY_FIELDS: &[&str] = &[
        "subject_id",
        "revision",
        "active",
        "inactive_at",
        "last_seen_snapshot_id",
        "media_kind",
        "schedulable",
        "blocked_reason",
        "lifecycle_state",
        "execution_state",
        "next_attempt_at",
        "retry_count",
        "max_retries",
        "retry_blocked",
        "force_eligible_once",
        "updated_at",
        "title",
        "release_year",
        "poster_url",
        "category_text",
        "douban_sort_time",
        "attention_tags",
    ];

    fn schema<'a>(document: &'a Value, name: &str) -> &'a Value {
        &document["components"]["schemas"][name]
    }

    #[test]
    fn subscription_retry_contract_documents_summary_and_rejection_cases() {
        let document: Value = serde_json::from_str(OPENAPI_DOCUMENT).expect("parse OpenAPI JSON");
        let operation = &document["paths"]["/api/subscriptions/wanted/{id}/retry"]["post"];
        let parameter = &operation["parameters"][0];
        assert_eq!(parameter["name"], "id");
        assert_eq!(parameter["in"], "path");
        assert_eq!(parameter["required"], true);
        assert_eq!(parameter["schema"]["type"], "string");
        assert_eq!(parameter["schema"]["minLength"], 1);
        assert_eq!(parameter["schema"]["maxLength"], 256);
        assert_eq!(
            operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/SubscriptionSummaryDto"
        );
        for status in ["400", "404", "409"] {
            assert_eq!(
                operation["responses"][status]["content"]["application/json"]["schema"]["$ref"],
                "#/components/schemas/ApiErrorDto"
            );
        }
        for (status, response) in [
            ("401", "ManagementUnauthorized"),
            ("405", "MethodNotAllowed"),
            ("500", "InternalError"),
            ("503", "ServiceUnavailable"),
        ] {
            assert_eq!(
                operation["responses"][status]["$ref"],
                format!("#/components/responses/{response}")
            );
        }
        let examples = &operation["responses"]["409"]["content"]["application/json"]["examples"];
        for (name, code) in [
            ("running", "subscription_execution_running"),
            ("inactive", "subscription_not_schedulable"),
            ("blocked", "subscription_not_schedulable"),
            ("completedTv", "subscription_rerun_unsupported"),
        ] {
            assert_eq!(examples[name]["value"]["code"], code);
        }
    }

    #[test]
    fn backend_openapi_contract_matches_the_stable_subscription_vocabulary() {
        let document: Value = serde_json::from_str(OPENAPI_DOCUMENT).expect("parse OpenAPI JSON");
        assert_eq!(document["openapi"], "3.1.0");

        let required = schema(&document, "SubscriptionSummaryDto")["required"]
            .as_array()
            .expect("summary required fields")
            .iter()
            .map(|value| value.as_str().expect("required field name"))
            .collect::<Vec<_>>();
        assert_eq!(required, SUMMARY_FIELDS);
        assert_eq!(
            schema(&document, "SubscriptionMediaKind")["enum"],
            serde_json::json!(["movie", "tv"])
        );
        assert_eq!(
            schema(&document, "SubscriptionExecutionState")["enum"],
            serde_json::json!(["idle", "running"])
        );
        assert!(document["paths"]["/api/subscriptions/wanted"]["get"].is_object());
        assert!(document["paths"]["/api/subscriptions/wanted/{id}"]["get"].is_object());
        assert!(document["paths"]["/api/mteam/torrents"]["get"].is_object());
        assert!(document["paths"]["/api/tmdb/movie/{id}"]["get"].is_object());
        assert!(document["paths"]["/api/tmdb/tv/{id}"]["get"].is_object());
        assert!(document["paths"]["/api/tmdb/tv/{id}/season/{season}"]["get"].is_object());
        for path in [
            "/api/douban/search",
            "/api/douban/library",
            "/api/douban/tags",
            "/api/douban/subject/{id}",
            "/api/douban/image",
            "/api/douban/qr/poll",
            "/api/douban/qr/image",
        ] {
            assert!(
                document["paths"][path]["get"].is_object(),
                "missing GET {path}"
            );
        }
        for path in ["/api/douban/subject/{id}/interest", "/api/douban/qr/start"] {
            assert!(
                document["paths"][path]["post"].is_object(),
                "missing POST {path}"
            );
        }
        assert_eq!(
            schema(&document, "MteamSearchSource")["enum"],
            serde_json::json!(["imdb", "douban", "keyword"])
        );
        assert_eq!(
            schema(&document, "MteamSearchResponseDto")["required"],
            serde_json::json!(["items", "page", "page_size"])
        );
        assert_eq!(
            schema(&document, "MteamTorrentDto")["additionalProperties"],
            false
        );
        assert_eq!(
            schema(&document, "TmdbSearchItemDto")["additionalProperties"],
            false
        );
        assert_eq!(
            schema(&document, "TmdbMediaDetailDto")["additionalProperties"],
            false
        );
        assert_eq!(
            schema(&document, "TmdbSeasonDetailDto")["additionalProperties"],
            false
        );
        for name in [
            "DoubanSearchItemDto",
            "DoubanSearchResponseDto",
            "DoubanSubjectDetailDto",
            "DoubanLibraryResponseDto",
            "DoubanTagHistoryResponseDto",
            "DoubanQrStartResponseDto",
            "DoubanQrPollResponseDto",
            "OperationLogEntryDto",
            "OperationLogRelatedDto",
            "OperationLogPageDto",
        ] {
            assert_eq!(
                schema(&document, name)["additionalProperties"],
                false,
                "{name} must be closed"
            );
        }
        assert_eq!(
            document["security"],
            serde_json::json!([{ "ManagementSession": [] }])
        );
        assert_eq!(
            document["components"]["securitySchemes"]["ManagementSession"]["name"],
            "tmdb_mteam_admin_session"
        );
    }
}
