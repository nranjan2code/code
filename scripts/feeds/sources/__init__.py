"""Source driver registry."""

from sources.rss import RssDriver
from sources.youtube import YouTubeDriver
from sources.aggregator import AggregatorDriver
from sources.custom import CustomHttpDriver

DRIVERS: dict[str, type] = {
    "rss": RssDriver,
    "youtube": YouTubeDriver,
    "aggregator": AggregatorDriver,
    "custom": CustomHttpDriver,
}


def get_driver(source_type: str):
    cls = DRIVERS.get(source_type)
    if not cls:
        raise ValueError(f"Unknown source type: {source_type}")
    return cls()
