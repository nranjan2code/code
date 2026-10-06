//! What a fetch returned, parsed into items. Every parser is bounded: in
//! bytes (the fetch), in XML depth and attributes (vak-ooxml's walk, which
//! refuses a DOCTYPE and never expands an entity), in items and in text.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::{Connector, HttpFormat, IntakeError, Item, MAX_FETCH_BYTES, MAX_ITEMS, item};

/// Parses `data`, the body a fetch of `connector` returned.
pub fn parse(connector: &Connector, data: &[u8]) -> Result<Vec<Item>, IntakeError> {
    if data.len() > MAX_FETCH_BYTES {
        return Err(IntakeError::Unreadable(
            "the response is larger than 4 MiB".into(),
        ));
    }
    let mut items = match connector {
        Connector::Rss { .. }
        | Connector::Youtube { .. }
        | Connector::Http {
            format: HttpFormat::Feed,
            ..
        } => feed(data)?,
        Connector::HackerNews { .. } => hacker_news(&json(data)?),
        Connector::Reddit { .. } => reddit(&json(data)?),
        Connector::Lobsters { .. } => lobsters(&json(data)?),
        Connector::Http {
            format: HttpFormat::Json,
            ..
        } => generic_json(&json(data)?),
        Connector::Http {
            format: HttpFormat::Text,
            ..
        } => {
            let text = String::from_utf8_lossy(data);
            item(None, "", None, None, None, &text)
                .into_iter()
                .collect()
        }
    };
    items.truncate(MAX_ITEMS);
    Ok(items)
}

fn json(data: &[u8]) -> Result<Value, IntakeError> {
    serde_json::from_slice(data).map_err(|error| IntakeError::Unreadable(error.to_string()))
}

