"""Helpers the generated routine clients call.

A generated ``KalamFunctions`` method runs the CALL or SELECT statement the
contract declares and reads one cell back. Pulling that read out of the
generated code keeps the error message in one place.
"""

from __future__ import annotations

from typing import Any, Mapping, Sequence

__all__ = ["RoutineResultError", "routine_result"]

#: Column alias the generated SELECT gives a function's return value.
RESULT_COLUMN = "result"


class RoutineResultError(ValueError):
    """A routine returned nothing where its contract declares a result."""


def routine_result(routine: str, rows: Sequence[Mapping[str, Any]]) -> Any:
    """The single result cell of a routine call.

    ``routine`` is the qualified name, so a server that returns an empty result
    set says which call it was rather than failing on an index later.
    """
    if not rows:
        raise RoutineResultError(f"{routine} returned no rows, but it declares a result")
    return rows[0].get(RESULT_COLUMN)
