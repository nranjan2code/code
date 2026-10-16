//! Map tiles behind the owner's allowance (docs/design/96-answer-projection-gaps.md P5).
//!
//! A map card draws its places with no network. Street tiles come only
//! through this route, only while `[maps] tiles` in the Shared layer is on,
//! and only from the host the owner's tile template names: the request
//! carries no cookie or referrer, follows no redirect and takes only an
//! image (`vak_tools::webfetch::guarded_get_image`).

use axum::Json;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// The most bytes one tile may be: a bound on what the server holds, far
/// above any tile a map server sends.
const TILE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// The template's address for one tile, when `z`, `x` and `y` name a tile
/// of the Web Mercator scheme.
fn tile_address(template: &str, z: u32, x: u32, y: u32) -> Option<String> {
    let side = 1u64.checked_shl(z).filter(|_| z <= 30)?;
    if u64::from(x) >= side || u64::from(y) >= side {
        return None;
    }
    Some(
        template
            .replace("{z}", &z.to_string())
            .replace("{x}", &x.to_string())
            .replace("{y}", &y.to_string()),
    )
}

pub(crate) async fn tile(Path((z, x, y)): Path<(u32, u32, String)>) -> Response {
    let maps = vak_config::shared_maps();
    if !maps.tiles {
        return (StatusCode::NOT_FOUND, "map tiles are off").into_response();
    }
    let Some(address) = y
        .strip_suffix(".png")
        .unwrap_or(&y)
        .parse()
        .ok()
        .and_then(|y| tile_address(&maps.tile_url, z, x, y))
    else {
        return (StatusCode::BAD_REQUEST, "no such tile").into_response();
    };
    match vak_tools::webfetch::guarded_get_image(&address, TILE_MAX_BYTES).await {
        Ok(fetched) if fetched.status == 200 => (
            [
                (header::CONTENT_TYPE, fetched.content_type),
                (header::CACHE_CONTROL, "private, max-age=86400".into()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            ],
            fetched.body,
        )
            .into_response(),
        Ok(fetched) => {
            tracing::debug!(status = fetched.status, "map tile refused upstream");
            StatusCode::BAD_GATEWAY.into_response()
        }
        Err(_) => {
            tracing::debug!(error_kind = "fetch", "map tile not fetched");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct MapsBody {
    #[serde(default)]
    tiles: Option<bool>,
    /// An `https` template with `{z}`, `{x}` and `{y}`; empty restores the default.
    #[serde(default)]
    tile_url: Option<String>,
    /// Empty restores the default.
    #[serde(default)]
    attribution: Option<String>,
}

/// `[maps]` lives in the Shared layer alone: map tiles are the owner's
/// allowance, whatever folder they work in.
pub(crate) async fn put_settings(Json(body): Json<MapsBody>) -> Response {
    let Some(path) = vak_config::global_path() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "user home unavailable").into_response();
    };
    if let Some(problem) = body
        .tile_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .and_then(vak_config::tile_url_problem)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": problem})),
        )
            .into_response();
    }
    if let Err(error) = vak_config::persist_maps(
        path,
        body.tiles,
        body.tile_url.as_deref(),
        body.attribution.as_deref(),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        )
            .into_response();
    }
    Json(vak_config::shared_maps()).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_address_is_only_a_tile_of_the_scheme() {
        let template = vak_config::DEFAULT_TILE_URL;
        assert_eq!(
            tile_address(template, 2, 3, 1).as_deref(),
            Some("https://tile.openstreetmap.org/2/3/1.png")
        );
        assert!(tile_address(template, 2, 4, 0).is_none());
        assert!(tile_address(template, 0, 0, 1).is_none());
        assert!(tile_address(template, 31, 0, 0).is_none());
    }

    #[tokio::test]
    async fn no_tile_is_fetched_while_the_owner_has_not_allowed_it() {
        vak_config::paths::isolate_home_for_tests();
        let response = tile(Path((1, 0, "0.png".into()))).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