fn date(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    DateTime::parse_from_rfc3339(raw)
        .or_else(|_| DateTime::parse_from_rfc2822(raw))
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn epoch(value: &Value) -> Option<DateTime<Utc>> {
    let seconds = value
        .as_i64()
        .or_else(|| value.as_f64().map(|f| f as i64))?;
    Utc.timestamp_opt(seconds, 0).single()
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// RSS 2.0, RSS 1.0 (RDF) and Atom, YouTube's uploads feed among them.
fn feed(data: &[u8]) -> Result<Vec<Item>, IntakeError> {
    #[derive(Default)]
    struct Entry {
        key: Option<String>,
        title: String,
        link: Option<String>,
        author: Option<String>,
        published: Option<String>,
        updated: Option<String>,
        body: String,
        summary: String,
    }
    let limits = vak_ooxml::Limits {
        max_xml_depth: 64,
        max_attributes: 64,
        ..vak_ooxml::Limits::default()
    };
    let mut items = Vec::new();
    let mut entry: Option<Entry> = None;
    let mut path: Vec<String> = Vec::new();
    let mut saw_root = false;
    let walked = vak_ooxml::xml::walk(data, "feed", &limits, |event| {
        use vak_ooxml::xml::XmlEvent;
        match event {
            XmlEvent::Open(element) => {
                let local = element.local().to_ascii_lowercase();
                if path.is_empty() {
                    saw_root = matches!(local.as_str(), "rss" | "feed" | "rdf");
                }
                if matches!(local.as_str(), "item" | "entry") && entry.is_none() {
                    entry = Some(Entry::default());
                } else if let Some(entry) = entry.as_mut()
                    && local == "link"
                    && let Some(href) = element.attr_unprefixed("href")
                {
                    let alternate = element
                        .attr_unprefixed("rel")
                        .is_none_or(|rel| rel == "alternate");
                    if alternate && entry.link.is_none() {
                        entry.link = Some(href.to_string());
                    }
                }
                path.push(local);
            }
            XmlEvent::Close(_) => {
                let closed = path.pop().unwrap_or_default();
                if matches!(closed.as_str(), "item" | "entry")
                    && !path.iter().any(|open| open == "item" || open == "entry")
                    && let Some(done) = entry.take()
                    && items.len() < MAX_ITEMS
                {
                    let body = if done.body.is_empty() {
                        done.summary
                    } else {
                        done.body
                    };
                    let published = done
                        .published
                        .as_deref()
                        .or(done.updated.as_deref())
                        .and_then(date);
                    items.extend(item(
                        done.key,
                        &done.title,
                        done.link.as_deref(),
                        done.author.as_deref(),
                        published,
                        &body,
                    ));
                }
            }
            XmlEvent::Text(text) => {
                let Some(entry) = entry.as_mut() else {
                    return Ok(());
                };
                let Some(at) = path
                    .iter()
                    .rposition(|open| open == "item" || open == "entry")
                else {
                    return Ok(());
                };
                let inner: Vec<&str> = path[at + 1..].iter().map(String::as_str).collect();
                match inner.as_slice() {
                    ["title"] => entry.title.push_str(&text),
                    ["link"] if entry.link.is_none() || text.trim().starts_with("http") => {
                        entry.link = Some(text.trim().to_string());
                    }
                    ["guid"] | ["id"] | ["videoid"] => {
                        entry.key = Some(text.trim().to_string());
                    }
                    ["pubdate"] | ["published"] | ["date"] => {
                        entry.published = Some(text.trim().to_string());
                    }
                    ["updated"] => entry.updated = Some(text.trim().to_string()),
                    ["creator"] | ["author"] | ["author", "name"] => {
                        entry.author.get_or_insert_with(String::new).push_str(&text);
                    }
                    ["encoded"] | ["content"] => entry.body.push_str(&text),
                    ["description"] | ["summary"] | ["group", "description"] => {
                        entry.summary.push_str(&text);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    });
    walked.map_err(|error| IntakeError::Unreadable(error.to_string()))?;
    if !saw_root {
        return Err(IntakeError::Unreadable("not an RSS or Atom feed".into()));
    }
    Ok(items)
}

fn hacker_news(value: &Value) -> Vec<Item> {
    let hits = value.get("hits").and_then(Value::as_array);
    hits.into_iter()
        .flatten()
        .filter_map(|hit| {
            let id = string(hit, "objectID")?;
            let discussion = format!("https://news.ycombinator.com/item?id={id}");
            item(
                Some(format!("hn:{id}")),
                string(hit, "title").unwrap_or_default(),
                Some(string(hit, "url").unwrap_or(&discussion)),
                string(hit, "author"),
                string(hit, "created_at").and_then(date),
                string(hit, "story_text").unwrap_or_default(),
            )
        })
        .collect()
}

fn reddit(value: &Value) -> Vec<Item> {
    let children = value.pointer("/data/children").and_then(Value::as_array);
    children
        .into_iter()
        .flatten()
        .filter_map(|child| {
            let post = child.get("data")?;
            let id = string(post, "id")?;
            let permalink = string(post, "permalink").map(|p| format!("https://www.reddit.com{p}"));
            item(
                Some(format!("reddit:{id}")),
                string(post, "title").unwrap_or_default(),
                string(post, "url").or(permalink.as_deref()),
                string(post, "author"),
                post.get("created_utc").and_then(epoch),
                string(post, "selftext").unwrap_or_default(),
            )
        })
        .collect()
}

fn lobsters(value: &Value) -> Vec<Item> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|story| {
            let id = string(story, "short_id")?;
            let submitter = story.get("submitter_user").and_then(|user| {
                user.as_str()
                    .or_else(|| user.get("username").and_then(Value::as_str))
            });
            let link = string(story, "url").or_else(|| string(story, "comments_url"));
            item(
                Some(format!("lobsters:{id}")),
                string(story, "title").unwrap_or_default(),
                link,
                submitter,
                string(story, "created_at").and_then(date),
                string(story, "description_plain")
                    .or_else(|| string(story, "description"))
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn generic_json(value: &Value) -> Vec<Item> {
    let list = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array));
    list.into_iter()
        .flatten()
        .filter_map(|entry| {
            let first = |keys: &[&str]| keys.iter().find_map(|key| string(entry, key));
            let key = first(&["id", "guid"]).map(str::to_string).or_else(|| {
                entry
                    .get("id")
                    .filter(|id| id.is_number())
                    .map(Value::to_string)
            });
            item(
                key,
                first(&["title", "name"]).unwrap_or_default(),
                first(&["url", "link"]),
                first(&["author", "by"]),
                first(&["published", "date", "created_at", "updated"]).and_then(date),
                first(&["content", "text", "summary", "description", "body"]).unwrap_or_default(),
            )
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{HackerNewsList, LobstersList, RedditSort};

    #[test]
    fn rss_items_parse_with_plain_text() {
        let rss = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
<channel><title>Site</title>
<item><title>First &amp; best</title><link>https://example.com/1</link>
<guid>g-1</guid><pubDate>Tue, 06 Oct 2026 09:00:00 GMT</pubDate>
<dc:creator>Ada</dc:creator>
<description>&lt;p&gt;Short&lt;/p&gt;</description>
<content:encoded><![CDATA[<p>The <b>whole</b> story</p>]]></content:encoded></item>
<item><title>Second</title><link>https://example.com/2</link></item>
</channel></rss>"#;
        let items = parse(&Connector::Rss { url: String::new() }, rss).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "g-1");
        assert_eq!(items[0].title, "First & best");
        assert_eq!(items[0].text, "The whole story");
        assert_eq!(items[0].author.as_deref(), Some("Ada"));
        assert!(items[0].published.is_some());
        assert_eq!(items[1].key, "https://example.com/2");
    }

    #[test]
    fn atom_and_youtube_entries_parse() {
        let atom = br#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/">
<entry><id>yt:video:abc</id><yt:videoId>abc</yt:videoId><title>A video</title>
<link rel="alternate" href="https://www.youtube.com/watch?v=abc"/>
<author><name>Channel</name></author><published>2026-10-01T10:00:00+00:00</published>
<media:group><media:description>What it shows</media:description></media:group></entry></feed>"#;
        let items = parse(
            &Connector::Youtube {
                channel_id: String::new(),
            },
            atom,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "abc");
        assert_eq!(
            items[0].link.as_deref(),
            Some("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(items[0].text, "What it shows");
        assert_eq!(items[0].author.as_deref(), Some("Channel"));
    }

    #[test]
    fn hostile_feeds_are_refused_not_expanded() {
        let doctype = br#"<?xml version="1.0"?><!DOCTYPE rss [<!ENTITY x "boom">]><rss><channel><item><title>&x;</title></item></channel></rss>"#;
        assert!(parse(&Connector::Rss { url: String::new() }, doctype).is_err());
        let deep = format!("<rss>{}{}</rss>", "<a>".repeat(200), "</a>".repeat(200));
        assert!(parse(&Connector::Rss { url: String::new() }, deep.as_bytes()).is_err());
        assert!(
            parse(
                &Connector::Rss { url: String::new() },
                b"<html><body>no</body></html>"
            )
            .is_err()
        );
        let huge = vec![b' '; MAX_FETCH_BYTES + 1];
        assert!(parse(&Connector::Rss { url: String::new() }, &huge).is_err());
    }

    #[test]
    fn aggregators_parse_their_json() {
        let hn = br#"{"hits":[{"objectID":"42","title":"Show HN: a thing","url":null,"author":"pg","created_at":"2026-10-01T10:00:00.000Z"}]}"#;
        let items = parse(
            &Connector::HackerNews {
                list: HackerNewsList::Show,
            },
            hn,
        )
        .unwrap();
        assert_eq!(items[0].key, "hn:42");
        assert_eq!(
            items[0].link.as_deref(),
            Some("https://news.ycombinator.com/item?id=42")
        );
        let reddit = br#"{"data":{"children":[{"data":{"id":"x1","title":"Post","url":"https://example.com/p","author":"u","created_utc":1790000000.0,"selftext":"body"}}]}}"#;
        let items = parse(
            &Connector::Reddit {
                subreddit: "rust".into(),
                sort: RedditSort::Hot,
            },
            reddit,
        )
        .unwrap();
        assert_eq!(
            (items[0].key.as_str(), items[0].text.as_str()),
            ("reddit:x1", "body")
        );
        let lobsters = br#"[{"short_id":"ab","title":"Story","url":"","comments_url":"https://lobste.rs/s/ab","submitter_user":{"username":"me"},"created_at":"2026-10-01T10:00:00.000-05:00"}]"#;
        let items = parse(
            &Connector::Lobsters {
                list: LobstersList::Newest,
            },
            lobsters,
        )
        .unwrap();
        assert_eq!(items[0].link.as_deref(), Some("https://lobste.rs/s/ab"));
        assert_eq!(items[0].author.as_deref(), Some("me"));
    }

    #[test]
    fn http_json_and_text_parse() {
        let json = br#"{"items":[{"id":7,"title":"Seven","link":"https://example.com/7","summary":"<i>s</i>"}]}"#;
        let connector = Connector::Http {
            url: String::new(),
            format: HttpFormat::Json,
        };
        let items = parse(&connector, json).unwrap();
        assert_eq!((items[0].key.as_str(), items[0].text.as_str()), ("7", "s"));
        let connector = Connector::Http {
            url: String::new(),
            format: HttpFormat::Text,
        };
        let items = parse(&connector, b"status: all good").unwrap();
        assert_eq!(items[0].text, "status: all good");
    }
}
