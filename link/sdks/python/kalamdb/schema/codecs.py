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
    Sequence,
    Type,
    TypeVar,
    cast,
)

if TYPE_CHECKING:  # pragma: no cover - import only for annotations
    from .._native import FileRef

__all__ = [
    "UnsupportedParameterError",
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
    "encode_param",
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
    """BYTES columns arrive as an array of byte numbers.

    That is what the server emits for Binary and LargeBinary cells. Base64 text
    is accepted too, for values that reach Python through a base64 transport
    outside the SQL API. A 16-byte FixedSizeBinary is a UUID on the wire and
    comes back as a string, so it is read with ``as_str``, not here.
    """
    value = _required(value, "byte string")
    if isinstance(value, (bytes, bytearray)):
        return bytes(value)
    if isinstance(value, str):
        return base64.b64decode(value)
    if isinstance(value, Sequence):
        try:
            return bytes(value)
        except (TypeError, ValueError):
            raise ValueError(f"cannot read {value!r} as bytes") from None
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
_EPOCH_DAY = _dt.date(1970, 1, 1)
_MICROS_PER_DAY = 86_400_000_000


def _from_epoch_micros(value: float) -> _dt.datetime:
    """A numeric TIMESTAMP is always microseconds since the epoch.

    The server normalises every timestamp precision to microseconds before it
    serialises a cell (`scalar_value_to_json` in kalamdb-commons), so there is
    nothing to detect here. An earlier version guessed milliseconds for small
    numbers, which silently moved timestamps near the epoch: 1970-01-02 is
    86_400_000_000 microseconds, below any plausible "looks like micros"
    threshold, so it was multiplied by 1000 and read back as 1972.
    """
    # timedelta rather than fromtimestamp: microsecond counts are outside the
    # range fromtimestamp accepts, and dividing first would round.
    return _EPOCH + _dt.timedelta(microseconds=value)


def _from_epoch_days(days: int) -> _dt.date:
    """A DATE cell: whole days since 1970-01-01."""
    return _EPOCH_DAY + _dt.timedelta(days=days)


def _from_midnight_micros(micros: int) -> _dt.time:
    """A TIME cell: microseconds since midnight, wrapped into one day.

    Wrapping matches the TypeScript SDK, so a value that a function pushed past
    midnight reads back as a wall-clock time instead of raising.
    """
    wrapped = micros % _MICROS_PER_DAY
    return (_dt.datetime.min + _dt.timedelta(microseconds=wrapped)).time()


def _integer_text(value: str) -> Optional[int]:
    """The integer a wire string holds, or None when it is not one."""
    text = value.strip()
    digits = text[1:] if text[:1] in ("+", "-") else text
    return int(text) if digits.isdigit() else None


def as_datetime(value: Any) -> _dt.datetime:
    value = _required(value, "timestamp")
    if isinstance(value, _dt.datetime):
        return value
    if isinstance(value, str):
        micros = _integer_text(value)
        if micros is not None:
            return _from_epoch_micros(micros)
        return _dt.datetime.fromisoformat(_iso(value))
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return _from_epoch_micros(value)
    raise ValueError(f"cannot read {value!r} as a timestamp")


def as_date(value: Any) -> _dt.date:
    """DATE columns arrive as Arrow Date32: whole days since 1970-01-01.

    A number is never a timestamp here, which is why this does not share
    ``as_datetime``'s microsecond reading. TypeScript draws the same line
    between ``asDate`` and ``asDateOnly``.
    """
    value = _required(value, "date")
    if isinstance(value, _dt.datetime):
        return value.date()
    if isinstance(value, _dt.date):
        return value
    if isinstance(value, bool):
        raise ValueError(f"cannot read {value!r} as a date")
    if isinstance(value, int):
        return _from_epoch_days(value)
    if isinstance(value, float) and value.is_integer():
        return _from_epoch_days(int(value))
    if isinstance(value, str):
        days = _integer_text(value)
        if days is not None:
            return _from_epoch_days(days)
        return _dt.date.fromisoformat(value.strip())
    raise ValueError(f"cannot read {value!r} as a date")


def as_time(value: Any) -> _dt.time:
    """TIME columns arrive as microseconds since midnight.

    The server normalises every Time32/Time64 precision to microseconds, the
    same as it does for timestamps.
    """
    value = _required(value, "time")
    if isinstance(value, _dt.time):
        return value
    if isinstance(value, _dt.datetime):
        return value.time()
    if isinstance(value, bool):
        raise ValueError(f"cannot read {value!r} as a time")
    if isinstance(value, int):
        return _from_midnight_micros(value)
    if isinstance(value, float) and value.is_integer():
        return _from_midnight_micros(int(value))
    if isinstance(value, str):
        micros = _integer_text(value)
        if micros is not None:
            return _from_midnight_micros(micros)
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


# -- parameter binding ---------------------------------------------------------


class UnsupportedParameterError(ValueError):
    """A value the server cannot accept as a bound SQL parameter.

    Raised before the statement is sent, so the caller sees which column is at
    fault instead of a positional "Parameter $4 invalid" from the server.
    """


def encode_param(value: Any) -> Any:
    """Encode one value for a bound SQL parameter.

    Dispatches on the Python type rather than the column's SQL type: the same
    rules apply wherever a value is bound, and a column declared through the
    generic ``Column(name, sql_type)`` fallback carries no SQL type worth
    dispatching on.

    Anything that does not reduce to a JSON scalar is refused here rather than
    sent, because the server binds scalars only.
    """
    encoded = _encode_value(value)
    if isinstance(encoded, (bytes, bytearray)):
        raise UnsupportedParameterError(
            "the server does not accept binary parameters yet. Write the column "
            "with an inline literal in your own SQL, or store the bytes as a FILE."
        )
    if isinstance(encoded, Mapping) or isinstance(
        encoded, (list, tuple, set, frozenset)
    ):
        raise UnsupportedParameterError(
            "the server does not accept array or object parameters yet "
            '("Array parameters not yet supported"). Leave the column out of the '
            "write, or set it with an inline literal in your own SQL."
        )
    return encoded


def _encode_value(value: Any) -> Any:
    """Reduce one declared value towards a JSON scalar."""
    # A generated enum is a str subclass as well, so Enum is matched first.
    if isinstance(value, Enum):
        return _encode_value(value.value)
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, _dt.datetime):
        return value.isoformat()
    if isinstance(value, _dt.date):
        return value.isoformat()
    if isinstance(value, _dt.time):
        return value.isoformat()
    # A generated row class encodes through to_row; a FileRef through to_dict.
    # Both land on a Mapping, which encode_param then refuses -- the point is
    # to reach a shape the caller recognises in the error, not to send it.
    to_row = getattr(value, "to_row", None)
    if callable(to_row):
        return to_row()
    to_dict = getattr(value, "to_dict", None)
    if callable(to_dict):
        return to_dict()
    return value
