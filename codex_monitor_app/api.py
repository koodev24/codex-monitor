import json
import ssl
import urllib.request

import certifi

from .config import (
    AUTH_REFRESH_CLIENT_ID,
    AUTH_REFRESH_URL,
    RESET_CREDITS_API_URL,
    USAGE_API_URL,
)
from .models import ResetCreditsPayload, UsageResponse

BROWSER_LIKE_HEADERS = {
    "Accept": "application/json, text/plain, */*",
    "Accept-Language": "en-US,en;q=0.9",
    "Origin": "https://chatgpt.com",
    "Referer": "https://chatgpt.com/",
    "Sec-CH-UA": '"Google Chrome";v="135", "Chromium";v="135", "Not.A/Brand";v="8"',
    "Sec-CH-UA-Mobile": "?0",
    "Sec-CH-UA-Platform": '"macOS"',
    "Sec-Fetch-Dest": "empty",
    "Sec-Fetch-Mode": "cors",
    "Sec-Fetch-Site": "same-origin",
    "User-Agent": (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/135.0.0.0 Safari/537.36"
    ),
}


class UsageApiClient:
    def __init__(
        self,
        usage_api_url: str = USAGE_API_URL,
        reset_credits_api_url: str = RESET_CREDITS_API_URL,
    ):
        self.usage_api_url = usage_api_url
        self.reset_credits_api_url = reset_credits_api_url

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

    def fetch_reset_credits(self, jwt: str, account_id: str) -> ResetCreditsPayload:
        request = urllib.request.Request(
            self.reset_credits_api_url,
            headers={
                **BROWSER_LIKE_HEADERS,
                "Authorization": f"Bearer {jwt}",
                "ChatGPT-Account-ID": account_id,
            },
        )

        context = ssl.create_default_context(cafile=certifi.where())
        with urllib.request.urlopen(request, context=context, timeout=15) as response:
            return json.loads(response.read().decode("utf-8"))


class AuthRefreshClient:
    def __init__(
        self,
        auth_refresh_url: str = AUTH_REFRESH_URL,
        client_id: str = AUTH_REFRESH_CLIENT_ID,
    ):
        self.auth_refresh_url = auth_refresh_url
        self.client_id = client_id

    def refresh_tokens(self, refresh_token: str) -> dict:
        body = json.dumps(
            {
                "client_id": self.client_id,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            }
        ).encode("utf-8")
        request = urllib.request.Request(
            self.auth_refresh_url,
            data=body,
            headers={
                **BROWSER_LIKE_HEADERS,
                "Accept": "application/json",
                "Content-Type": "application/json",
            },
            method="POST",
        )

        context = ssl.create_default_context(cafile=certifi.where())
        with urllib.request.urlopen(request, context=context, timeout=15) as response:
            return json.loads(response.read().decode("utf-8"))
