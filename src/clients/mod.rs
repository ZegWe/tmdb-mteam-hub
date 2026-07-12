//! Outbound provider clients and their enforced transport policies.

pub mod douban;
pub mod http;
pub mod mteam;
pub mod qbittorrent;
pub mod tmdb;

use douban::DoubanClient;
use http::ClientError;
use mteam::MteamClient;
use tmdb::TmdbClient;

/// Long-lived clients assembled once with the application state.
#[derive(Clone, Debug)]
pub(crate) struct UpstreamClients {
    pub(crate) douban: DoubanClient,
    pub(crate) mteam: MteamClient,
    pub(crate) tmdb: TmdbClient,
}

impl UpstreamClients {
    pub(crate) fn new() -> Result<Self, ClientError> {
        Ok(Self {
            douban: DoubanClient::new()?,
            mteam: MteamClient::new()?,
            tmdb: TmdbClient::new()?,
        })
    }
}
