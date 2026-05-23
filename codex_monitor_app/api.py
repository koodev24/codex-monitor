import json
import ssl
import sys
import urllib.request

import certifi

from .config import USAGE_API_URL
from .models import UsageResponse


if sys.platform.startswith("win"):
    PLATFORM_HEADER = '"Windows"'
    USER_AGENT = (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/135.0.0.0 Safari/537.36"
    )
else:
    PLATFORM_HEADER = '"macOS"'
    USER_AGENT = (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/135.0.0.0 Safari/537.36"
    )


BROWSER_LIKE_HEADERS = {
    "Accept": "application/json, text/plain, */*",
    "Accept-Language": "en-US,en;q=0.9",
    "Origin": "https://chatgpt.com",
    "Referer": "https://chatgpt.com/",
    "Sec-CH-UA": '"Google Chrome";v="135", "Chromium";v="135", "Not.A/Brand";v="8"',
    "Sec-CH-UA-Mobile": "?0",
    "Sec-CH-UA-Platform": PLATFORM_HEADER,
    "Sec-Fetch-Dest": "empty",
    "Sec-Fetch-Mode": "cors",
    "Sec-Fetch-Site": "same-origin",
    "User-Agent": USER_AGENT,
}


class UsageApiClient:
    def __init__(self, usage_api_url: str = USAGE_API_URL):
        self.usage_api_url = usage_api_url

    def fetch_usage(self, jwt: str) -> UsageResponse:
        request = urllib.request.Request(
            self.usage_api_url,
            headers={
                **BROWSER_LIKE_HEADERS,
                "Authorization": f"Bearer {jwt}",
            },
        )

        context = ssl.create_default_context(cafile=certifi.where())
        with urllib.request.urlopen(request, context=context, timeout=15) as response:
            return json.loads(response.read().decode("utf-8"))
