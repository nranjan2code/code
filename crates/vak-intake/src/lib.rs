//! Intake connectors (plan M6.5, docs/design/76-intake-and-knowledge.md):
//! the closed set of source kinds, the one request each makes, bounded
//! parsers for what comes back, and detection that labels an item and never
//! drops it. Nothing here touches the network or a file: the host fetches,
//! and the parsers run in the network-denied broker worker (invariant 14),
//! because what a source returns is hostile bytes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod detect;
mod parse;
mod text;

pub use detect::{Detection, Disposition, detect};
pub use parse::parse;

/// The most bytes one fetch may return.
pub const MAX_FETCH_BYTES: usize = 4 * 1024 * 1024;
/// The most items one fetch yields; later ones wait for the next poll.
pub const MAX_ITEMS: usize = 200;
/// The most characters of an item's text, after its markup is stripped.
pub const MAX_TEXT_CHARS: usize = 16 * 1024;
const MAX_TITLE_CHARS: usize = 500;
const MAX_URL_CHARS: usize = 2048;

/// What a source fetches. A closed, host-owned set: a workspace never
/// contributes one (doc 76 §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Connector {
    /// An RSS or Atom feed.
    Rss { url: String },
    /// A YouTube channel's uploads feed.
    Youtube { channel_id: String },
    HackerNews {
        #[serde(default)]
        list: HackerNewsList,
    },
    Reddit {
        subreddit: String,
        #[serde(default)]
        sort: RedditSort,
    },
    Lobsters {
        #[serde(default)]
        list: LobstersList,
    },
    /// Any HTTP endpoint answering in `format`.
    Http {
        url: String,
        #[serde(default)]
        format: HttpFormat,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HackerNewsList {
    #[default]
    FrontPage,
    Newest,
    Ask,
    Show,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedditSort {
    #[default]
    Hot,
    New,
    Top,
    Rising,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LobstersList {
    #[default]
    Hottest,
    Newest,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpFormat {
    /// RSS or Atom.
    #[default]
    Feed,
    /// A JSON array of objects, or an object whose `items` is one.
    Json,
    /// The whole body is one item.
    Text,
}

/// Why a connector or what it fetched was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntakeError {
    #[error("{0}")]
    Invalid(String),
    #[error("the response could not be read: {0}")]
    Unreadable(String),
}

impl Connector {
    /// The connector's kind, as its tag names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Rss { .. } => "rss",
            Self::Youtube { .. } => "youtube",
            Self::HackerNews { .. } => "hacker_news",
            Self::Reddit { .. } => "reddit",
            Self::Lobsters { .. } => "lobsters",
            Self::Http { .. } => "http",
        }
    }

    /// Refuses a connector whose fields could address something other than
    /// what its kind fetches.
    pub fn validate(&self) -> Result<(), IntakeError> {
        match self {
            Self::Rss { url } | Self::Http { url, .. } => check_url(url),
            Self::Youtube { channel_id } => {
                let ok = channel_id.len() == 24
                    && channel_id.starts_with("UC")
                    && channel_id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
                if ok {
                    Ok(())
                } else {
                    Err(IntakeError::Invalid(
                        "a YouTube channel id is 24 characters starting with UC".into(),
                    ))
                }
            }
            Self::Reddit { subreddit, .. } => {
                let ok = (2..=21).contains(&subreddit.len())
                    && subreddit
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_');
                if ok {
                    Ok(())
                } else {
                    Err(IntakeError::Invalid(
                        "a subreddit name is 2 to 21 letters, digits or underscores".into(),
                    ))
                }
            }
            Self::HackerNews { .. } | Self::Lobsters { .. } => Ok(()),
        }
    }

    /// The one URL a poll fetches.
    pub fn url(&self) -> String {
        match self {
            Self::Rss { url } | Self::Http { url, .. } => url.clone(),
            Self::Youtube { channel_id } => {
                format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}")
            }
            Self::HackerNews { list } => {
                let (path, tags) = match list {
                    HackerNewsList::FrontPage => ("search", "front_page"),
                    HackerNewsList::Newest => ("search_by_date", "story"),
                    HackerNewsList::Ask => ("search_by_date", "ask_hn"),
                    HackerNewsList::Show => ("search_by_date", "show_hn"),
                };
                format!("https://hn.algolia.com/api/v1/{path}?tags={tags}&hitsPerPage=50")
            }
            Self::Reddit { subreddit, sort } => {
                let sort = match sort {
                    RedditSort::Hot => "hot",
                    RedditSort::New => "new",
                    RedditSort::Top => "top",
                    RedditSort::Rising => "rising",
                };
                format!("https://www.reddit.com/r/{subreddit}/{sort}.json?limit=50&raw_json=1")
            }
            Self::Lobsters { list } => match list {
                LobstersList::Hottest => "https://lobste.rs/hottest.json".into(),
                LobstersList::Newest => "https://lobste.rs/newest.json".into(),
            },
        }
    }
}

fn check_url(raw: &str) -> Result<(), IntakeError> {
    if raw.len() > MAX_URL_CHARS {
        return Err(IntakeError::Invalid(
            "the URL is longer than 2048 characters".into(),
        ));
    }
    let url = url::Url::parse(raw)
        .map_err(|error| IntakeError::Invalid(format!("invalid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(IntakeError::Invalid(
            "only http and https URLs can be fetched".into(),
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(IntakeError::Invalid("the URL has no host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(IntakeError::Invalid(
            "a source URL carries no credentials".into(),
        ));
    }
    Ok(())
}

/// One observation a connector parsed: plain text, bounded, never markup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// What names the item at its source (a guid, an id, a link); a re-poll
    /// that sees the same key sees the same item.
    pub key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

impl Item {
    /// The hex SHA-256 of the key: the stable part of an item's id.
    pub fn key_digest(&self) -> String {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(self.key.as_bytes()))
    }
}

/// Builds an item from raw fields, stripping markup and applying every
/// bound; `None` when it has neither a title nor text worth keeping.
pub(crate) fn item(
    key: Option<String>,
    title: &str,
    link: Option<&str>,
    author: Option<&str>,
    published: Option<DateTime<Utc>>,
    body: &str,
) -> Option<Item> {
    let title = text::bounded(&text::strip_markup(title), MAX_TITLE_CHARS);
    let body = text::bounded(&text::strip_markup(body), MAX_TEXT_CHARS);
    if title.is_empty() && body.is_empty() {
        return None;
    }
    let link = link
        .map(str::trim)
        .filter(|link| check_url(link).is_ok())
        .map(str::to_string);
    let key = key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .or_else(|| link.clone())
        .unwrap_or_else(|| format!("{title}\n{body}"));
    Some(Item {
        key: text::bounded(&key, MAX_URL_CHARS),
        title,
        link,
        author: author
            .map(|author| text::bounded(&text::strip_markup(author), MAX_TITLE_CHARS))
            .filter(|author| !author.is_empty()),
        published,
        text: body,
    })
}
