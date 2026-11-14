"""Run declared tables against a connected client.

:meth:`kalamdb.schema.Table.bind` pairs a table definition with a
:class:`~kalamdb.KalamClient` and hands back a :class:`BoundTable`, so the DDL,
INSERTs and SELECTs the definition already knows how to render can be executed
without restating the table name, its columns, or its primary key::

    from kalamdb import Auth, KalamClient
    from kalamdb.schema import KTable, bigint, text, timestamp

    notes = KTable.shared(
        "playground.notes",
        id=bigint("id").primary_key().server_default("SNOWFLAKE_ID()"),
        body=text("body").not_null(),
        created_at=timestamp("created_at").not_null().server_default("NOW()"),
    ).compression("zstd")

    async with KalamClient(url, Auth.basic("admin", password)) as client:
        db = notes.bind(client)
        await db.create()
        await db.insert({"body": "hello"})
        rows = await db.select(order_by="created_at DESC", limit=10)
"""

from __future__ import annotations

from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence

from .schema import Table

__all__ = ["BoundTable"]


class BoundTable:
    """A table definition paired with the client that runs its statements.

    Every method renders SQL from the definition and awaits the client, so the
    declared column types, server defaults and primary key stay the one source
    of truth. Rows coming back are coerced to the declared Python types; pass
    ``parse=False`` to read them exactly as the server sent them.
    """

    def __init__(self, table: Table, client: Any) -> None:
        self.table = table
        self.client = client

    @property
    def qualified_name(self) -> str:
        return self.table.qualified_name

    async def create(self, if_not_exists: bool = True) -> Dict[str, Any]:
        """CREATE TABLE with the declared columns and table options."""
        return await self.client.query(self.table.create_table_sql(if_not_exists))

    async def drop(self, if_exists: bool = True) -> Dict[str, Any]:
        return await self.client.query(self.table.drop_table_sql(if_exists))

    async def insert(self, data: Mapping[str, Any]) -> Dict[str, Any]:
        """Insert one row, skipping server-filled columns.

        A FILE column handed a ``(filename, data[, mime])`` tuple or a
        ``pathlib.Path`` is uploaded with the statement; a FILE column handed a
        string is treated as a reference to a file that already exists.
        """
        sql, params, files = self.table.insert_with_files_sql(data)
        if files:
            return await self.client.query_with_files(sql, files, params)
        return await self.client.query(sql, params)

    async def insert_many(self, rows: Iterable[Mapping[str, Any]]) -> Dict[str, Any]:
        """Insert several rows in one statement. Every row must fill the same
        columns; rows carrying uploads go through :meth:`insert` instead."""
        sql, params = self.table.insert_many_sql(rows)
        return await self.client.query(sql, params)

    async def select(
        self,
        where: str = "",
        params: Optional[Sequence[Any]] = None,
        *,
        columns: Optional[Sequence[str]] = None,
        order_by: str = "",
        limit: Optional[int] = None,
        offset: Optional[int] = None,
        parse: bool = True,
    ) -> List[Dict[str, Any]]:
        """Rows matching ``where``, whose $1, $2, ... are bound from ``params``."""
        sql = self.table.select_sql(
            where, columns=columns, order_by=order_by, limit=limit, offset=offset
        )
        rows = await self.client.query_rows(sql, list(params) if params else None)
        return self.table.parse_rows(rows) if parse else list(rows)

    async def get(self, pk_value: Any, *, parse: bool = True) -> Optional[Dict[str, Any]]:
        """The row carrying this primary key, or None when there is no such row."""
        pk = self.table.primary_key_column
        if pk is None:
            raise ValueError(
                f"{self.qualified_name} declares no primary_key() column, so get() "
                "has nothing to match on"
            )
        rows = await self.select(f"{pk.name} = $1", [pk_value], limit=1, parse=parse)
        return rows[0] if rows else None

    async def delete(self, pk_value: Any) -> Dict[str, Any]:
        """Delete the row carrying this primary key."""
        sql, params = self.table.delete_by_pk_sql(pk_value)
        return await self.client.query(sql, params)

    async def count(
        self, where: str = "", params: Optional[Sequence[Any]] = None
    ) -> int:
        rows = await self.client.query_rows(
            self.table.count_sql(where), list(params) if params else None
        )
        if not rows:
            return 0
        return int(next(iter(rows[0].values())))

    async def live(self, where: str = "", **options: Any) -> Any:
        """Materialised live rows for this table's SELECT.

        Options pass through to ``KalamClient.live`` (``batch_size``,
        ``last_rows``, ``from_``, ``limit``, ``key_columns``, ...). The declared
        primary key is used as the key column unless ``key_columns`` says
        otherwise, so updates replace rows instead of stacking up.

        Live subscriptions take no query parameters, so ``where`` must carry its
        own values rather than $1, $2, ... placeholders.
        """
        pk = self.table.primary_key_column
        if pk is not None and "key_columns" not in options:
            options["key_columns"] = [pk.name]
        return await self.client.live(self.table.select_sql(where), **options)

    async def live_events(self, where: str = "", **options: Any) -> Any:
        """Raw change events for this table's SELECT, straight from
        ``KalamClient.live_events``. As with :meth:`live`, ``where`` carries its
        own values — a live subscription takes no query parameters."""
        return await self.client.live_events(self.table.select_sql(where), **options)

    def __repr__(self) -> str:
        return f"BoundTable({self.qualified_name!r}, kind={self.table.kind.value})"
