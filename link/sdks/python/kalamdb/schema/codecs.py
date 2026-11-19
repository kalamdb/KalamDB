"""Value decoders and encoders used by generated row classes.

Generated ``from_row`` calls the ``as_*`` helpers and generated ``to_row``
calls the ``encode_*`` helpers, so wire values are converted in one place
rather than at every call site.
"""

from __future__ import annotations

import base64
import datetime as _dt
import json as _json
from enum import Enum
from typing import (
    TYPE_CHECKING,
    Any,
    Callable,
    Dict,
    List,
    Mapping,
    Optional,
    Type,
    TypeVar,
    cast,
)

if TYPE_CHECKING:  # pragma: no cover - import only for annotations
    from .._native import FileRef

__all__ = [
    "as_bool",
    "as_bool_or_none",
    "as_bytes",
    "as_bytes_or_none",
    "as_date",
    "as_date_or_none",
    "as_datetime",
    "as_datetime_or_none",
    "as_enum",
    "as_enum_or_none",
    "as_file_ref",
    "as_file_ref_or_none",
    "as_float",
    "as_float_or_none",
    "as_float_list",
    "as_int",
    "as_int_or_none",
    "as_json",
    "as_list",
    "as_list_or_none",
    "as_str",
    "as_str_or_none",
    "as_struct",
    "as_struct_or_none",
    "as_time",
    "as_time_or_none",
    "encode_date",
    "encode_datetime",
    "encode_enum",
    "encode_file",
    "encode_list",
    "encode_row",
    "encode_time",
]

T = TypeVar("T")
E = TypeVar("E", bound=Enum)


def _required(value: Any, kind: str) -> Any:
    if value is None:
        raise ValueError(f"expected a {kind} but the column was null")
    return value


# -- scalars ------------------------------------------------------------------


def as_bool(value: Any) -> bool:
    value = _required(value, "boolean")
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return bool(value)
    if isinstance(value, str):
        lowered = value.strip().lower()
        if lowered in ("true", "t", "1"):
            return True
        if lowered in ("false", "f", "0"):
            return False
    raise ValueError(f"cannot read {value!r} as a boolean")


def as_int(value: Any) -> int:
    value = _required(value, "integer")
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, str):
        return int(value.strip())
    raise ValueError(f"cannot read {value!r} as an integer")


def as_float(value: Any) -> float:
    value = _required(value, "float")
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    if isinstance(value, str):
        return float(value.strip())
    raise ValueError(f"cannot read {value!r} as a float")


def as_str(value: Any) -> str:
    value = _required(value, "string")
    if isinstance(value, str):
        return value
    return str(value)


def as_bytes(value: Any) -> bytes:
    value = _required(value, "byte string")
    if isinstance(value, (bytes, bytearray)):
        return bytes(value)
    if isinstance(value, str):
        return base64.b64decode(value)
    raise ValueError(f"cannot read {value!r} as bytes")


def as_json(value: Any) -> Any:
    """JSON columns arrive already decoded, or as text that still needs parsing."""
    if isinstance(value, str):
        return _json.loads(value)
    return value


# -- temporal -----------------------------------------------------------------


def _iso(value: str) -> str:
    # datetime.fromisoformat only accepts a 'Z' suffix from 3.11 onward.
    text = value.strip()
    if text.endswith(("Z", "z")):
        return f"{text[:-1]}+00:00"
    return text


_EPOCH = _dt.datetime(1970, 1, 1, tzinfo=_dt.timezone.utc)

# The server sends TIMESTAMP values as microseconds since the epoch (16 digits
# for current dates); plain milliseconds (13 digits) are accepted for
# compatibility. Same threshold as the Dart and TypeScript SDKs: year 2001 is
# ~1e12 in milliseconds and ~1e15 in microseconds, so nothing between 1970 and
# 2286 is ambiguous. A numeric value is never seconds.
_MILLIS_MAX = 9_999_999_999_999


def _from_epoch(value: float) -> _dt.datetime:
    micros = value if abs(value) > _MILLIS_MAX else value * 1000
    # timedelta rather than fromtimestamp: microsecond counts are outside the
    # range fromtimestamp accepts, and dividing first would round.
    return _EPOCH + _dt.timedelta(microseconds=micros)


def as_datetime(value: Any) -> _dt.datetime:
    value = _required(value, "timestamp")
    if isinstance(value, _dt.datetime):
        return value
    if isinstance(value, str):
        return _dt.datetime.fromisoformat(_iso(value))
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return _from_epoch(value)
    raise ValueError(f"cannot read {value!r} as a timestamp")


