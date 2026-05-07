//! Resolve Douban subject id (P4529) from Wikidata using TMDB `external_ids.wikidata_id`.

use serde_json::Value;

/// Fetch first Douban film/TV subject id from Wikidata entity `Q…`.
pub async fn fetch_douban_id_via_wikidata(qid_raw: &str) -> Option<String> {
    let qid = normalize_wikidata_qid(qid_raw)?;
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .ok()?;

    let text = client
        .get("https://www.wikidata.org/w/api.php")
        .query(&[
            ("action", "wbgetentities"),
            ("ids", qid.as_str()),
            ("format", "json"),
            ("props", "claims"),
        ])
        .header(
            "User-Agent",
            "tmdb-mteam-hub/0.1 (https://www.wikidata.org/wiki/Special:MyLanguage/Wikidata:Data_access)",
        )
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    let v: Value = serde_json::from_str(&text).ok()?;
    let ent = v.get("entities")?.get(&qid)?;
    let p4529 = ent.get("claims")?.get("P4529")?.as_array()?;
    let first = p4529.first()?;
    let val = first.get("mainsnak")?.get("datavalue")?.get("value")?;
    match val {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn normalize_wikidata_qid(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with('Q') && s[1..].chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }
    if let Some(idx) = s.find("/wiki/") {
        let q = s[idx + 6..].split('?').next()?.split('#').next()?.trim();
        if q.starts_with('Q') && q[1..].chars().all(|c| c.is_ascii_digit()) {
            return Some(q.to_string());
        }
    }
    None
}

/// Returns `true` if `douban_id` / `douban_url` were inserted.
pub async fn enrich_detail_with_douban(v: &mut Value) -> bool {
    let has_douban = v
        .get("douban_id")
        .and_then(|x| x.as_str())
        .is_some_and(|s| !s.is_empty());
    if has_douban {
        return false;
    }

    let qid_opt = v
        .pointer("/external_ids/wikidata_id")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("wikidata_id").and_then(|x| x.as_str()));

    let Some(qid) = qid_opt else {
        return false;
    };

    match fetch_douban_id_via_wikidata(qid).await {
        Some(did) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("douban_id".to_string(), Value::String(did.clone()));
                obj.insert(
                    "douban_url".to_string(),
                    Value::String(format!("https://movie.douban.com/subject/{did}/")),
                );
            }
            true
        }
        None => {
            tracing::debug!("wikidata {} has no usable P4529 (douban)", qid);
            false
        }
    }
}
