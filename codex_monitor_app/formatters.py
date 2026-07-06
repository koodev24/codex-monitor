from datetime import datetime, timezone
from typing import List, Optional

from .models import ResetCredit, ResetCreditsPayload


def format_reset_display(reset_ts: Optional[float], now_ts: float) -> str:
    if not reset_ts:
        return "-"

    reset_time_str = datetime.fromtimestamp(reset_ts).strftime("%Y-%m-%d %H:%M")

    if now_ts >= reset_ts:
        countdown_text = "0m"
    else:
        diff = int(reset_ts - now_ts)
        days = diff // 86400
        hours = (diff % 86400) // 3600
        minutes = (diff % 3600) // 60

        time_parts: List[str] = []
        if days > 0:
            time_parts.append(f"{days}d")
        if hours > 0:
            time_parts.append(f"{hours}h")
        time_parts.append(f"{minutes}m")
        countdown_text = " ".join(time_parts)

    return f"{reset_time_str} ({countdown_text})"


def format_quota_left(used_percent: float) -> str:
    return f"{100 - used_percent}%"


def _parse_iso_timestamp(value: Optional[str]) -> Optional[float]:
    if not isinstance(value, str) or not value:
        return None

    normalized = value
    if normalized.endswith("Z"):
        normalized = normalized[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.timestamp()


def _countdown_text(seconds_remaining: int) -> str:
    if seconds_remaining <= 0:
        return "expired"

    days = seconds_remaining // 86400
    hours = (seconds_remaining % 86400) // 3600
    minutes = (seconds_remaining % 3600) // 60

    time_parts: List[str] = []
    if days > 0:
        time_parts.append(f"{days}d")
    if hours > 0:
        time_parts.append(f"{hours}h")
    time_parts.append(f"{minutes}m")
    return " ".join(time_parts)


def format_reset_granted_at(granted_at: Optional[str]) -> str:
    granted_ts = _parse_iso_timestamp(granted_at)
    if granted_ts is None:
        return "-"
    return datetime.fromtimestamp(granted_ts).strftime("%Y-%m-%d %H:%M")


def format_reset_credit_expires(expires_at: Optional[str], now_ts: float) -> str:
    expires_ts = _parse_iso_timestamp(expires_at)
    if expires_ts is None:
        return "-"

    expires_time_str = datetime.fromtimestamp(expires_ts).strftime("%Y-%m-%d %H:%M")
    countdown = _countdown_text(int(expires_ts - now_ts))
    return f"{expires_time_str} ({countdown})"


def format_reset_time_remaining(expires_at: Optional[str], now_ts: float) -> str:
    expires_ts = _parse_iso_timestamp(expires_at)
    if expires_ts is None:
        return "-"

    return _countdown_text(int(expires_ts - now_ts))


def soonest_expiring_credit(
    payload: Optional[ResetCreditsPayload],
) -> Optional[ResetCredit]:
    if not payload:
        return None

    credits = payload.get("credits") if isinstance(payload, dict) else None
    if not isinstance(credits, list):
        return None

    available = [c for c in credits if isinstance(c, dict) and c.get("status") == "available"]
    if not available:
        return None

    parsed = [
        (credit, _parse_iso_timestamp(credit.get("expires_at")))
        for credit in available
    ]
    parsed = [(credit, ts) for credit, ts in parsed if ts is not None]
    if not parsed:
        return available[0]

    parsed.sort(key=lambda item: item[1])
    return parsed[0][0]
