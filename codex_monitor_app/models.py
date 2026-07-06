from typing import Any, Dict, List, Optional, TypedDict


class RateLimitWindow(TypedDict, total=False):
    used_percent: float
    limit_window_seconds: int
    reset_after_seconds: int
    reset_at: float


class ResetCredit(TypedDict, total=False):
    reset_type: str
    status: str
    granted_at: str
    expires_at: str
    redeem_started_at: Optional[str]
    redeemed_at: Optional[str]


class ResetCreditsPayload(TypedDict, total=False):
    available_count: int
    total_earned_count: int
    credits: List[ResetCredit]


class AccountUsage(TypedDict, total=False):
    reset_ts: float
    used_percent: float
    primary_window: RateLimitWindow
    secondary_window: RateLimitWindow
    short_window: RateLimitWindow
    weekly_window: RateLimitWindow
    last_fetched: float
    archived: bool
    resets: Optional[ResetCreditsPayload]


class AuthTokens(TypedDict, total=False):
    id_token: Optional[str]
    access_token: Optional[str]
    refresh_token: Optional[str]
    account_id: Optional[str]


class AuthFileSnapshot(TypedDict, total=False):
    auth_mode: Optional[str]
    OPENAI_API_KEY: Optional[str]
    tokens: AuthTokens
    last_refresh: Optional[str]


UsageMap = Dict[str, AccountUsage]


class RateLimitPayload(TypedDict, total=False):
    allowed: bool
    limit_reached: bool
    primary_window: Optional[RateLimitWindow]
    secondary_window: Optional[RateLimitWindow]


class UsageResponse(TypedDict, total=False):
    user_id: str
    account_id: str
    email: str
    plan_type: str
    rate_limit: RateLimitPayload
    code_review_rate_limit: Optional[Any]
    additional_rate_limits: Optional[Any]
    credits: Optional[Dict[str, Any]]
    spend_control: Optional[Dict[str, Any]]
    rate_limit_reached_type: Optional[str]
    promo: Optional[Any]
    referral_beacon: Optional[Any]