def as_date(value: Any) -> _dt.date:
    value = _required(value, "date")
    if isinstance(value, _dt.datetime):
        return value.date()
    if isinstance(value, _dt.date):
        return value
    if isinstance(value, str):
        return _dt.date.fromisoformat(value.strip())
    raise ValueError(f"cannot read {value!r} as a date")


def as_time(value: Any) -> _dt.time:
    value = _required(value, "time")
    if isinstance(value, _dt.time):
        return value
    if isinstance(value, _dt.datetime):
        return value.time()
    if isinstance(value, str):
        return _dt.time.fromisoformat(_iso(value))
    raise ValueError(f"cannot read {value!r} as a time")


# -- composites ---------------------------------------------------------------


def as_enum(value: Any, cls: Type[E]) -> E:
    value = _required(value, cls.__name__)
    try:
        return cls(value)
    except ValueError:
        labels = ", ".join(repr(member.value) for member in cls)
        raise ValueError(
            f"{value!r} is not a {cls.__name__}; expected one of {labels}"
        ) from None


def as_struct(value: Any, cls: Type[T]) -> T:
    value = _required(value, cls.__name__)
    if isinstance(value, cls):
        return value
    if isinstance(value, str):
        value = _json.loads(value)
    if not isinstance(value, Mapping):
        raise ValueError(f"cannot read {value!r} as a {cls.__name__}")
    # Row classes are generated with from_row; the cast keeps that contract
    # visible to type checkers without importing the generated module.
    return cast(T, cls.from_row(value))  # type: ignore[attr-defined]


def as_list(value: Any, item: Callable[[Any], T]) -> List[T]:
    value = _required(value, "array")
    if isinstance(value, str):
        value = _json.loads(value)
    if isinstance(value, (str, bytes, Mapping)) or not hasattr(value, "__iter__"):
        raise ValueError(f"cannot read {value!r} as an array")
    return [item(element) for element in value]


def as_float_list(value: Any) -> List[float]:
    """EMBEDDING columns arrive as a plain array of numbers."""
    return as_list(value, as_float)


def as_file_ref(value: Any) -> "FileRef":
    value = _required(value, "file reference")
    from .._native import FileRef  # imported lazily: the SDK works without a file column

    if isinstance(value, FileRef):
        return value
    parsed = FileRef.from_value(value)
    if parsed is None:
        raise ValueError(f"cannot read {value!r} as a file reference")
    return parsed


# -- nullable wrappers --------------------------------------------------------


def as_bool_or_none(value: Any) -> Optional[bool]:
    return None if value is None else as_bool(value)


def as_int_or_none(value: Any) -> Optional[int]:
    return None if value is None else as_int(value)


def as_float_or_none(value: Any) -> Optional[float]:
    return None if value is None else as_float(value)


def as_str_or_none(value: Any) -> Optional[str]:
    return None if value is None else as_str(value)


def as_bytes_or_none(value: Any) -> Optional[bytes]:
    return None if value is None else as_bytes(value)


def as_datetime_or_none(value: Any) -> Optional[_dt.datetime]:
    return None if value is None else as_datetime(value)


def as_date_or_none(value: Any) -> Optional[_dt.date]:
    return None if value is None else as_date(value)


def as_time_or_none(value: Any) -> Optional[_dt.time]:
    return None if value is None else as_time(value)


def as_enum_or_none(value: Any, cls: Type[E]) -> Optional[E]:
    return None if value is None else as_enum(value, cls)


def as_struct_or_none(value: Any, cls: Type[T]) -> Optional[T]:
    return None if value is None else as_struct(value, cls)


def as_list_or_none(value: Any, item: Callable[[Any], T]) -> Optional[List[T]]:
    return None if value is None else as_list(value, item)


def as_file_ref_or_none(value: Any) -> Optional["FileRef"]:
    return None if value is None else as_file_ref(value)


# -- encoders -----------------------------------------------------------------


def encode_datetime(value: Optional[_dt.datetime]) -> Optional[str]:
    return None if value is None else value.isoformat()


def encode_date(value: Optional[_dt.date]) -> Optional[str]:
    return None if value is None else value.isoformat()


def encode_time(value: Optional[_dt.time]) -> Optional[str]:
    return None if value is None else value.isoformat()


def encode_enum(value: Any) -> Any:
    return None if value is None else value.value


def encode_row(value: Any) -> Optional[Dict[str, Any]]:
    """Encode a nested row class; ``None`` passes straight through."""
    return None if value is None else value.to_row()


def encode_list(value: Any, item: Callable[[Any], Any]) -> Optional[List[Any]]:
    return None if value is None else [item(element) for element in value]


def encode_file(value: Any) -> Any:
    if value is None:
        return None
    to_dict = getattr(value, "to_dict", None)
    return to_dict() if callable(to_dict) else value
