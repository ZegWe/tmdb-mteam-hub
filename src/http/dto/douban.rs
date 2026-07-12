use serde::{Deserialize, Serialize};

use crate::app::douban_catalog::{
    DoubanInterest, DoubanInterestResult, DoubanLibraryCommand, DoubanLibraryItem,
    DoubanLibraryList, DoubanLibraryOutcome, DoubanQrPollOutcome, DoubanQrStartOutcome,
    DoubanRating, DoubanSearchCommand, DoubanSearchItem, DoubanSearchOutcome,
    DoubanSnapshotCompleteness, DoubanSubjectDetail, DoubanTagCategory, DoubanTagCount,
    DoubanTagHistoryCommand, DoubanTagHistoryOutcome, MarkDoubanInterestCommand,
};

#[derive(Debug, Deserialize)]
pub(crate) struct DoubanSearchQuery {
    pub(crate) q: String,
    #[serde(default = "default_page")]
    pub(crate) page: usize,
    #[serde(default = "default_page_size")]
    pub(crate) page_size: usize,
}

impl From<DoubanSearchQuery> for DoubanSearchCommand {
    fn from(value: DoubanSearchQuery) -> Self {
        Self {
            query: value.q,
            page: value.page,
            page_size: value.page_size,
        }
    }
}

fn default_page() -> usize {
    1
}

fn default_page_size() -> usize {
    20
}

#[derive(Debug, Deserialize)]
pub(crate) struct DoubanLibraryQuery {
    #[serde(default)]
    force_refresh: bool,
    #[serde(default = "default_douban_library_limit")]
    limit: usize,
}

impl From<DoubanLibraryQuery> for DoubanLibraryCommand {
    fn from(value: DoubanLibraryQuery) -> Self {
        Self {
            force_refresh: value.force_refresh,
            limit: value.limit,
        }
    }
}

fn default_douban_library_limit() -> usize {
    200
}

#[derive(Debug, Deserialize)]
pub(crate) struct DoubanTagHistoryQuery {
    #[serde(default = "default_douban_tag_history_limit")]
    limit: usize,
}

impl From<DoubanTagHistoryQuery> for DoubanTagHistoryCommand {
    fn from(value: DoubanTagHistoryQuery) -> Self {
        Self { limit: value.limit }
    }
}

fn default_douban_tag_history_limit() -> usize {
    80
}

#[derive(Debug, Deserialize)]
pub(crate) struct DoubanQrQuery {
    session_id: String,
}

