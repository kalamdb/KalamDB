"""Unit tests for the value codecs generated row classes call. No server needed."""

import datetime as dt
from dataclasses import dataclass
from enum import Enum
from typing import Any, Mapping, Optional

import pytest

from kalamdb.schema import (
    KTable,
    UnsupportedParameterError,
    as_bool,
    as_bytes,
    as_date,
    as_datetime,
    as_enum,
    as_int,
    as_json,
    as_list,
    as_str,
    as_str_or_none,
    as_struct,
    as_time,
    bigint,
    encode_datetime,
    encode_enum,
    encode_param,
    encode_row,
    text,
    timestamp,
)


class Status(str, Enum):
    ACTIVE = "active"
    BLOCKED = "blocked"


@dataclass(frozen=True)
class Address:
    city: Optional[str]
    country: str

    @classmethod
    def from_row(cls, row: Mapping[str, Any]) -> "Address":
        return cls(city=as_str_or_none(row.get("city")), country=as_str(row.get("country")))

    def to_row(self):
        return {"city": self.city, "country": self.country}


@dataclass(frozen=True)
class Note:
    """Shaped like a generated row class: decoding goes through the codecs."""

    id: int
    author: str
    body: str

    @classmethod
    def from_row(cls, row: Mapping[str, Any]) -> "Note":
        return cls(
            id=as_int(row.get("id")),
            author=as_str(row.get("author")),
            body=as_str(row.get("body")),
        )

    def to_row(self):
        return {"id": self.id, "author": self.author, "body": self.body}


class NoteColumns:
    id = bigint("id").primary_key().server_default()
    author = text("author").not_null()
    body = text("body").not_null()
    created_at = timestamp("created_at").not_null().server_default()


def notes_table():
    return KTable.shared("playground.notes", NoteColumns, row=Note)


# -- decoding through a row class ---------------------------------------------


def test_decode_uses_the_row_class():
    note = notes_table().decode({"id": "7", "author": "manik", "body": "hello"})
    assert note == Note(id=7, author="manik", body="hello")

# -- codecs -------------------------------------------------------------------


def test_scalar_codecs_accept_wire_shapes():
    assert as_int("7") == 7
    assert as_int(7.0) == 7
    assert as_str(7) == "7"
    assert as_bool("true") is True
    assert as_bool(0) is False
    assert as_str_or_none(None) is None


def test_scalar_codecs_reject_nulls_for_required_columns():
    with pytest.raises(ValueError, match="null"):
        as_int(None)


def test_datetime_codec_handles_the_zulu_suffix():
    parsed = as_datetime("2026-01-18T10:22:00Z")
    assert parsed.year == 2026 and parsed.tzinfo is not None
    assert encode_datetime(dt.datetime(2026, 1, 18, 10, 22)) == "2026-01-18T10:22:00"
    assert encode_datetime(None) is None


def test_datetime_codec_reads_numeric_epochs():
    """A numeric timestamp is microseconds since the epoch, always.

    The server normalises every timestamp precision to microseconds before it
    serialises a cell, so there is nothing to detect.
    """
    assert as_datetime(1_704_067_200_000_000) == dt.datetime(
        2024, 1, 1, tzinfo=dt.timezone.utc
    )
    # Numeric values also arrive as text, the way Int64 does.
    assert as_datetime("1704067200000000") == dt.datetime(
        2024, 1, 1, tzinfo=dt.timezone.utc
    )


def test_datetime_codec_keeps_timestamps_near_the_epoch():
    """A small microsecond count is not a millisecond count.

    Reading 1970-01-02 as milliseconds multiplied it by a thousand and moved it
    to 1972, which is how this was first noticed against a live server.
    """
    assert as_datetime(86_400_000_000) == dt.datetime(
        1970, 1, 2, tzinfo=dt.timezone.utc
    )
    assert as_datetime(0) == dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)
    assert as_datetime(-86_400_000_000) == dt.datetime(
        1969, 12, 31, tzinfo=dt.timezone.utc
    )


def test_date_codec_reads_day_offsets():
    """DATE arrives as Arrow Date32: whole days since 1970-01-01."""
    assert as_date(20_454) == dt.date(2026, 1, 1)
    assert as_date(0) == dt.date(1970, 1, 1)
    assert as_date(-1) == dt.date(1969, 12, 31)
    assert as_date("20454") == dt.date(2026, 1, 1)
    # ISO text still works, for values that reach Python another way.
    assert as_date("2026-01-01") == dt.date(2026, 1, 1)
    with pytest.raises(ValueError):
        as_date(True)


def test_time_codec_reads_microseconds_since_midnight():
    assert as_time(45_296_000_000) == dt.time(12, 34, 56)
    assert as_time(0) == dt.time(0, 0)
    assert as_time(45_296_500_000) == dt.time(12, 34, 56, 500_000)
    assert as_time("45296000000") == dt.time(12, 34, 56)
    assert as_time("12:34:56") == dt.time(12, 34, 56)


def test_bytes_codec_reads_the_byte_array_the_server_sends():
    assert as_bytes([1, 2, 255]) == b"\x01\x02\xff"
    assert as_bytes(b"ab") == b"ab"
    # Base64 text is still accepted for non-SQL transports.
    assert as_bytes("YWI=") == b"ab"
    with pytest.raises(ValueError):
        as_bytes([1, 2, 999])


def test_encode_param_reduces_declared_types_to_scalars():
    assert encode_param(Status.ACTIVE) == "active"
    assert encode_param(dt.datetime(2026, 1, 1)) == "2026-01-01T00:00:00"
    assert encode_param(dt.date(2026, 1, 1)) == "2026-01-01"
    assert encode_param(dt.time(12, 34)) == "12:34:00"
    assert encode_param(None) is None
    assert encode_param(7) == 7


def test_encode_param_refuses_what_the_server_cannot_bind():
    """Arrays, objects and binary are refused before the statement is sent."""
    with pytest.raises(UnsupportedParameterError, match="array or object"):
        encode_param(["a", "b"])
    with pytest.raises(UnsupportedParameterError, match="array or object"):
        encode_param({"city": "Jammu"})
    with pytest.raises(UnsupportedParameterError, match="binary"):
        encode_param(b"\x00")


def test_enum_codec_round_trips_and_reports_unknown_labels():
    assert as_enum("active", Status) is Status.ACTIVE
    assert encode_enum(Status.BLOCKED) == "blocked"
    with pytest.raises(ValueError, match="not a Status"):
        as_enum("retired", Status)


def test_struct_and_list_codecs():
    address = as_struct({"city": None, "country": "IN"}, Address)
    assert address == Address(city=None, country="IN")
    assert encode_row(address) == {"city": None, "country": "IN"}
    assert encode_row(None) is None
    assert as_list(["a", "b"], as_str) == ["a", "b"]
    assert as_list('["a"]', as_str) == ["a"]
    with pytest.raises(ValueError, match="array"):
        as_list({"a": 1}, as_str)


def test_json_codec_parses_text_and_passes_objects_through():
    assert as_json('{"a": 1}') == {"a": 1}
    assert as_json({"a": 1}) == {"a": 1}
    assert as_json(None) is None
