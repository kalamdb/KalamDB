"""Unit tests for the value codecs generated row classes call. No server needed."""

import datetime as dt
from dataclasses import dataclass
from enum import Enum
from typing import Any, Mapping, Optional

import pytest

from kalamdb.schema import (
    KTable,
    as_bool,
    as_datetime,
    as_enum,
    as_int,
    as_json,
    as_list,
    as_str,
    as_str_or_none,
    as_struct,
    bigint,
    encode_datetime,
    encode_enum,
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
