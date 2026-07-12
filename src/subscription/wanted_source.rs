use crate::clients::douban::DoubanClient;
use crate::douban::{self, DoubanLibraryStatus, SnapshotCompleteness};

use super::worker::{
    WantedItem, WantedSnapshot, WantedSnapshotCompleteness, WantedSnapshotMetadata, WantedSource,
    WantedSourceError, WantedSourceFuture,
};

#[derive(Clone)]
pub(crate) struct DoubanWantedSource {
    client: DoubanClient,
}

impl DoubanWantedSource {
    pub(crate) fn new(client: DoubanClient) -> Self {
        Self { client }
    }
}

impl WantedSource for DoubanWantedSource {
    fn fetch_wanted(&self, cookie_header: String, limit: usize) -> WantedSourceFuture {
        let client = self.client.clone();
        Box::pin(async move {
            let list = douban::library(&client, &cookie_header, DoubanLibraryStatus::Wish, limit)
                .await
                .map_err(|error| WantedSourceError::new(error.to_string()))?;
            Ok(WantedSnapshot {
                items: list
                    .items
                    .into_iter()
                    .map(|item| WantedItem {
                        subject_id: item.subject_id,
                        title: item.title,
                        abstract_text: item.abstract_text,
                        abstract_2: item.abstract_2,
                        cover_url: item.cover_url,
                        poster_url: item.poster_url,
                        date: item.date,
                        tags: item.tags,
                    })
                    .collect(),
                metadata: WantedSnapshotMetadata {
                    completeness: match list.snapshot.completeness {
                        SnapshotCompleteness::Complete => WantedSnapshotCompleteness::Complete,
                        SnapshotCompleteness::Partial => WantedSnapshotCompleteness::Partial,
                    },
                    fetched_pages: list.snapshot.fetched_pages,
                    truncated_by_limit: list.snapshot.truncated_by_limit,
                    end_observed: list.snapshot.end_observed,
                },
            })
        })
    }
}