impl DoubanQrQuery {
    pub(crate) fn into_session_id(self) -> String {
        self.session_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DoubanLibraryItemDto {
    source: String,
    media_type: String,
    id: String,
    subject_id: String,
    title: String,
    url: String,
    abstract_text: String,
    abstract_2: String,
    cover_url: String,
    poster_url: String,
    status: String,
    status_label: String,
    date: String,
    comment: String,
    tags: Vec<String>,
    user_rating: Option<u8>,
}

impl From<DoubanLibraryItem> for DoubanLibraryItemDto {
    fn from(value: DoubanLibraryItem) -> Self {
        Self {
            source: value.source,
            media_type: value.media_type,
            id: value.id,
            subject_id: value.subject_id,
            title: value.title,
            url: value.url,
            abstract_text: value.abstract_text,
            abstract_2: value.abstract_2,
            cover_url: value.cover_url,
            poster_url: value.poster_url,
            status: value.status,
            status_label: value.status_label,
            date: value.date,
            comment: value.comment,
            tags: value.tags,
            user_rating: value.user_rating,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DoubanLibraryListDto {
    status: String,
    label: String,
    items: Vec<DoubanLibraryItemDto>,
    completeness: &'static str,
    fetched_pages: usize,
    truncated_by_limit: bool,
    end_observed: bool,
}

impl From<DoubanLibraryList> for DoubanLibraryListDto {
    fn from(value: DoubanLibraryList) -> Self {
        Self {
            status: value.status,
            label: value.label,
            items: value.items.into_iter().map(Into::into).collect(),
            completeness: match value.completeness {
                DoubanSnapshotCompleteness::Complete => "complete",
                DoubanSnapshotCompleteness::Partial => "partial",
            },
            fetched_pages: value.fetched_pages,
            truncated_by_limit: value.truncated_by_limit,
            end_observed: value.end_observed,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DoubanLibraryResponseDto {
    source: String,
    cached: bool,
    fetched_at: u64,
    ttl_seconds: u64,
    limit: usize,
    wish: DoubanLibraryListDto,
    collect: DoubanLibraryListDto,
}

impl From<DoubanLibraryOutcome> for DoubanLibraryResponseDto {
    fn from(value: DoubanLibraryOutcome) -> Self {
        Self {
            source: value.source,
            cached: value.cached,
            fetched_at: value.fetched_at,
            ttl_seconds: value.ttl_seconds,
            limit: value.limit,
            wish: value.wish.into(),
            collect: value.collect.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DoubanTagCountDto {
    tag: String,
    count: u64,
    category: String,
}

impl From<DoubanTagCount> for DoubanTagCountDto {
    fn from(value: DoubanTagCount) -> Self {
        Self {
            tag: value.tag,
            count: value.count,
            category: value.category,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DoubanTagCategoryDto {
    name: String,
    wanted_tag: String,
}

impl From<DoubanTagCategory> for DoubanTagCategoryDto {
    fn from(value: DoubanTagCategory) -> Self {
        Self {
            name: value.name,
            wanted_tag: value.wanted_tag,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DoubanTagHistoryResponseDto {
    source: String,
    cached: bool,
    updated_at: Option<u64>,
    tags: Vec<String>,
    tag_counts: Vec<DoubanTagCountDto>,
    subscription_categories: Vec<DoubanTagCategoryDto>,
}

impl From<DoubanTagHistoryOutcome> for DoubanTagHistoryResponseDto {
    fn from(value: DoubanTagHistoryOutcome) -> Self {
        Self {
            source: value.source,
            cached: value.cached,
            updated_at: value.updated_at,
            tags: value.tags,
            tag_counts: value.tag_counts.into_iter().map(Into::into).collect(),
            subscription_categories: value
                .subscription_categories
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DoubanQrStartResponseDto {
    session_id: String,
    image_url: String,
}

impl From<DoubanQrStartOutcome> for DoubanQrStartResponseDto {
    fn from(value: DoubanQrStartOutcome) -> Self {
        Self {
            session_id: value.session_id,
            image_url: value.image_url,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DoubanQrPollResponseDto {
    done: bool,
    login_status: String,
    message: String,
    description: String,
    cookie_saved: bool,
}

impl From<DoubanQrPollOutcome> for DoubanQrPollResponseDto {
    fn from(value: DoubanQrPollOutcome) -> Self {
        Self {
            done: value.done,
            login_status: value.login_status,
            message: value.message,
            description: value.description,
            cookie_saved: value.cookie_saved,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DoubanRatingDto {
    value: Option<f64>,
    count: Option<u64>,
    info: String,
    star_count: Option<f64>,
}

impl From<DoubanRating> for DoubanRatingDto {
    fn from(value: DoubanRating) -> Self {
        Self {
            value: value.value,
            count: value.count,
            info: value.info,
            star_count: value.star_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DoubanSearchItemDto {
    source: String,
    media_type: String,
    id: String,
    subject_id: String,
    title: String,
    url: String,
    abstract_text: String,
    abstract_2: String,
    cover_url: String,
    poster_url: String,
    rating: DoubanRatingDto,
    vote_average: Option<f64>,
}

impl From<DoubanSearchItem> for DoubanSearchItemDto {
    fn from(value: DoubanSearchItem) -> Self {
        Self {
            source: value.source,
            media_type: value.media_type,
            id: value.id,
            subject_id: value.subject_id,
            title: value.title,
            url: value.url,
            abstract_text: value.abstract_text,
            abstract_2: value.abstract_2,
            cover_url: value.cover_url,
            poster_url: value.poster_url,
            rating: value.rating.into(),
            vote_average: value.vote_average,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct DoubanSearchResponseDto {
    items: Vec<DoubanSearchItemDto>,
    page: usize,
    page_size: usize,
    has_more: bool,
}

impl From<DoubanSearchOutcome> for DoubanSearchResponseDto {
    fn from(value: DoubanSearchOutcome) -> Self {
        Self {
            items: value.items.into_iter().map(Into::into).collect(),
            page: value.page,
            page_size: value.page_size,
            has_more: value.has_more,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DoubanInterestDto {
    Wish,
    Collect,
}

impl From<DoubanInterest> for DoubanInterestDto {
    fn from(value: DoubanInterest) -> Self {
        match value {
            DoubanInterest::Wish => Self::Wish,
            DoubanInterest::Collect => Self::Collect,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct DoubanSubjectDetailDto {
    source: String,
    media_type: String,
    id: String,
    subject_id: String,
    url: String,
    title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    original_title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    aka: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    languages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    countries: Vec<String>,
    image: String,
    poster_url: String,
    directors: Vec<String>,
    writers: Vec<String>,
    actors: Vec<String>,
    genres: Vec<String>,
    date_published: String,
    duration: String,
    summary: String,
    rating: DoubanRatingDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_interest: Option<DoubanInterestDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_rating: Option<u8>,
}

impl From<DoubanSubjectDetail> for DoubanSubjectDetailDto {
    fn from(value: DoubanSubjectDetail) -> Self {
        Self {
            source: value.source,
            media_type: value.media_type,
            id: value.id,
            subject_id: value.subject_id,
            url: value.url,
            title: value.title,
            original_title: value.original_title,
            aka: value.aka,
            languages: value.languages,
            countries: value.countries,
            image: value.image,
            poster_url: value.poster_url,
            directors: value.directors,
            writers: value.writers,
            actors: value.actors,
            genres: value.genres,
            date_published: value.date_published,
            duration: value.duration,
            summary: value.summary,
            rating: value.rating.into(),
            user_interest: value.user_interest.map(Into::into),
            user_rating: value.user_rating,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DoubanInterestRequestDto {
    Wish,
    Collect,
}

impl From<DoubanInterestRequestDto> for DoubanInterest {
    fn from(value: DoubanInterestRequestDto) -> Self {
        match value {
            DoubanInterestRequestDto::Wish => Self::Wish,
            DoubanInterestRequestDto::Collect => Self::Collect,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct MarkDoubanInterestRequestDto {
    interest: DoubanInterestRequestDto,
    #[serde(default)]
    rating: Option<u8>,
    #[serde(default)]
    tags: String,
}

impl MarkDoubanInterestRequestDto {
    pub(crate) fn into_command(self, subject_id: String) -> MarkDoubanInterestCommand {
        MarkDoubanInterestCommand {
            subject_id,
            interest: self.interest.into(),
            rating: self.rating,
            tags: self.tags,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct DoubanInterestResponseDto {
    ok: bool,
    interest: DoubanInterestDto,
    rating: Option<u8>,
    tags: String,
}

impl From<DoubanInterestResult> for DoubanInterestResponseDto {
    fn from(value: DoubanInterestResult) -> Self {
        Self {
            ok: value.ok,
            interest: value.interest.into(),
            rating: value.rating,
            tags: value.tags,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rating() -> DoubanRating {
        DoubanRating {
            value: Some(8.8),
            count: Some(42),
            info: "8.8".to_string(),
            star_count: Some(4.5),
        }
    }

    #[test]
    fn search_response_is_closed_and_uses_one_canonical_items_collection() {
        let item = DoubanSearchItem {
            source: "douban".to_string(),
            media_type: "douban".to_string(),
            id: "1292052".to_string(),
            subject_id: "1292052".to_string(),
            title: "肖申克的救赎".to_string(),
            url: "https://movie.douban.com/subject/1292052/".to_string(),
            abstract_text: "1994 / 美国".to_string(),
            abstract_2: String::new(),
            cover_url: "/api/douban/image?url=cover".to_string(),
            poster_url: "/api/douban/image?url=cover".to_string(),
            rating: rating(),
            vote_average: Some(8.8),
        };
        let value = serde_json::to_value(DoubanSearchResponseDto::from(DoubanSearchOutcome {
            items: vec![item],
            page: 1,
            page_size: 20,
            has_more: false,
        }))
        .unwrap();

        assert_eq!(value["items"][0]["subject_id"], "1292052");
        assert_eq!(value["items"][0].as_object().unwrap().len(), 12);
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert!(value.get("movies").is_none());
        assert!(value.get("tv").is_none());
    }

    #[test]
    fn subject_detail_preserves_optional_field_omission() {
        let value = serde_json::to_value(DoubanSubjectDetailDto::from(DoubanSubjectDetail {
            source: "douban".to_string(),
            media_type: "douban".to_string(),
            id: "1292052".to_string(),
            subject_id: "1292052".to_string(),
            url: "https://movie.douban.com/subject/1292052/".to_string(),
            title: "肖申克的救赎".to_string(),
            original_title: String::new(),
            aka: Vec::new(),
            languages: Vec::new(),
            countries: Vec::new(),
            image: String::new(),
            poster_url: String::new(),
            directors: Vec::new(),
            writers: Vec::new(),
            actors: Vec::new(),
            genres: Vec::new(),
            date_published: String::new(),
            duration: String::new(),
            summary: String::new(),
            rating: rating(),
            user_interest: None,
            user_rating: None,
        }))
        .unwrap();

        assert!(value.get("original_title").is_none());
        assert!(value.get("aka").is_none());
        assert!(value.get("user_interest").is_none());
        assert!(value.get("user_rating").is_none());
    }

    #[test]
    fn qr_poll_response_cannot_serialize_provider_cookie() {
        let value = serde_json::to_value(DoubanQrPollResponseDto::from(DoubanQrPollOutcome {
            done: true,
            login_status: "done".to_string(),
            message: "ok".to_string(),
            description: "saved".to_string(),
            cookie_saved: true,
        }))
        .unwrap();

        assert_eq!(value["cookie_saved"], true);
        assert!(value.get("cookie_header").is_none());
        assert_eq!(value.as_object().unwrap().len(), 5);
    }
}
