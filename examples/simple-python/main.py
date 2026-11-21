"""Minimal KalamDB Python example.

Tables are declared once in kalam/schema.sql. `kalam schema gen --languages
python` writes generated/kalam.py, which gives you a typed row class, typed
columns, typed write inputs, and a table declaration. Nothing here creates
tables: that is what schema.sql and migrations are for.

The `tags TEXT[]` column is declared but never written. The SQL API binds
scalar parameters only, so an array cannot be bound yet; see "Array columns"
in README.md.
"""

import asyncio
import os

from kalamdb import Auth, KalamClient

from generated.kalam import NoteState, Notes, notes

URL = os.environ.get("KALAMDB_URL", "http://localhost:2900")
USER = os.environ.get("KALAMDB_USER", "root")
PASSWORD = os.environ.get("KALAMDB_PASSWORD", "")


async def main() -> None:
    async with KalamClient(URL, Auth.basic(USER, PASSWORD)) as client:
        # INSERT: id and created_at are server-filled, so they are left out.
        # The mapping is a generated TypedDict, so a wrong field type is a type
        # error here rather than a server error at run time.
        sql, params = notes.insert_sql(
            {
                "author": "manik",
                "body": "schema.sql is the source of truth",
                "state": NoteState.DRAFT,
            }
        )
        await client.query(sql, params)

        # SELECT: the where fragment is parameterised, never interpolated.
        sql, params = notes.select_sql("author = $1", ["manik"])
        rows = await client.query_rows(sql, params)
        note: Notes = notes.decode(rows[0])

        # UPDATE by primary key. The enum is encoded on the way out, so the
        # member goes over the wire as its label.
        sql, params = notes.update_sql(note.id, {"state": NoteState.PUBLISHED})
        await client.query(sql, params)

        sql, params = notes.select_by_key_sql(note.id)
        for row in await client.query_rows(sql, params):
            note = notes.decode(row)
            # Typed attributes: created_at is a datetime, state is an enum.
            print(f"{note.created_at:%Y-%m-%d}  {note.state.value:9}  {note.body}")

        # DELETE by primary key.
        sql, params = notes.delete_sql(note.id)
        await client.query(sql, params)


if __name__ == "__main__":
    asyncio.run(main())
